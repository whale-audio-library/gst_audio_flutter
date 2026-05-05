use std::{
    collections::BTreeSet,
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const ANDROID_PLUGINS: &[&str] = &[
    "coreelements",
    "playback",
    "typefindfunctions",
    "audioconvert",
    "audioparsers",
    "audioresample",
    "volume",
    "autodetect",
    "opensles",
    "gio",
    "wavparse",
    "id3demux",
    "isomp4",
    "matroska",
    "ogg",
    "vorbis",
    "opus",
    "flac",
    "icydemux",
    "mpg123",
    "soup",
];

const ANDROID_GIO_MODULES: &[&str] = &["openssl"];

fn main() {
    println!("cargo:rerun-if-env-changed=GSTREAMER_ANDROID_PREFIX");
    println!("cargo:rerun-if-env-changed=GSTREAMER_ROOT_ANDROID");
    println!("cargo:rerun-if-env-changed=GSTREAMER_ANDROID_ABI");
    println!("cargo:rerun-if-env-changed=GSTREAMER_IOS_LIBRARY_DIR");
    println!("cargo:rerun-if-env-changed=TARGET");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        // GStreamer iOS bundles GIO resolver objects that need libresolv.
        println!("cargo:rustc-link-lib=resolv");
        return;
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        return;
    }

    let prefix = android_prefix();
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let init_c = out_dir.join("gst_android_static_plugins.c");
    fs::write(&init_c, android_init_source()).expect("failed to write GStreamer Android init C");

    let cflags = pkg_config_args(
        &prefix,
        "pkg-config",
        &[
            "--cflags",
            "gio-2.0",
            "gstreamer-1.0",
            "gstreamer-audio-1.0",
            "gstreamer-base-1.0",
        ],
    );

    let mut build = cc::Build::new();
    build.file(&init_c);
    for flag in cflags {
        if flag != "-pthread" {
            build.flag(&flag);
        }
    }
    build.compile("gst_android_static_plugins");

    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib/gstreamer-1.0").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib/gio/modules").display()
    );

    println!("cargo:rustc-link-arg=-Wl,--whole-archive");
    for plugin in ANDROID_PLUGINS {
        println!("cargo:rustc-link-lib=static=gst{plugin}");
    }
    for module in ANDROID_GIO_MODULES {
        println!("cargo:rustc-link-lib=static=gio{module}");
    }
    println!("cargo:rustc-link-arg=-Wl,--no-whole-archive");

    for lib in pkg_config_libs(&prefix, ANDROID_PLUGINS, ANDROID_GIO_MODULES) {
        if is_system_link_arg(&lib) {
            println!("cargo:rustc-link-lib={lib}");
        } else {
            println!("cargo:rustc-link-lib=static={lib}");
        }
    }
}

fn android_prefix() -> PathBuf {
    if let Some(value) = env::var_os("GSTREAMER_ANDROID_PREFIX") {
        return PathBuf::from(value);
    }

    let root = PathBuf::from(
        env::var_os("GSTREAMER_ROOT_ANDROID")
            .expect("GSTREAMER_ROOT_ANDROID must point to the Android GStreamer SDK"),
    );
    let abi =
        env::var("GSTREAMER_ANDROID_ABI").unwrap_or_else(|_| match env::var("TARGET").as_deref() {
            Ok("armv7-linux-androideabi") => "armv7".to_string(),
            Ok("aarch64-linux-android") => "arm64".to_string(),
            Ok("i686-linux-android") => "x86".to_string(),
            Ok("x86_64-linux-android") => "x86_64".to_string(),
            Ok(target) => panic!("unsupported Android Rust target: {target}"),
            Err(_) => panic!("TARGET is not set"),
        });
    root.join(abi)
}

