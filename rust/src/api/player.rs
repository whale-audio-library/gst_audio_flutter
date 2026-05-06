use std::{
    collections::VecDeque,
    env,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender, TryRecvError},
        Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use gio::prelude::*;
use glib::{Value, ValueArray};
use gst::prelude::*;
use gstreamer as gst;
use rand::Rng;

const SPECTRUM_BANDS: u32 = 48;
const SPECTRUM_THRESHOLD_DB: i32 = -60;
const VISUALIZATION_NOISE_GATE_DB: f64 = -55.0;
const VISUALIZATION_INTERVAL_MS: u64 = 50;
const VISUALIZATION_WAVEFORM_POINTS: usize = 96;
const VISUALIZATION_SMOOTHING_ATTACK: f64 = 0.55;
const VISUALIZATION_SMOOTHING_RELEASE: f64 = 0.25;
const BEAT_HISTORY_FRAMES: usize = 24;
const BEAT_MIN_ENERGY: f64 = 0.12;
const BEAT_STRENGTH_THRESHOLD: f64 = 0.35;
const BEAT_COOLDOWN_FRAMES: u32 = 4;

#[derive(Clone, Debug)]
pub struct Track {
    pub uri: String,
    pub title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatMode {
    None,
    One,
    All,
}

#[derive(Clone, Debug)]
pub struct PlaybackState {
    pub playlist: Vec<Track>,
    pub current_index: i32,
    pub current_uri: String,
    pub current_title: String,
    pub is_playing: bool,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub buffering_percent: i32,
    pub is_buffering: bool,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub shuffle: bool,
    pub repeat_mode: RepeatMode,
    pub output_device_id: String,
    pub output_device_name: String,
    pub last_error: String,
}

#[derive(Clone, Debug)]
pub struct AudioOutputDevice {
    pub id: String,
    pub name: String,
    pub device_class: String,
    pub is_current: bool,
}

#[derive(Clone, Debug)]
pub struct VisualizationFrame {
    pub timestamp_ms: i64,
    pub pcm: Vec<f64>,
    pub magnitude: Vec<f64>,
    pub rms: Vec<f64>,
    pub peak: Vec<f64>,
    pub decay: Vec<f64>,
    pub rms_normalized: f64,
    pub peak_normalized: f64,
    pub beat: bool,
    pub beat_strength: f64,
    pub waveform: Vec<f64>,
    pub normalized: Vec<f64>,
    pub is_active: bool,
}

impl VisualizationFrame {
    fn silent() -> Self {
        Self {
            timestamp_ms: 0,
            pcm: Vec::new(),
            magnitude: Vec::new(),
            rms: Vec::new(),
            peak: Vec::new(),
            decay: Vec::new(),
            rms_normalized: 0.0,
            peak_normalized: 0.0,
            beat: false,
            beat_strength: 0.0,
            waveform: Vec::new(),
            normalized: Vec::new(),
            is_active: false,
        }
    }

    fn from_spectrum_structure(structure: &gst::StructureRef) -> Option<Self> {
        if !structure.has_name("spectrum") {
            return None;
        }

        let magnitude = numeric_values(structure, "magnitude")?;
        let timestamp_ms = structure
            .get::<u64>("running-time")
            .or_else(|_| structure.get::<u64>("stream-time"))
            .or_else(|_| structure.get::<u64>("timestamp"))
            .map(|value| (value / 1_000_000) as i64)
            .unwrap_or(0);
        let normalized = magnitude
            .iter()
            .map(|value| normalize_db(*value))
            .collect::<Vec<_>>();
        let is_active = normalized.iter().any(|value| *value > 0.01)
            || magnitude
                .iter()
                .any(|value| *value > VISUALIZATION_NOISE_GATE_DB);

        Some(Self {
            timestamp_ms,
            pcm: Vec::new(),
            magnitude,
            rms: Vec::new(),
            peak: Vec::new(),
            decay: Vec::new(),
            rms_normalized: 0.0,
            peak_normalized: 0.0,
            beat: false,
            beat_strength: 0.0,
            waveform: Vec::new(),
            normalized,
            is_active,
        })
    }
}

#[derive(Clone, Debug)]
struct LevelFrame {
    timestamp_ms: i64,
    rms: Vec<f64>,
    peak: Vec<f64>,
    decay: Vec<f64>,
    rms_normalized: f64,
    peak_normalized: f64,
}

impl LevelFrame {
    fn from_structure(structure: &gst::StructureRef) -> Option<Self> {
        if !structure.has_name("level") {
            return None;
        }

        let rms = numeric_values(structure, "rms")?;
        let peak = numeric_values(structure, "peak").unwrap_or_default();
        let decay = numeric_values(structure, "decay").unwrap_or_default();
        let timestamp_ms = structure
            .get::<u64>("running-time")
            .or_else(|_| structure.get::<u64>("stream-time"))
            .or_else(|_| structure.get::<u64>("timestamp"))
            .map(|value| (value / 1_000_000) as i64)
            .unwrap_or(0);
        let rms_normalized = average_normalized_db(&rms);
        let peak_normalized = peak
            .iter()
            .map(|value| normalize_db(*value))
            .fold(0.0, f64::max);

        Some(Self {
            timestamp_ms,
            rms,
            peak,
            decay,
            rms_normalized,
            peak_normalized,
        })
    }
}

#[derive(Debug)]
enum Command {
    SetPlaylist {
        tracks: Vec<Track>,
        start_index: i32,
    },
    Play,
    PlayIndex(i32),
    Pause,
    TogglePlayPause,
    Stop,
    Previous,
    Next,
    Seek(i64),
    SetVolume(f64),
    SetMuted(bool),
    FadeTo {
        target: f64,
        duration_ms: i64,
    },
    SetSpeed(f64),
    SetShuffle(bool),
    SetRepeatMode(RepeatMode),
    SetOutputDevice(String),
    RefreshState(Sender<PlaybackState>),
    RefreshVisualization(Sender<VisualizationFrame>),
    ListOutputDevices(Sender<Vec<AudioOutputDevice>>),
    Shutdown(Sender<()>),
}

struct PlayerRuntime {
    tx: Sender<Command>,
    handle: JoinHandle<()>,
}

impl PlayerRuntime {
    fn start() -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("gst-audio-player".to_string())
            .spawn(move || {
                if let Err(err) = run_player_thread(rx, ready_tx) {
                    eprintln!("GStreamer player thread exited: {err}");
                }
            })
            .map_err(|err| format!("failed to spawn GStreamer player thread: {err}"))?;

        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self { tx, handle }),
            Ok(Err(err)) => {
                let _ = handle.join();
                Err(format!("failed to start GStreamer player: {err}"))
            }
            Err(err) => {
                drop(tx);
                drop(handle);
                Err(format!("timed out starting GStreamer player: {err}"))
            }
        }
    }

    fn join(self) -> Result<(), String> {
        self.handle
            .join()
            .map_err(|_| "player thread panicked".to_string())
    }
}

static RUNTIME: OnceLock<Mutex<Option<PlayerRuntime>>> = OnceLock::new();

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
    if let Err(err) = ensure_runtime_started() {
        eprintln!("Failed to initialize GStreamer player runtime: {err}");
    }
}

pub fn shutdown_player() -> Result<(), String> {
    let mut runtime = runtime_slot()
        .lock()
        .map_err(|_| "player runtime lock is poisoned".to_string())?;
    let Some(runtime) = runtime.take() else {
        return Ok(());
    };

    let (done_tx, done_rx) = mpsc::channel();
    if runtime.tx.send(Command::Shutdown(done_tx)).is_ok() {
        let _ = done_rx.recv_timeout(Duration::from_secs(2));
    }

    runtime
        .join()
        .map_err(|err| format!("{err} during shutdown"))
}

