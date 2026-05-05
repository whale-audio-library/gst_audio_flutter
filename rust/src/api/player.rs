use std::{
    path::Path,
    sync::{
        mpsc::{self, Sender, TryRecvError},
        Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use gio::prelude::*;
use gst::prelude::*;
use gstreamer as gst;
use rand::Rng;

#[cfg(any(target_os = "ios", target_os = "windows"))]
use std::env;

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

#[cfg(target_os = "ios")]
fn configure_platform_gstreamer_runtime() {
    let tmp_dir = env::temp_dir();
    set_env_path_if_missing("TMP", &tmp_dir);
    set_env_path_if_missing("TEMP", &tmp_dir);
    set_env_path_if_missing("TMPDIR", &tmp_dir);
    set_env_path_if_missing("XDG_RUNTIME_DIR", &tmp_dir);
    set_env_path_if_missing("XDG_CACHE_HOME", &tmp_dir);
    set_env_path_if_missing("HOME", &tmp_dir);
}

#[cfg(not(any(target_os = "ios", target_os = "windows")))]
fn configure_platform_gstreamer_runtime() {}

#[cfg(target_os = "ios")]
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
    last_error: String,
}

impl GStreamerPlayer {
    fn new() -> Result<Self, String> {
        let playbin = gst::ElementFactory::make("playbin")
            .build()
            .map_err(|err| format!("failed to create playbin: {err}"))?;
        playbin.set_property_from_str("flags", "audio+soft-volume+buffering");
        let (audio_sink, volume_element) = build_audio_sink(None, 1.0, false)?;
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
                false
            }
            Command::TogglePlayPause => {
                if self.is_playing {
                    self.pause_playback();
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
            self.want_playing = true;
            match self.playbin.set_state(gst::State::Playing) {
                Ok(_) => {
                    self.is_playing = true;
                    if (self.speed - 1.0).abs() >= f64::EPSILON {
                        self.apply_speed_after_seek(self.current_position_ms());
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
        self.buffering_percent = 100;
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
        self.apply_speed_after_seek(position_ms.max(0));
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
                        self.is_playing = state.current() == gst::State::Playing;
                    }
                }
                gst::MessageView::Buffering(buffering) => {
                    let percent = buffering.percent().clamp(0, 100);
                    self.buffering_percent = percent;
                    if percent < 100 {
                        self.is_buffering = true;
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
        let (current_uri, current_title) = self
            .current_track()
            .map(|track| (track.uri.clone(), track.title.clone()))
            .unwrap_or_else(|| (String::new(), String::new()));

        PlaybackState {
            playlist: self.playlist.clone(),
            current_index: self.current_index.map(|index| index as i32).unwrap_or(-1),
            current_uri,
            current_title,
            is_playing: self.is_playing,
            position_ms,
            duration_ms,
            buffering_percent: self.buffering_percent,
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

    fn reset_buffering_for_uri(&mut self, uri: &str) {
        self.buffering_percent = if is_http_uri(uri) { 0 } else { 100 };
        self.is_buffering = false;
    }
}

fn build_audio_sink(
    device_id: Option<&str>,
    volume: f64,
    muted: bool,
) -> Result<(gst::Element, gst::Element), String> {
    let bin = gst::Bin::new();
    let volume_element = gst::ElementFactory::make("volume")
        .build()
        .map_err(|err| format!("failed to create volume element: {err}"))?;
    volume_element.set_property("volume", volume.clamp(0.0, 1.5));
    volume_element.set_property("mute", muted);

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
        _ => gst::ElementFactory::make("autoaudiosink")
            .build()
            .map_err(|err| format!("failed to create autoaudiosink: {err}"))?,
    };

    bin.add_many([&volume_element, &sink])
        .map_err(|err| format!("failed to build audio sink bin: {err}"))?;
    volume_element
        .link(&sink)
        .map_err(|err| format!("failed to link audio sink bin: {err}"))?;

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