fn android_init_source() -> String {
    let declarations = ANDROID_PLUGINS
        .iter()
        .map(|plugin| format!("GST_PLUGIN_STATIC_DECLARE({plugin});"))
        .collect::<Vec<_>>()
        .join("\n");
    let registrations = ANDROID_PLUGINS
        .iter()
        .map(|plugin| format!("  GST_PLUGIN_STATIC_REGISTER({plugin});"))
        .collect::<Vec<_>>()
        .join("\n");
    let gio_declarations = ANDROID_GIO_MODULES
        .iter()
        .map(|module| format!("GST_G_IO_MODULE_DECLARE({module});"))
        .collect::<Vec<_>>()
        .join("\n");
    let gio_loads = ANDROID_GIO_MODULES
        .iter()
        .map(|module| format!("  GST_G_IO_MODULE_LOAD({module});"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"#include <gio/gio.h>
#include <gst/gst.h>

#define GST_G_IO_MODULE_DECLARE(name) \
extern void G_PASTE(g_io_, G_PASTE(name, _load)) (gpointer module)

#define GST_G_IO_MODULE_LOAD(name) \
G_PASTE(g_io_, G_PASTE(name, _load)) (NULL)

{declarations}

{gio_declarations}

void
gst_audio_android_register_static_plugins (void)
{{
{registrations}
{gio_loads}
}}
"#
    )
}

fn pkg_config_libs(prefix: &Path, plugins: &[&str], gio_modules: &[&str]) -> Vec<String> {
    let mut packages = Vec::new();
    packages.extend(plugins.iter().map(|plugin| format!("gst{plugin}")));
    packages.extend(gio_modules.iter().map(|module| format!("gio{module}")));

    let package_refs = packages.iter().map(String::as_str).collect::<Vec<_>>();
    let args = ["--libs", "--static"]
        .into_iter()
        .chain(package_refs)
        .collect::<Vec<_>>();
    let raw_args = pkg_config_args(prefix, "pkg-config", &args);
    let mut libs = BTreeSet::new();
    let skipped_libs = plugins
        .iter()
        .map(|plugin| format!("gst{plugin}"))
        .chain(gio_modules.iter().map(|module| format!("gio{module}")))
        .collect::<BTreeSet<_>>();

    for arg in raw_args {
        if let Some(lib) = arg.strip_prefix("-l") {
            if let Some(name) = lib.strip_prefix(':') {
                if let Some(name) = name.strip_prefix("lib").and_then(|n| n.strip_suffix(".a")) {
                    libs.insert(name.to_string());
                }
            } else if !skipped_libs.contains(lib) {
                libs.insert(lib.to_string());
            }
        }
    }

    libs.into_iter().collect()
}

fn pkg_config_args(prefix: &Path, executable: &str, args: &[&str]) -> Vec<String> {
    let separator = if cfg!(windows) { ";" } else { ":" };
    let pkg_config_path = [
        prefix.join("lib/pkgconfig"),
        prefix.join("lib/gstreamer-1.0/pkgconfig"),
        prefix.join("lib/gio/modules/pkgconfig"),
    ]
    .iter()
    .map(PathBuf::as_path)
    .map(Path::display)
    .map(|path| path.to_string())
    .collect::<Vec<_>>()
    .join(separator);

    let output = Command::new(executable)
        .args(args.iter().map(OsStr::new))
        .env("PKG_CONFIG_ALLOW_CROSS", "1")
        .env("PKG_CONFIG_PATH", &pkg_config_path)
        .env("PKG_CONFIG_LIBDIR", &pkg_config_path)
        .env("PKG_CONFIG_ALL_STATIC", "1")
        .output()
        .expect("failed to run pkg-config for GStreamer Android SDK");

    if !output.status.success() {
        panic!(
            "pkg-config failed for GStreamer Android SDK: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(ToString::to_string)
        .collect()
}

fn is_system_link_arg(lib: &str) -> bool {
    matches!(
        lib,
        "android"
            | "atomic"
            | "c"
            | "dl"
            | "EGL"
            | "GLESv2"
            | "log"
            | "m"
            | "OpenSLES"
            | "pthread"
            | "unwind"
            | "vulkan"
    )
}