pub fn set_playlist(inputs: Vec<String>, start_index: i32) -> Result<PlaybackState, String> {
    let tracks = inputs
        .into_iter()
        .filter(|item| !item.trim().is_empty())
        .map(|item| Track {
            title: title_from_input(&item),
            uri: input_to_uri(&item),
        })
        .collect::<Vec<_>>();

    send(Command::SetPlaylist {
        tracks,
        start_index,
    })?;
    get_state()
}

pub fn play() -> Result<PlaybackState, String> {
    send(Command::Play)?;
    get_state()
}

pub fn play_index(index: i32) -> Result<PlaybackState, String> {
    send(Command::PlayIndex(index))?;
    get_state()
}

pub fn pause() -> Result<PlaybackState, String> {
    send(Command::Pause)?;
    get_state()
}

pub fn toggle_play_pause() -> Result<PlaybackState, String> {
    send(Command::TogglePlayPause)?;
    get_state()
}

pub fn stop() -> Result<PlaybackState, String> {
    send(Command::Stop)?;
    get_state()
}

pub fn previous() -> Result<PlaybackState, String> {
    send(Command::Previous)?;
    get_state()
}

pub fn next() -> Result<PlaybackState, String> {
    send(Command::Next)?;
    get_state()
}

pub fn seek_ms(position_ms: i64) -> Result<PlaybackState, String> {
    send(Command::Seek(position_ms))?;
    get_state()
}

pub fn set_volume(volume: f64) -> Result<PlaybackState, String> {
    send(Command::SetVolume(volume))?;
    get_state()
}

pub fn set_muted(muted: bool) -> Result<PlaybackState, String> {
    send(Command::SetMuted(muted))?;
    get_state()
}

pub fn fade_in(duration_ms: i64) -> Result<PlaybackState, String> {
    send(Command::FadeTo {
        target: 1.0,
        duration_ms,
    })?;
    get_state()
}

pub fn fade_out(duration_ms: i64) -> Result<PlaybackState, String> {
    send(Command::FadeTo {
        target: 0.0,
        duration_ms,
    })?;
    get_state()
}

pub fn set_speed(speed: f64) -> Result<PlaybackState, String> {
    send(Command::SetSpeed(speed))?;
    get_state()
}

pub fn set_shuffle(enabled: bool) -> Result<PlaybackState, String> {
    send(Command::SetShuffle(enabled))?;
    get_state()
}

pub fn set_repeat_mode(mode: RepeatMode) -> Result<PlaybackState, String> {
    send(Command::SetRepeatMode(mode))?;
    get_state()
}

pub fn set_output_device(device_id: String) -> Result<PlaybackState, String> {
    send(Command::SetOutputDevice(device_id))?;
    get_state()
}

pub fn list_output_devices() -> Result<Vec<AudioOutputDevice>, String> {
    let (reply_tx, reply_rx) = mpsc::channel();
    send(Command::ListOutputDevices(reply_tx))?;
    recv_reply(reply_rx, "output devices")
}

pub fn get_state() -> Result<PlaybackState, String> {
    let (reply_tx, reply_rx) = mpsc::channel();
    send(Command::RefreshState(reply_tx))?;
    recv_reply(reply_rx, "playback state")
}

pub fn get_visualization_frame() -> Result<VisualizationFrame, String> {
    let (reply_tx, reply_rx) = mpsc::channel();
    send(Command::RefreshVisualization(reply_tx))?;
    recv_reply(reply_rx, "visualization frame")
}

fn runtime_slot() -> &'static Mutex<Option<PlayerRuntime>> {
    RUNTIME.get_or_init(|| Mutex::new(None))
}

fn ensure_runtime_started() -> Result<(), String> {
    let mut runtime = runtime_slot()
        .lock()
        .map_err(|_| "player runtime lock is poisoned".to_string())?;
    ensure_runtime(&mut runtime)
}

fn ensure_runtime(runtime: &mut Option<PlayerRuntime>) -> Result<(), String> {
    if runtime
        .as_ref()
        .map(|runtime| runtime.handle.is_finished())
        .unwrap_or(false)
    {
        let finished = runtime.take().expect("runtime existed");
        if let Err(err) = finished.join() {
            eprintln!("Discarding stopped GStreamer player runtime: {err}");
        }
    }

    if runtime.is_none() {
        *runtime = Some(PlayerRuntime::start()?);
    }
    Ok(())
}

fn send(command: Command) -> Result<(), String> {
    let mut command = Some(command);

    for _ in 0..2 {
        let mut runtime = runtime_slot()
            .lock()
            .map_err(|_| "player runtime lock is poisoned".to_string())?;
        ensure_runtime(&mut runtime)?;
        let Some(active_runtime) = runtime.as_ref() else {
            return Err("player runtime failed to start".to_string());
        };

        match active_runtime
            .tx
            .send(command.take().expect("command is pending"))
        {
            Ok(()) => return Ok(()),
            Err(err) => {
                command = Some(err.0);
                if let Some(stopped_runtime) = runtime.take() {
                    if let Err(err) = stopped_runtime.join() {
                        eprintln!("Discarding disconnected GStreamer player runtime: {err}");
                    }
                }
            }
        }
    }

    Err("failed to send player command: player thread stopped".to_string())
}

fn recv_reply<T>(reply_rx: mpsc::Receiver<T>, label: &str) -> Result<T, String> {
    match reply_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(value) => Ok(value),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!("timed out waiting for {label}")),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(format!("player thread stopped before returning {label}"))
        }
    }
}

