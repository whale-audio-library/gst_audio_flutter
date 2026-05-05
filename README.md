# gst_audio_flutter

Flutter + Rust audio player. Flutter owns the UI; Rust owns the playback queue and GStreamer control logic through `gstreamer-rs` and `flutter_rust_bridge`.

Project root used in this workspace:

```text
/home/chrome-book/code/gst_audio_flutter
```

For cross-platform GStreamer usage, setup, packaging status, and common issues, see [docs/gstreamer-cross-platform-guide.md](docs/gstreamer-cross-platform-guide.md).

For Android-only SDK/static-plugin/APK details, see [docs/gstreamer-android-guide.md](docs/gstreamer-android-guide.md).

## Toolchain

- Flutter SDK used here: `/home/chrome-book/fvm/versions/3.41.6/bin/flutter`
- Rust: Cargo 1.94.1
- FRB codegen: `flutter_rust_bridge_codegen 2.11.1`
- GStreamer development packages: `glib-2.0` and `gstreamer-1.0` from `pkg-config`

If `flutter` is not on `PATH`, run commands with:

```bash
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH <command>
```

## Build And Run

```bash
cd /home/chrome-book/code/gst_audio_flutter
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter run -d linux
```

Build release bundle:

```bash
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter build linux
./build/linux/x64/release/bundle/gst_audio_flutter
```

Regenerate FRB bindings after changing Rust public API:

```bash
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter_rust_bridge_codegen generate
```

## Android

Android is one supported target, not the only project target. Read the cross-platform overview first: [docs/gstreamer-cross-platform-guide.md](docs/gstreamer-cross-platform-guide.md).

This project is wired to the GStreamer Android universal SDK. The SDK used here is:

```bash
/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2
```

If the SDK is missing, download and extract it:

```bash
mkdir -p /home/chrome-book/code/.cache/gstreamer-android
cd /home/chrome-book/code/.cache/gstreamer-android
curl -LO https://gstreamer.freedesktop.org/pkg/android/1.28.2/gstreamer-1.0-android-universal-1.28.2.tar.xz
echo 'ba9f1c6e10f0736fbf48cd741634fa90e49d50c961a4454230040b2517490d2a  gstreamer-1.0-android-universal-1.28.2.tar.xz' | sha256sum -c -
mkdir -p gstreamer-1.0-android-universal-1.28.2
tar -xJf gstreamer-1.0-android-universal-1.28.2.tar.xz -C gstreamer-1.0-android-universal-1.28.2
```

Build for the current x86_64 emulator:

```bash
cd /home/chrome-book/code/gst_audio_flutter
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter build apk --debug --target-platform android-x64
```

Install and launch:

```bash
/home/chrome-book/Android/Sdk/platform-tools/adb install -r build/app/outputs/flutter-apk/app-debug.apk
/home/chrome-book/Android/Sdk/platform-tools/adb shell am start -W -n com.example.gst_audio_flutter/.MainActivity
```

For an arm64 device, use `--target-platform android-arm64`. If building multiple ABIs, keep `GSTREAMER_ROOT_ANDROID` set; the local Cargokit patch maps Flutter ABI names to the SDK ABI folders.

Android playback test:

```bash
cd /home/chrome-book/code/gst_audio_flutter
mkdir -p test-assets
gst-launch-1.0 -q audiotestsrc num-buffers=80 wave=sine freq=440 ! audioconvert ! wavenc ! filesink location=test-assets/tone.wav
python3 -m http.server 8765 --bind 127.0.0.1 --directory test-assets
/home/chrome-book/Android/Sdk/platform-tools/adb reverse tcp:8765 tcp:8765
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter test integration_test/android_playback_test.dart -d emulator-5554
```

The Android build statically links GLib, GStreamer core libraries, and the selected static plugins into `libgst_audio_core.so`. `readelf -d` should only show Android system dependencies such as `libOpenSLES.so`, `liblog.so`, `libdl.so`, `libm.so`, and `libc.so`.

## Features

- Playback controls: play, pause, stop, previous, next
- Queue modes: sequential, shuffle, repeat one, repeat all
- Position: current time, duration, millisecond seek
- Audio controls: volume, mute, fade in, fade out
- Speed: 0.5x, 1x, 1.5x, 2x
- Output switching: default output or GStreamer `Audio/Sink` devices exposed by the host
- Local file paths and HTTP/HTTPS URLs are accepted by the queue

## Verification

Commands run successfully in this workspace:

```bash
cd rust && cargo check && cargo test
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter analyze
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter test
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter build linux
```

Playback smoke test:

```bash
cd rust
mkdir -p test-assets
gst-launch-1.0 -q audiotestsrc num-buffers=80 wave=sine freq=440 ! audioconvert ! wavenc ! filesink location=test-assets/tone.wav
cargo run --example smoke_play -- test-assets/tone.wav
```

The smoke test loads the generated WAV through the same Rust player API used by Flutter, starts playback, polls position, and exits without a GStreamer error.

Android commands run successfully in this workspace:

```bash
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 flutter build apk --debug --target-platform android-x64
adb install -r build/app/outputs/flutter-apk/app-debug.apk
adb shell am start -W -n com.example.gst_audio_flutter/.MainActivity
flutter test integration_test/android_playback_test.dart -d emulator-5554
```

## Notes

- This Linux build is dynamically linked against system Flutter, GLib, GStreamer, and GStreamer plugins.
- Android HTTP playback is enabled through the statically linked `soup` plugin and the app's `INTERNET` permission.
- Android uses `openslessink` by default. Speaker, headset, and Bluetooth routing is normally controlled by Android system audio policy; the Rust output-device API remains available, but selectable `Audio/Sink` devices may be limited or empty on Android.
- The Android plugin set intentionally avoids GStreamer Rust plugins such as `uriplaylistbin`, because the prebuilt SDK's Rust plugin static libraries can expose Rust allocator symbols that are incompatible with the app's Rust toolchain during static linking. The playlist/queue behavior in this app is implemented in Rust application code instead.
