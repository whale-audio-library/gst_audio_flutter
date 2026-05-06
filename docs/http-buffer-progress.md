# HTTP Buffer Progress

This document records how HTTP buffering progress is defined, implemented, and verified in this project.

## Problem

GStreamer `Buffering` messages report the internal playback queue state. That value is useful for pause/resume decisions, but it is not the same as "how much of the HTTP media file has been downloaded".

For the Flutter UI, `bufferingPercent` must represent real HTTP media download progress:

```text
downloaded media bytes / total media bytes
```

The value should move monotonically from 0 to 100 for normal HTTP downloads. It should not jump to 100 just because the internal queue is full enough to continue playback.

## Implementation

The Rust player keeps two separate concepts:

- GStreamer buffering state: used only to decide whether playback should pause or resume while the pipeline waits for data.
- HTTP download progress: exposed to Flutter as `PlayerState.bufferingPercent`.

The implementation is in `rust/src/api/player.rs`.

Current behavior:

- `playbin` enables the `download` and `buffering` flags.
- `source-setup` installs a pad probe on the HTTP source output pad.
- The pad probe counts actual buffer bytes that pass from the HTTP source into the pipeline.
- HTTP response headers are read from GStreamer element messages.
- `Content-Range` total length is preferred when present.
- `Content-Length` is used as a fallback.
- The exported percentage is clamped to `0..100` and kept monotonic.
- Non-HTTP inputs report `100`, because there is no remote download progress to expose.

On Android and iOS, the player also points GStreamer download-buffer temporary files at the platform temp directory. This avoids invalid cache paths such as `/data/.cache/<unknown>-XXXXXX` on Android.

## Flutter UI

The Flutter progress bar reads `state.bufferingPercent`.

Labels are intentionally separate from playback state:

- `Downloading`: HTTP bytes are still being fetched.
- `Buffering`: playback is currently waiting for enough data.
- `Buffered`: download progress reached 100%.

This prevents the UI from showing "Buffered" while the media is still downloading.

## Test Assets

Generate the expanded local test assets with:

```bash
python3 tool/generate_test_audio_assets.py --output-dir test-assets/generated
```

The generated set includes longer and larger WAV files used to make partial HTTP progress observable:

- `test-assets/generated/long-30s.wav`
- `test-assets/generated/large-60s-stereo.wav`

The test server supports slow and interrupted responses:

```bash
python3 tool/audio_test_server.py --port 8765 --directory test-assets
```

Example slow URL:

```text
http://127.0.0.1:8765/slow/generated/large-60s-stereo.wav?chunk=4096&delay=0.02
```

## Integration Tests

The focused HTTP progress test is:

```text
integration_test/http_buffer_progress_test.dart
```

It plays a large WAV through the slow HTTP endpoint and asserts that `bufferingPercent` reports partial progress strictly between `0` and `100` before the download completes.

The broader matrix test is:

```text
integration_test/audio_matrix_test.dart
```

It covers multiple HTTP behaviors and audio formats, including:

- WAV over normal HTTP.
- Slow large HTTP response.
- Interrupted HTTP transfer recovery.
- Rapid playlist switches.
- FLAC.
- OGG Vorbis.
- OGG Opus.
- Longer generated WAV.

## Linux Desktop Verification

Run the server first:

```bash
python3 tool/generate_test_audio_assets.py --output-dir test-assets/generated
python3 tool/audio_test_server.py --port 8765 --directory test-assets
```

Then run:

```bash
flutter test integration_test/http_buffer_progress_test.dart -d linux
flutter test integration_test/audio_matrix_test.dart -d linux
```

## Android Verification

Start the same local server on the host, then expose it to the emulator:

```bash
adb reverse tcp:8765 tcp:8765
```

Run:

```bash
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
  flutter test integration_test/http_buffer_progress_test.dart -d emulator-5554

GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
  flutter test integration_test/audio_matrix_test.dart -d emulator-5554
```

## iOS CI Verification

iOS is tested through GitHub Actions CI.

The iOS workflow runs:

```text
integration_test/ios_playback_test.dart
integration_test/http_buffer_progress_test.dart
integration_test/audio_matrix_test.dart
```

The fix was verified by GitHub Actions run:

```text
https://github.com/whale-audio-library/gst_audio_flutter/actions/runs/25406858870
```

That run used commit:

```text
81a2dff Fix HTTP buffer progress accuracy
```

## Troubleshooting

If `bufferingPercent` reaches `100` too early, check that the UI is not reading GStreamer `Buffering` message percent directly. That value is queue fullness, not HTTP file download progress.

If `bufferingPercent` stays `0`, check:

- The stream is HTTP or HTTPS.
- The server returns `Content-Length` or `Content-Range`.
- The GStreamer HTTP source emits headers.
- The source pad probe is installed on the actual HTTP source element.

If Android fails with a download-buffer temporary file error, confirm that the runtime temp environment and `downloadbuffer.temp-template` are configured before playback starts.