fn run_player_thread(
    rx: mpsc::Receiver<Command>,
    ready_tx: Sender<Result<(), String>>,
) -> Result<(), String> {
    configure_platform_gstreamer_runtime();
    if let Err(err) = gst::init() {
        let err = err.to_string();
        let _ = ready_tx.send(Err(err.clone()));
        return Err(err);
    }
    register_static_plugins();

    let mut player = match GStreamerPlayer::new() {
        Ok(player) => player,
        Err(err) => {
            let _ = ready_tx.send(Err(err.clone()));
            return Err(err);
        }
    };
    let _ = ready_tx.send(Ok(()));
    loop {
        loop {
            match rx.try_recv() {
                Ok(command) => {
                    if player.handle_command(command) {
                        return Ok(());
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    player.stop_pipeline();
                    return Ok(());
                }
            }
        }

        player.poll_bus();
        player.tick_fade();
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(target_os = "windows")]
fn configure_platform_gstreamer_runtime() {
    let Some(app_dir) = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
    else {
        return;
    };

    prepend_env_path("PATH", &app_dir);

    let plugin_dir = app_dir.join("lib").join("gstreamer-1.0");
    if plugin_dir.is_dir() {
        prepend_env_path("GST_PLUGIN_PATH", &plugin_dir);
        prepend_env_path("GST_PLUGIN_PATH_1_0", &plugin_dir);
    }

    let gio_module_dir = app_dir.join("lib").join("gio").join("modules");
    if gio_module_dir.is_dir() {
        prepend_env_path("GIO_EXTRA_MODULES", &gio_module_dir);
    }

    let scanner = app_dir
        .join("libexec")
        .join("gstreamer-1.0")
        .join("gst-plugin-scanner.exe");
    if scanner.is_file() && env::var_os("GST_PLUGIN_SCANNER").is_none() {
        env::set_var("GST_PLUGIN_SCANNER", scanner);
    }
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn configure_platform_gstreamer_runtime() {
    let tmp_dir = env::temp_dir();
    set_env_path_if_missing("TMP", &tmp_dir);
    set_env_path_if_missing("TEMP", &tmp_dir);
    set_env_path_if_missing("TMPDIR", &tmp_dir);
    set_env_path_if_missing("XDG_RUNTIME_DIR", &tmp_dir);
    set_env_path_if_missing("XDG_CACHE_HOME", &tmp_dir);
    set_env_path_if_missing("HOME", &tmp_dir);
}

#[cfg(not(any(target_os = "ios", target_os = "android", target_os = "windows")))]
fn configure_platform_gstreamer_runtime() {}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn set_env_path_if_missing(key: &str, value: &Path) {
    if env::var_os(key).is_none() {
        env::set_var(key, value);
    }
}

#[cfg(target_os = "windows")]
fn prepend_env_path(key: &str, value: &Path) {
    let mut paths = vec![value.to_path_buf()];
    if let Some(existing) = env::var_os(key) {
        for path in env::split_paths(&existing) {
            if path != value {
                paths.push(path);
            }
        }
    }

    if let Ok(joined) = env::join_paths(paths) {
        env::set_var(key, joined);
    }
}

#[cfg(target_os = "android")]
fn register_static_plugins() {
    unsafe {
        gst_audio_android_register_static_plugins();
    }
}

#[cfg(target_os = "ios")]
fn register_static_plugins() {
    unsafe {
        let plugins: &[(&str, unsafe extern "C" fn() -> glib::ffi::gboolean)] = &[
            ("coreelements", gst_plugin_coreelements_register),
            ("playback", gst_plugin_playback_register),
            ("typefindfunctions", gst_plugin_typefindfunctions_register),
            ("audioconvert", gst_plugin_audioconvert_register),
            ("audioparsers", gst_plugin_audioparsers_register),
            ("audioresample", gst_plugin_audioresample_register),
            ("volume", gst_plugin_volume_register),
            ("level", gst_plugin_level_register),
            ("spectrum", gst_plugin_spectrum_register),
            ("app", gst_plugin_app_register),
            ("autodetect", gst_plugin_autodetect_register),
            ("osxaudio", gst_plugin_osxaudio_register),
            ("gio", gst_plugin_gio_register),
            ("wavparse", gst_plugin_wavparse_register),
            ("id3demux", gst_plugin_id3demux_register),
            ("isomp4", gst_plugin_isomp4_register),
            ("matroska", gst_plugin_matroska_register),
            ("ogg", gst_plugin_ogg_register),
            ("vorbis", gst_plugin_vorbis_register),
            ("opus", gst_plugin_opus_register),
            ("flac", gst_plugin_flac_register),
            ("icydemux", gst_plugin_icydemux_register),
            ("soup", gst_plugin_soup_register),
        ];

        for (name, register) in plugins {
            if register() == 0 {
                eprintln!("failed to register GStreamer iOS static plugin: {name}");
            }
        }
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn register_static_plugins() {}

#[cfg(target_os = "android")]
extern "C" {
    fn gst_audio_android_register_static_plugins();
}

#[cfg(target_os = "ios")]
extern "C" {
    fn gst_plugin_coreelements_register() -> glib::ffi::gboolean;
    fn gst_plugin_playback_register() -> glib::ffi::gboolean;
    fn gst_plugin_typefindfunctions_register() -> glib::ffi::gboolean;
    fn gst_plugin_audioconvert_register() -> glib::ffi::gboolean;
    fn gst_plugin_audioparsers_register() -> glib::ffi::gboolean;
    fn gst_plugin_audioresample_register() -> glib::ffi::gboolean;
    fn gst_plugin_volume_register() -> glib::ffi::gboolean;
    fn gst_plugin_level_register() -> glib::ffi::gboolean;
    fn gst_plugin_spectrum_register() -> glib::ffi::gboolean;
    fn gst_plugin_app_register() -> glib::ffi::gboolean;
    fn gst_plugin_autodetect_register() -> glib::ffi::gboolean;
    fn gst_plugin_osxaudio_register() -> glib::ffi::gboolean;
    fn gst_plugin_gio_register() -> glib::ffi::gboolean;
    fn gst_plugin_wavparse_register() -> glib::ffi::gboolean;
    fn gst_plugin_id3demux_register() -> glib::ffi::gboolean;
    fn gst_plugin_isomp4_register() -> glib::ffi::gboolean;
    fn gst_plugin_matroska_register() -> glib::ffi::gboolean;
    fn gst_plugin_ogg_register() -> glib::ffi::gboolean;
    fn gst_plugin_vorbis_register() -> glib::ffi::gboolean;
    fn gst_plugin_opus_register() -> glib::ffi::gboolean;
    fn gst_plugin_flac_register() -> glib::ffi::gboolean;
    fn gst_plugin_icydemux_register() -> glib::ffi::gboolean;
    fn gst_plugin_soup_register() -> glib::ffi::gboolean;
}

struct FadeState {
    start: f64,
    target: f64,
    delta: f64,
    steps: u32,
    current_step: u32,
}

struct GStreamerPlayer {
    playbin: gst::Element,
    playlist: Vec<Track>,
    current_index: Option<usize>,
    is_playing: bool,
    want_playing: bool,
    volume: f64,
    muted: bool,
    speed: f64,
    shuffle: bool,
    repeat_mode: RepeatMode,
    output_device_id: String,
    output_device_name: String,
    output_devices: Vec<gst::Device>,
    audio_sink: gst::Element,
    volume_element: gst::Element,
    fade: Option<FadeState>,
    buffering_percent: i32,
    is_buffering: bool,
    downloaded_bytes: Arc<AtomicU64>,
    download_total_bytes: Option<u64>,
    visualization_frame: VisualizationFrame,
    visualization_pcm: Arc<Mutex<Vec<f64>>>,
    visualization_waveform: VecDeque<f64>,
    visualization_beat_history: VecDeque<f64>,
    visualization_beat_cooldown: u32,
    paused_position_ms: Option<i64>,
    last_error: String,
}

impl GStreamerPlayer {
    fn new() -> Result<Self, String> {
        let playbin = gst::ElementFactory::make("playbin")
            .build()
            .map_err(|err| format!("failed to create playbin: {err}"))?;
        playbin.set_property_from_str("flags", "audio+soft-volume+buffering+download");
        let downloaded_bytes = Arc::new(AtomicU64::new(0));
        let probe_downloaded_bytes = Arc::clone(&downloaded_bytes);
        playbin.connect("source-setup", false, move |values| {
            if let Some(source) = values
                .get(1)
                .and_then(|value| value.get::<gst::Element>().ok())
            {
                install_download_probe(&source, &probe_downloaded_bytes);
            }
            None
        });
        playbin.connect("element-setup", false, move |values| {
            if let Some(element) = values
                .get(1)
                .and_then(|value| value.get::<gst::Element>().ok())
            {
                configure_download_buffer(&element);
            }
            None
        });
        let visualization_pcm = Arc::new(Mutex::new(Vec::new()));
        let (audio_sink, volume_element) =
            build_audio_sink(None, 1.0, false, Arc::clone(&visualization_pcm))?;
        playbin.set_property("audio-sink", &audio_sink);

        Ok(Self {
            playbin,
            playlist: Vec::new(),
            current_index: None,
            is_playing: false,
            want_playing: false,
            volume: 1.0,
            muted: false,
            speed: 1.0,
            shuffle: false,
            repeat_mode: RepeatMode::None,
            output_device_id: String::new(),
            output_device_name: "System default".to_string(),
            output_devices: Vec::new(),
            audio_sink,
            volume_element,
            fade: None,
            buffering_percent: 100,
            is_buffering: false,
            downloaded_bytes,
            download_total_bytes: None,
            visualization_frame: VisualizationFrame::silent(),
            visualization_pcm,
            visualization_waveform: VecDeque::with_capacity(VISUALIZATION_WAVEFORM_POINTS),
            visualization_beat_history: VecDeque::with_capacity(BEAT_HISTORY_FRAMES),
            visualization_beat_cooldown: 0,
            paused_position_ms: None,
            last_error: String::new(),
        })
    }

    fn handle_command(&mut self, command: Command) -> bool {
        match command {
            Command::SetPlaylist {
                tracks,
                start_index,
            } => {
                self.playlist = tracks;
                self.current_index = checked_index(start_index, self.playlist.len());
                if self.current_index.is_some() {
                    self.load_current(false);
                } else {
                    self.stop_pipeline();
                }
                false
            }
            Command::Play => {
                if self.current_index.is_none() && !self.playlist.is_empty() {
                    self.current_index = Some(0);
                    self.load_current(false);
                }
                self.start_playback();
                false
            }
            Command::PlayIndex(index) => {
                if let Some(index) = checked_index(index, self.playlist.len()) {
                    self.current_index = Some(index);
                    self.load_current(true);
                }
                false
            }
            Command::Pause => {
                self.pause_playback();
                self.clear_visualization();
                false
            }
            Command::TogglePlayPause => {
                self.poll_bus();
                if self.want_playing {
                    self.pause_playback();
                    self.clear_visualization();
                } else {
                    self.start_playback();
                }
                false
            }
            Command::Stop => {
                self.stop_pipeline();
                false
            }
            Command::Previous => {
                self.previous_track();
                false
            }
            Command::Next => {
                self.next_track(true);
                false
            }
            Command::Seek(position_ms) => {
                self.seek(position_ms);
                false
            }
            Command::SetVolume(volume) => {
                self.stop_fade();
                self.apply_volume(volume.clamp(0.0, 1.5));
                false
            }
            Command::SetMuted(muted) => {
                self.muted = muted;
                self.volume_element.set_property("mute", muted);
                false
            }
            Command::FadeTo {
                target,
                duration_ms,
            } => {
                self.fade_to(target.clamp(0.0, 1.5), duration_ms);
                false
            }
            Command::SetSpeed(speed) => {
                self.set_speed(speed);
                false
            }
            Command::SetShuffle(shuffle) => {
                self.shuffle = shuffle;
                false
            }
            Command::SetRepeatMode(mode) => {
                self.repeat_mode = mode;
                false
            }
            Command::SetOutputDevice(device_id) => {
                self.set_output_device(device_id);
                false
            }
            Command::RefreshState(reply_tx) => {
                let _ = reply_tx.send(self.snapshot());
                false
            }
            Command::RefreshVisualization(reply_tx) => {
                self.poll_bus();
                let _ = reply_tx.send(self.visualization_snapshot());
                false
            }
            Command::ListOutputDevices(reply_tx) => {
                let _ = reply_tx.send(self.list_output_devices());
                false
            }
            Command::Shutdown(done_tx) => {
                self.stop_fade();
                self.stop_pipeline();
                let _ = done_tx.send(());
                true
            }
        }
    }

    fn load_current(&mut self, autoplay: bool) {
        let Some(track) = self.current_track() else {
            self.stop_pipeline();
            return;
        };
        let uri = track.uri.clone();

        let was_playing = autoplay || self.want_playing || self.is_playing;
        self.stop_pipeline();
        self.paused_position_ms = None;
        self.reset_buffering_for_uri(&uri);
        self.playbin.set_property("uri", &uri);
        self.last_error.clear();

        if was_playing {
            self.start_playback();
        }
    }

    fn start_playback(&mut self) {
        if self.current_index.is_none() && !self.playlist.is_empty() {
            self.current_index = Some(0);
            self.load_current(false);
        }

        if self.current_index.is_some() {
            self.poll_bus();
            let paused_position = self.paused_position_ms.take();
            let resume_position = paused_position.unwrap_or_else(|| self.current_position_ms());
            let needs_resume_seek =
                paused_position.is_some() || self.playbin.current_state() == gst::State::Paused;
            let resume_position = if needs_resume_seek {
                resume_position
            } else {
                self.current_position_ms()
            };
            self.want_playing = true;
            match self.playbin.set_state(gst::State::Playing) {
                Ok(_) => {
                    self.is_playing = true;
                    if needs_resume_seek || (self.speed - 1.0).abs() >= f64::EPSILON {
                        if needs_resume_seek {
                            let _ = self.playbin.state(gst::ClockTime::from_mseconds(250));
                        }
                        self.apply_speed_after_seek(resume_position);
                        if needs_resume_seek {
                            let _ = self.playbin.set_state(gst::State::Playing);
                        }
                    }
                    self.last_error.clear();
                }
                Err(err) => {
                    self.want_playing = false;
                    self.is_playing = false;
                    self.last_error = format!("failed to start playback: {err}");
                }
            }
        }
    }

    fn pause_playback(&mut self) {
        self.paused_position_ms = Some(self.current_position_ms());
        self.want_playing = false;
        match self.playbin.set_state(gst::State::Paused) {
            Ok(_) => {
                self.is_playing = false;
                self.last_error.clear();
            }
            Err(err) => self.last_error = format!("failed to pause playback: {err}"),
        }
    }

    fn stop_pipeline(&mut self) {
        self.want_playing = false;
        let _ = self.playbin.set_state(gst::State::Null);
        let _ = self.playbin.state(gst::ClockTime::from_seconds(2));
        self.is_playing = false;
        self.is_buffering = false;
        self.reset_download_tracking();
        self.buffering_percent = 100;
        self.paused_position_ms = None;
        self.clear_visualization();
    }

    fn previous_track(&mut self) {
        if self.playlist.is_empty() {
            return;
        }

        let index = self.current_index.unwrap_or(0);
        self.current_index = if index > 0 {
            Some(index - 1)
        } else if self.repeat_mode == RepeatMode::All {
            Some(self.playlist.len() - 1)
        } else {
            Some(0)
        };
        self.load_current(true);
    }

    fn next_track(&mut self, user_requested: bool) {
        if self.playlist.is_empty() {
            self.stop_pipeline();
            return;
        }

        if let Some(next_index) = self.compute_next_index(user_requested) {
            self.current_index = Some(next_index);
            self.load_current(true);
        } else {
            self.stop_pipeline();
        }
    }

    fn compute_next_index(&self, user_requested: bool) -> Option<usize> {
        if self.playlist.is_empty() {
            return None;
        }

        let current = self.current_index.unwrap_or(0);
        if !user_requested && self.repeat_mode == RepeatMode::One {
            return Some(current);
        }

        if self.shuffle && self.playlist.len() > 1 {
            let mut next = rand::rng().random_range(0..self.playlist.len());
            if next == current {
                next = (next + 1) % self.playlist.len();
            }
            return Some(next);
        }

        if current + 1 < self.playlist.len() {
            Some(current + 1)
        } else if self.repeat_mode == RepeatMode::All || user_requested {
            Some(0)
        } else {
            None
        }
    }

    fn seek(&mut self, position_ms: i64) {
        if self.current_index.is_none() {
            return;
        }
        let position_ms = position_ms.max(0);
        self.apply_speed_after_seek(position_ms);
        if !self.want_playing {
            self.paused_position_ms = Some(position_ms);
        }
    }

    fn set_speed(&mut self, speed: f64) {
        let clamped = clamp_speed(speed);
        self.speed = clamped;
        self.apply_speed_after_seek(self.current_position_ms());
    }

    fn apply_speed_after_seek(&mut self, position_ms: i64) {
        if self.current_index.is_none() {
            return;
        }

        let position = gst::ClockTime::from_mseconds(position_ms.max(0) as u64);
        let result = if (self.speed - 1.0).abs() < f64::EPSILON {
            self.playbin
                .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, position)
        } else {
            self.playbin.seek(
                self.speed,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                position,
                gst::SeekType::None,
                gst::ClockTime::NONE,
            )
        };

        if let Err(err) = result {
            self.last_error = format!("failed to seek: {err}");
        }
    }

    fn apply_volume(&mut self, volume: f64) {
        self.volume = volume;
        self.volume_element.set_property("volume", volume);
    }

    fn fade_to(&mut self, target: f64, duration_ms: i64) {
        self.stop_fade();
        if duration_ms <= 0 {
            self.apply_volume(target);
            return;
        }

        let steps = ((duration_ms as f64 / 50.0).ceil() as u32).max(1);
        let start = self.volume;
        let delta = (target - start) / steps as f64;

        self.fade = Some(FadeState {
            start,
            target,
            delta,
            steps,
            current_step: 0,
        });
    }

    fn tick_fade(&mut self) {
        let Some(fade) = &mut self.fade else {
            return;
        };

        fade.current_step += 1;
        let next_volume = if fade.current_step >= fade.steps {
            fade.target
        } else {
            fade.start + fade.delta * fade.current_step as f64
        }
        .clamp(0.0, 1.5);

        self.volume = next_volume;
        self.volume_element.set_property("volume", next_volume);

        if fade.current_step >= fade.steps {
            self.fade = None;
        }
    }

    fn stop_fade(&mut self) {
        self.fade = None;
    }

    fn set_output_device(&mut self, device_id: String) {
        match build_audio_sink(
            if device_id.is_empty() {
                None
            } else {
                Some(device_id.as_str())
            },
            self.volume,
            self.muted,
            Arc::clone(&self.visualization_pcm),
        ) {
            Ok((sink, volume)) => {
                let was_playing = self.want_playing || self.is_playing;
                let position = self.current_position_ms();
                let _ = self.playbin.set_state(gst::State::Ready);
                self.playbin.set_property("audio-sink", &sink);
                self.audio_sink = sink;
                self.volume_element = volume;
                self.output_device_id = device_id;
                self.output_device_name = self
                    .list_output_devices()
                    .into_iter()
                    .find(|device| device.id == self.output_device_id)
                    .map(|device| device.name)
                    .unwrap_or_else(|| {
                        if self.output_device_id.is_empty() {
                            "System default".to_string()
                        } else {
                            self.output_device_id.clone()
                        }
                    });

                if was_playing {
                    self.start_playback();
                    self.seek(position);
                }
                self.last_error.clear();
            }
            Err(err) => self.last_error = err,
        }
    }

    fn poll_bus(&mut self) {
        let Some(bus) = self.playbin.bus() else {
            return;
        };

        while let Some(message) = bus.pop() {
            match message.view() {
                gst::MessageView::Eos(_) => self.next_track(false),
                gst::MessageView::Error(err) => {
                    self.want_playing = false;
                    self.is_playing = false;
                    self.is_buffering = false;
                    self.clear_visualization();
                    self.last_error = match err.debug() {
                        Some(debug) => format!("{} ({debug})", err.error()),
                        None => err.error().to_string(),
                    };
                }
                gst::MessageView::StateChanged(state) => {
                    if message
                        .src()
                        .map(|src| {
                            src.as_ptr() == self.playbin.upcast_ref::<gst::Object>().as_ptr()
                        })
                        .unwrap_or(false)
                    {
                        self.is_playing = self.want_playing
                            && !self.is_buffering
                            && state.current() == gst::State::Playing;
                    }
                }
                gst::MessageView::Buffering(buffering) => {
                    let percent = buffering.percent().clamp(0, 100);
                    if percent < 100 {
                        self.is_buffering = true;
                        self.is_playing = false;
                        if self.want_playing {
                            let _ = self.playbin.set_state(gst::State::Paused);
                        }
                    } else if self.want_playing {
                        self.is_buffering = false;
                        let _ = self.playbin.set_state(gst::State::Playing);
                    } else {
                        self.is_buffering = false;
                    }
                }
                gst::MessageView::Element(element) => {
                    if let Some(structure) = element.message().structure() {
                        self.update_http_download_total(structure);
                        self.update_visualization(structure);
                    }
                }
                gst::MessageView::ClockLost(_) => {
                    if self.want_playing {
                        let _ = self.playbin.set_state(gst::State::Paused);
                        let _ = self.playbin.set_state(gst::State::Playing);
                    }
                }
                gst::MessageView::DurationChanged(_) => {}
                _ => {}
            }
        }
    }

    fn snapshot(&mut self) -> PlaybackState {
        self.poll_bus();
        let position_ms = self.current_position_ms();
        let duration_ms = self.current_duration_ms();
        let buffering_percent = self.current_buffering_percent();
        self.sync_playing_state();
        let is_playing = self.playback_active();
        let (current_uri, current_title) = self
            .current_track()
            .map(|track| (track.uri.clone(), track.title.clone()))
            .unwrap_or_else(|| (String::new(), String::new()));

        PlaybackState {
            playlist: self.playlist.clone(),
            current_index: self.current_index.map(|index| index as i32).unwrap_or(-1),
            current_uri,
            current_title,
            is_playing,
            position_ms,
            duration_ms,
            buffering_percent,
            is_buffering: self.is_buffering,
            volume: self.volume,
            muted: self.muted,
            speed: self.speed,
            shuffle: self.shuffle,
            repeat_mode: self.repeat_mode,
            output_device_id: self.output_device_id.clone(),
            output_device_name: self.output_device_name.clone(),
            last_error: self.last_error.clone(),
        }
    }

    fn playback_active(&self) -> bool {
        self.current_index.is_some() && self.want_playing && !self.is_buffering
    }

    fn sync_playing_state(&mut self) {
        self.is_playing = self.want_playing
            && !self.is_buffering
            && self.playbin.current_state() == gst::State::Playing;
    }

    fn visualization_snapshot(&mut self) -> VisualizationFrame {
        self.refresh_pcm_visualization();
        if self.playback_active() {
            self.visualization_frame.beat = false;
            self.visualization_frame.beat_strength *= 0.82;
            return self.visualization_frame.clone();
        }

        self.visualization_frame.is_active = false;
        self.visualization_frame.beat = false;
        self.visualization_frame.beat_strength *= 0.75;
        self.visualization_frame.rms_normalized *= 0.75;
        self.visualization_frame.peak_normalized *= 0.75;
        self.visualization_frame.normalized = self
            .visualization_frame
            .normalized
            .iter()
            .map(|value| value * 0.75)
            .collect();
        self.visualization_frame.waveform = self
            .visualization_frame
            .waveform
            .iter()
            .map(|value| value * 0.75)
            .collect();
        if self
            .visualization_frame
            .normalized
            .iter()
            .all(|value| *value < 0.01)
            && self.visualization_frame.rms_normalized < 0.01
            && self
                .visualization_frame
                .waveform
                .iter()
                .all(|value| *value < 0.01)
        {
            self.clear_visualization();
        }
        self.visualization_frame.clone()
    }

    fn refresh_pcm_visualization(&mut self) {
        let pcm = self
            .visualization_pcm
            .lock()
            .ok()
            .map(|values| values.clone())
            .unwrap_or_default();
        if pcm.is_empty() {
            return;
        }

        self.visualization_frame.pcm = pcm.clone();
        self.visualization_frame.waveform = pcm;
        self.visualization_frame.is_active = true;
    }

    fn list_output_devices(&mut self) -> Vec<AudioOutputDevice> {
        self.refresh_output_devices();

        let mut devices = vec![AudioOutputDevice {
            id: String::new(),
            name: "System default".to_string(),
            device_class: "Audio/Sink".to_string(),
            is_current: self.output_device_id.is_empty(),
        }];

        devices.extend(
            self.output_devices
                .iter()
                .enumerate()
                .map(|(index, device)| {
                    let id = device_id(device, index);
                    AudioOutputDevice {
                        id: id.clone(),
                        name: device.display_name().to_string(),
                        device_class: device.device_class().to_string(),
                        is_current: id == self.output_device_id,
                    }
                }),
        );

        devices
    }

    fn refresh_output_devices(&mut self) {
        let monitor = gst::DeviceMonitor::new();
        monitor.set_show_all_devices(true);
        let _ = monitor.add_filter(Some("Audio/Sink"), None);

        if monitor.start().is_ok() {
            self.output_devices = monitor.devices().iter().cloned().collect();
            monitor.stop();
        } else {
            self.output_devices.clear();
        }
    }

    fn current_track(&self) -> Option<&Track> {
        self.current_index
            .and_then(|index| self.playlist.get(index))
    }

    fn current_position_ms(&self) -> i64 {
        self.playbin
            .query_position::<gst::ClockTime>()
            .map(|time| time.mseconds() as i64)
            .unwrap_or(0)
    }

    fn current_duration_ms(&self) -> i64 {
        self.playbin
            .query_duration::<gst::ClockTime>()
            .map(|time| time.mseconds() as i64)
            .unwrap_or(0)
    }

    fn current_buffering_percent(&mut self) -> i32 {
        let is_http = self
            .current_track()
            .map(|track| is_http_uri(&track.uri))
            .unwrap_or(false);
        if !is_http {
            return 100;
        }

        if let Some(percent) = self.current_transfer_percent() {
            self.buffering_percent = self.buffering_percent.max(percent).clamp(0, 100);
        }

        self.buffering_percent
    }

    fn current_transfer_percent(&self) -> Option<i32> {
        let total_bytes = self.download_total_bytes?;
        if total_bytes == 0 {
            return None;
        }

        let downloaded_bytes = self.downloaded_bytes.load(Ordering::Relaxed);
        if downloaded_bytes == 0 {
            return None;
        }

        let percent = downloaded_bytes.min(total_bytes).saturating_mul(100) / total_bytes;
        Some(percent as i32)
    }

    fn reset_buffering_for_uri(&mut self, uri: &str) {
        self.reset_download_tracking();
        self.buffering_percent = if is_http_uri(uri) { 0 } else { 100 };
        self.is_buffering = false;
    }

    fn reset_download_tracking(&mut self) {
        self.downloaded_bytes.store(0, Ordering::Relaxed);
        self.download_total_bytes = None;
    }

    fn update_http_download_total(&mut self, structure: &gst::StructureRef) {
        if !structure.has_name("http-headers") {
            return;
        }

        let is_current_http = self
            .current_track()
            .map(|track| is_http_uri(&track.uri))
            .unwrap_or(false);
        if !is_current_http {
            return;
        }

        let Ok(response_headers) = structure.get::<gst::Structure>("response-headers") else {
            return;
        };
        let Some(total_bytes) = http_content_total(response_headers.as_ref()) else {
            return;
        };

        self.download_total_bytes = Some(total_bytes);
    }

    fn update_visualization(&mut self, structure: &gst::StructureRef) {
        if let Some(level) = LevelFrame::from_structure(structure) {
            self.update_level_visualization(level);
            return;
        }

        let Some(frame) = VisualizationFrame::from_spectrum_structure(structure) else {
            return;
        };

        let frame = self.enrich_spectrum_frame(frame);
        if self.playback_active() {
            self.visualization_frame = frame;
        } else {
            self.visualization_frame = VisualizationFrame {
                is_active: false,
                ..frame
            };
        }
    }

    fn update_level_visualization(&mut self, level: LevelFrame) {
        self.visualization_frame.timestamp_ms = self
            .visualization_frame
            .timestamp_ms
            .max(level.timestamp_ms);
        self.visualization_frame.rms = level.rms;
        self.visualization_frame.peak = level.peak;
        self.visualization_frame.decay = level.decay;
        self.visualization_frame.rms_normalized = level.rms_normalized;
        self.visualization_frame.peak_normalized = level.peak_normalized;

        if self.visualization_frame.waveform.is_empty() {
            push_limited(
                &mut self.visualization_waveform,
                level.peak_normalized.max(level.rms_normalized),
                VISUALIZATION_WAVEFORM_POINTS,
            );
            self.visualization_frame.waveform =
                self.visualization_waveform.iter().copied().collect();
        }

        let (beat, strength) = self.detect_beat(level.rms_normalized);
        self.visualization_frame.beat = beat && self.playback_active();
        self.visualization_frame.beat_strength = if self.visualization_frame.beat {
            strength
        } else {
            self.visualization_frame.beat_strength.max(strength * 0.5)
        };
        self.visualization_frame.is_active =
            self.visualization_frame.is_active || level.rms_normalized > 0.01;
    }

    fn enrich_spectrum_frame(&mut self, mut frame: VisualizationFrame) -> VisualizationFrame {
        frame.normalized = smooth_values(&self.visualization_frame.normalized, &frame.normalized);
        frame.rms = self.visualization_frame.rms.clone();
        frame.peak = self.visualization_frame.peak.clone();
        frame.decay = self.visualization_frame.decay.clone();
        frame.rms_normalized = self.visualization_frame.rms_normalized;
        frame.peak_normalized = self.visualization_frame.peak_normalized;
        frame.beat = self.visualization_frame.beat;
        frame.beat_strength = self.visualization_frame.beat_strength;
        frame.pcm = self.visualization_frame.pcm.clone();
        frame.waveform = if self.visualization_frame.waveform.is_empty() {
            self.visualization_waveform.iter().copied().collect()
        } else {
            self.visualization_frame.waveform.clone()
        };
        frame.is_active =
            frame.is_active || frame.rms_normalized > 0.01 || frame.peak_normalized > 0.01;
        frame
    }

    fn detect_beat(&mut self, energy: f64) -> (bool, f64) {
        let history_average = if self.visualization_beat_history.is_empty() {
            0.0
        } else {
            self.visualization_beat_history.iter().sum::<f64>()
                / self.visualization_beat_history.len() as f64
        };
        let strength = if history_average > 0.0 {
            ((energy - history_average) / history_average.max(0.01)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let beat = self.visualization_beat_cooldown == 0
            && energy > BEAT_MIN_ENERGY
            && strength > BEAT_STRENGTH_THRESHOLD;

        if beat {
            self.visualization_beat_cooldown = BEAT_COOLDOWN_FRAMES;
        } else {
            self.visualization_beat_cooldown = self.visualization_beat_cooldown.saturating_sub(1);
        }

        push_limited(
            &mut self.visualization_beat_history,
            energy,
            BEAT_HISTORY_FRAMES,
        );

        (beat, strength)
    }

    fn clear_visualization(&mut self) {
        self.visualization_frame = VisualizationFrame::silent();
        if let Ok(mut pcm) = self.visualization_pcm.lock() {
            pcm.clear();
        }
        self.visualization_waveform.clear();
        self.visualization_beat_history.clear();
        self.visualization_beat_cooldown = 0;
    }
}

fn install_download_probe(element: &gst::Element, downloaded_bytes: &Arc<AtomicU64>) -> bool {
    if is_http_source_element(element) {
        return install_download_probe_on_source(element, downloaded_bytes);
    }

    if let Some(bin) = element.dynamic_cast_ref::<gst::Bin>() {
        for child in bin.iterate_recurse().into_iter().flatten() {
            configure_download_buffer(&child);
            if install_download_probe(&child, downloaded_bytes) {
                return true;
            }
        }
    }

    false
}

fn is_http_source_element(element: &gst::Element) -> bool {
    let factory_name = element
        .factory()
        .map(|factory| factory.name().to_string())
        .unwrap_or_default();
    if matches!(factory_name.as_str(), "souphttpsrc" | "neonhttpsrc") {
        return true;
    }

    if element.find_property("location").is_none() {
        return false;
    }

    element
        .property::<Option<String>>("location")
        .as_deref()
        .map(is_http_uri)
        .unwrap_or(false)
}

fn install_download_probe_on_source(
    element: &gst::Element,
    downloaded_bytes: &Arc<AtomicU64>,
) -> bool {
    let Some(src_pad) = element.static_pad("src") else {
        return false;
    };

    let probe_downloaded_bytes = Arc::clone(downloaded_bytes);
    src_pad.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
        if let Some(buffer) = info.buffer() {
            probe_downloaded_bytes.fetch_add(buffer.size() as u64, Ordering::Relaxed);
        }
        gst::PadProbeReturn::Ok
    });
    true
}

fn configure_download_buffer(element: &gst::Element) {
    if element
        .factory()
        .map(|factory| factory.name().to_string())
        .as_deref()
        != Some("downloadbuffer")
    {
        return;
    }

    let temp_template = env::temp_dir().join("gst-audio-download-XXXXXX");
    if let Some(template) = temp_template.to_str() {
        element.set_property("temp-template", template);
    }
}

fn http_content_total(headers: &gst::StructureRef) -> Option<u64> {
    http_header_value(headers, "Content-Range")
        .and_then(|value| parse_content_range_total(&value))
        .or_else(|| {
            http_header_value(headers, "Content-Length")
                .and_then(|value| value.trim().parse::<u64>().ok())
        })
}

fn http_header_value(headers: &gst::StructureRef, name: &str) -> Option<String> {
    if let Ok(value) = headers.get::<String>(name) {
        return Some(value);
    }

    headers
        .iter()
        .find(|(field, _)| field.as_str().eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.get::<String>().ok())
}

fn parse_content_range_total(value: &str) -> Option<u64> {
    let (_, total) = value.rsplit_once('/')?;
    let total = total.trim();
    if total == "*" {
        None
    } else {
        total.parse::<u64>().ok()
    }
}

fn numeric_values(structure: &gst::StructureRef, name: &str) -> Option<Vec<f64>> {
    let values = structure
        .get::<ValueArray>(name)
        .ok()
        .map(|values| {
            values
                .as_slice()
                .iter()
                .filter_map(numeric_value)
                .collect::<Vec<_>>()
        })
        .or_else(|| {
            structure.get::<gst::List>(name).ok().map(|values| {
                values
                    .as_slice()
                    .iter()
                    .filter_map(|value| numeric_value(value))
                    .collect::<Vec<_>>()
            })
        })?;

    let values = values
        .into_iter()
        .map(|value| {
            if value.is_finite() {
                value
            } else {
                SPECTRUM_THRESHOLD_DB as f64
            }
        })
        .collect::<Vec<_>>();

    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

fn average_normalized_db(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.iter().map(|value| normalize_db(*value)).sum::<f64>() / values.len() as f64
}

fn smooth_values(previous: &[f64], next: &[f64]) -> Vec<f64> {
    if previous.len() != next.len() {
        return next.to_vec();
    }

    previous
        .iter()
        .zip(next.iter())
        .map(|(previous, next)| {
            let factor = if next > previous {
                VISUALIZATION_SMOOTHING_ATTACK
            } else {
                VISUALIZATION_SMOOTHING_RELEASE
            };
            previous + (next - previous) * factor
        })
        .collect()
}

fn push_limited(values: &mut VecDeque<f64>, value: f64, limit: usize) {
    if values.len() == limit {
        values.pop_front();
    }
    values.push_back(value.clamp(0.0, 1.0));
}

fn attach_pcm_appsink(appsink: &gst::Element, visualization_pcm: Arc<Mutex<Vec<f64>>>) {
    appsink.connect("new-sample", false, move |values| {
        if let Some(appsink) = values
            .first()
            .and_then(|value| value.get::<gst::Element>().ok())
        {
            let sample = appsink.emit_by_name::<Option<gst::Sample>>("pull-sample", &[]);
            if let Some(pcm) = sample.as_ref().and_then(pcm_from_sample) {
                if let Ok(mut latest_pcm) = visualization_pcm.lock() {
                    *latest_pcm = pcm;
                }
            }
        }
        Some(gst::FlowReturn::Ok.to_value())
    });
}

fn pcm_from_sample(sample: &gst::Sample) -> Option<Vec<f64>> {
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    let bytes = map.as_slice();
    if bytes.len() < 4 {
        return None;
    }

    let samples = bytes
        .chunks_exact(4)
        .map(|chunk| {
            f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]).clamp(-1.0, 1.0) as f64
        })
        .collect::<Vec<_>>();
    Some(downsample_pcm(&samples, VISUALIZATION_WAVEFORM_POINTS))
}

fn downsample_pcm(samples: &[f64], points: usize) -> Vec<f64> {
    if samples.is_empty() {
        return Vec::new();
    }
    if samples.len() <= points {
        return samples.to_vec();
    }

    let samples_per_point = samples.len() as f64 / points as f64;
    (0..points)
        .map(|index| {
            let start = (index as f64 * samples_per_point).floor() as usize;
            let end = (((index + 1) as f64 * samples_per_point).ceil() as usize)
                .min(samples.len())
                .max(start + 1);
            let window = &samples[start..end];
            let peak = window
                .iter()
                .copied()
                .max_by(|a, b| a.abs().total_cmp(&b.abs()))
                .unwrap_or(0.0);
            peak.clamp(-1.0, 1.0)
        })
        .collect()
}

fn link_tee_branch(tee: &gst::Element, branch: &gst::Element) -> Result<(), String> {
    let tee_pad = tee
        .request_pad_simple("src_%u")
        .ok_or_else(|| "tee did not provide a src pad".to_string())?;
    let branch_pad = branch
        .static_pad("sink")
        .ok_or_else(|| format!("{} has no sink pad", branch.name()))?;
    tee_pad
        .link(&branch_pad)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn numeric_value(value: &Value) -> Option<f64> {
    value
        .get::<f64>()
        .ok()
        .or_else(|| value.get::<f32>().ok().map(f64::from))
        .or_else(|| value.get::<i32>().ok().map(f64::from))
}

fn normalize_db(value: f64) -> f64 {
    if !value.is_finite() || value <= VISUALIZATION_NOISE_GATE_DB {
        return 0.0;
    }

    let floor_db = SPECTRUM_THRESHOLD_DB as f64;
    ((value.clamp(floor_db, 0.0) - floor_db) / -floor_db).clamp(0.0, 1.0)
}

fn build_audio_sink(
    device_id: Option<&str>,
    volume: f64,
    muted: bool,
    visualization_pcm: Arc<Mutex<Vec<f64>>>,
) -> Result<(gst::Element, gst::Element), String> {
    let bin = gst::Bin::new();
    let volume_element = gst::ElementFactory::make("volume")
        .build()
        .map_err(|err| format!("failed to create volume element: {err}"))?;
    volume_element.set_property("volume", volume.clamp(0.0, 1.5));
    volume_element.set_property("mute", muted);

    let convert = gst::ElementFactory::make("audioconvert")
        .build()
        .map_err(|err| format!("failed to create audioconvert element: {err}"))?;
    let resample = gst::ElementFactory::make("audioresample")
        .build()
        .map_err(|err| format!("failed to create audioresample element: {err}"))?;
    let level = gst::ElementFactory::make("level")
        .build()
        .map_err(|err| format!("failed to create level element: {err}"))?;
    level.set_property("post-messages", true);
    level.set_property("interval", VISUALIZATION_INTERVAL_MS * 1_000_000);
    let spectrum = gst::ElementFactory::make("spectrum")
        .build()
        .map_err(|err| format!("failed to create spectrum element: {err}"))?;
    spectrum.set_property("bands", SPECTRUM_BANDS);
    spectrum.set_property("threshold", SPECTRUM_THRESHOLD_DB);
    spectrum.set_property("post-messages", true);
    spectrum.set_property("message-magnitude", true);
    spectrum.set_property("message-phase", false);
    spectrum.set_property("interval", VISUALIZATION_INTERVAL_MS * 1_000_000);
    let tee = gst::ElementFactory::make("tee")
        .build()
        .map_err(|err| format!("failed to create tee element: {err}"))?;
    let playback_queue = gst::ElementFactory::make("queue")
        .build()
        .map_err(|err| format!("failed to create playback queue element: {err}"))?;
    let pcm_queue = gst::ElementFactory::make("queue")
        .build()
        .map_err(|err| format!("failed to create pcm queue element: {err}"))?;
    pcm_queue.set_property_from_str("leaky", "downstream");
    pcm_queue.set_property("max-size-buffers", 2u32);
    pcm_queue.set_property("max-size-bytes", 0u32);
    pcm_queue.set_property("max-size-time", 0u64);
    let pcm_convert = gst::ElementFactory::make("audioconvert")
        .build()
        .map_err(|err| format!("failed to create pcm audioconvert element: {err}"))?;
    let pcm_resample = gst::ElementFactory::make("audioresample")
        .build()
        .map_err(|err| format!("failed to create pcm audioresample element: {err}"))?;
    let pcm_caps = gst::Caps::builder("audio/x-raw")
        .field("format", "F32LE")
        .field("channels", 1i32)
        .field("layout", "interleaved")
        .build();
    let pcm_capsfilter = gst::ElementFactory::make("capsfilter")
        .property("caps", &pcm_caps)
        .build()
        .map_err(|err| format!("failed to create pcm capsfilter element: {err}"))?;
    let pcm_sink = gst::ElementFactory::make("appsink")
        .build()
        .map_err(|err| format!("failed to create appsink element: {err}"))?;
    pcm_sink.set_property("emit-signals", true);
    pcm_sink.set_property("sync", false);
    pcm_sink.set_property("drop", true);
    pcm_sink.set_property("max-buffers", 1u32);
    pcm_sink.set_property("enable-last-sample", false);
    attach_pcm_appsink(&pcm_sink, visualization_pcm);

    let sink = match device_id {
        Some(id) if !id.is_empty() => create_device_sink(id)?,
        #[cfg(target_os = "android")]
        _ => gst::ElementFactory::make("openslessink")
            .build()
            .map_err(|err| format!("failed to create openslessink: {err}"))?,
        #[cfg(target_os = "ios")]
        _ => gst::ElementFactory::make("osxaudiosink")
            .build()
            .map_err(|err| format!("failed to create osxaudiosink: {err}"))?,
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        _ => {
            let factory = env::var("GST_AUDIO_FLUTTER_AUDIO_SINK")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "autoaudiosink".to_string());
            let sink = gst::ElementFactory::make(factory.trim())
                .build()
                .map_err(|err| format!("failed to create {factory} element: {err}"))?;
            if factory.trim() == "fakesink" {
                sink.set_property("sync", true);
            }
            sink
        }
    };

    bin.add_many([
        &volume_element,
        &convert,
        &resample,
        &level,
        &spectrum,
        &tee,
        &playback_queue,
        &sink,
        &pcm_queue,
        &pcm_convert,
        &pcm_resample,
        &pcm_capsfilter,
        &pcm_sink,
    ])
    .map_err(|err| format!("failed to build audio sink bin: {err}"))?;
    gst::Element::link_many([
        &volume_element,
        &convert,
        &resample,
        &level,
        &spectrum,
        &tee,
    ])
    .map_err(|err| format!("failed to link audio analyzer chain: {err}"))?;
    gst::Element::link_many([&playback_queue, &sink])
        .map_err(|err| format!("failed to link audio playback branch: {err}"))?;
    gst::Element::link_many([
        &pcm_queue,
        &pcm_convert,
        &pcm_resample,
        &pcm_capsfilter,
        &pcm_sink,
    ])
    .map_err(|err| format!("failed to link pcm visualization branch: {err}"))?;
    link_tee_branch(&tee, &playback_queue)
        .map_err(|err| format!("failed to link tee playback branch: {err}"))?;
    link_tee_branch(&tee, &pcm_queue)
        .map_err(|err| format!("failed to link tee pcm branch: {err}"))?;

    let sink_pad = volume_element
        .static_pad("sink")
        .ok_or_else(|| "volume element has no sink pad".to_string())?;
    let ghost_pad = gst::GhostPad::with_target(&sink_pad)
        .map_err(|err| format!("failed to create audio sink ghost pad: {err}"))?;
    bin.add_pad(&ghost_pad)
        .map_err(|err| format!("failed to add audio sink ghost pad: {err}"))?;

    Ok((bin.upcast::<gst::Element>(), volume_element))
}

fn create_device_sink(selected_device_id: &str) -> Result<gst::Element, String> {
    let monitor = gst::DeviceMonitor::new();
    monitor.set_show_all_devices(true);
    let _ = monitor.add_filter(Some("Audio/Sink"), None);
    monitor
        .start()
        .map_err(|err| format!("failed to start GStreamer device monitor: {err}"))?;

    let devices = monitor.devices();
    let result = devices
        .iter()
        .enumerate()
        .find(|(index, device)| device_id(device, *index) == selected_device_id)
        .map(|(_, device)| device.create_element(Some("selected-audio-sink")))
        .transpose()
        .map_err(|err| format!("failed to create sink for selected device: {err}"))?
        .ok_or_else(|| format!("audio output device not found: {selected_device_id}"));

    monitor.stop();
    result
}

fn device_id(device: &gst::Device, index: usize) -> String {
    let class = device.device_class();
    let name = device.display_name();
    let properties = device.properties();
    let stable = properties
        .as_ref()
        .and_then(|props| {
            for key in ["device.path", "device.name", "sysfs.path", "api.alsa.path"] {
                if let Ok(value) = props.get::<&str>(key) {
                    return Some(value.to_string());
                }
            }
            None
        })
        .unwrap_or_else(|| format!("{}:{name}:{index}", class.as_str()));
    format!("{index}:{stable}")
}

fn checked_index(index: i32, len: usize) -> Option<usize> {
    if index < 0 || index as usize >= len {
        None
    } else {
        Some(index as usize)
    }
}

fn input_to_uri(input: &str) -> String {
    let trimmed = input.trim();
    if let Some(asset_path) = trimmed.strip_prefix("asset:///") {
        return flutter_asset_to_uri(asset_path);
    }
    if looks_like_uri(trimmed) {
        trimmed.to_string()
    } else {
        gio::File::for_commandline_arg(trimmed).uri().to_string()
    }
}

fn title_from_input(input: &str) -> String {
    let trimmed = input.trim();
    if looks_like_uri(trimmed) {
        trimmed
            .rsplit('/')
            .next()
            .filter(|item| !item.is_empty())
            .unwrap_or(trimmed)
            .to_string()
    } else {
        Path::new(trimmed)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(trimmed)
            .to_string()
    }
}

fn looks_like_uri(input: &str) -> bool {
    input.contains("://") || input.starts_with("file:")
}

fn is_http_uri(uri: &str) -> bool {
    uri.starts_with("http://") || uri.starts_with("https://")
}

fn flutter_asset_to_uri(asset_path: &str) -> String {
    let normalized_asset_path = asset_path.trim_start_matches('/');

    #[cfg(target_os = "ios")]
    {
        if let Ok(executable) = env::current_exe() {
            if let Some(app_dir) = executable.parent() {
                let asset = app_dir
                    .join("Frameworks")
                    .join("App.framework")
                    .join("flutter_assets")
                    .join(normalized_asset_path);
                if asset.is_file() {
                    return gio::File::for_path(asset).uri().to_string();
                }
            }
        }
    }

    format!("asset:///{normalized_asset_path}")
}

fn clamp_speed(speed: f64) -> f64 {
    const ALLOWED: [f64; 4] = [0.5, 1.0, 1.5, 2.0];
    ALLOWED
        .iter()
        .copied()
        .min_by(|a, b| {
            (speed - *a)
                .abs()
                .partial_cmp(&(speed - *b).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(1.0)
}
