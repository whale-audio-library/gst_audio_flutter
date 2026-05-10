# Testing And CI Guide

This document describes the current test topology for `gst_audio_flutter`, the GitHub workflows that run it, and the main failure modes that were seen while bringing iOS native audio tests online.

## Scope

This project now has three distinct test layers:

1. Local/unit/integration checks in the main `CI` workflow
2. Android device tests, including Patrol native audio coverage
3. iOS simulator tests, split into package/test/debug workflows

The split is intentional. iOS packaging, iOS test execution, and SSH-based remote debugging have different failure modes and should not be treated as one job.

## Current GitHub Workflows

### `.github/workflows/ci.yml`

General Linux/Android workflow.

Primary jobs:

- `Flutter and Rust checks`
  - `flutter analyze`
  - `flutter test`
  - `cargo test`
  - Linux desktop build
  - Linux audio visualization integration test
- `Android APK build`
  - Release APK build
- `Android Patrol native audio test`
  - Emulator boot
  - Local HTTP fixture server
  - `patrol test --target patrol_test/native_audio_controls_test.dart`

This workflow is the main entry point for Android CI.

### `.github/workflows/ios-build.yml`

Dedicated iOS packaging workflow.

Purpose:

- Build unsigned iOS artifacts
- Package an unsigned IPA

Key output artifact:

- `gst-audio-flutter-ios-unsigned`

This workflow is intentionally separate from iOS tests.

### `.github/workflows/ios-tests.yml`

Dedicated iOS simulator test workflow.

Purpose:

- Create a fresh simulator
- Run native XCTest checks
- Run Patrol native audio test
- Run iOS Flutter integration tests

Key behavior:

- Creates a fresh `iPhone 16 Pro` simulator each run
- Boots and validates the simulator before tests
- Runs `RunnerTests`
- Runs Patrol using `patrol build ios` plus `xcodebuild test-without-building`
- Runs the legacy Flutter integration tests after Patrol
- Uploads `ios-integration-logs`

Optional input:

- `debug_tmate`
  - Enables SSH remote debugging
  - Publishes an SSH command into the PR comments

### `.github/workflows/ios-debug.yml`

Dedicated iOS remote debug workflow.

Purpose:

- Provide an isolated SSH-debuggable iOS runner
- Run exactly one iOS test flow at a time

Supported modes:

- `patrol_native_audio`
- `ios_playback`
- `ios_http_buffer`
- `ios_audio_visualization`
- `ios_audio_matrix`

This workflow is the preferred path when reproducing an iOS CI-only issue.

## Local Test Entry Points

### Android

#### Local Flutter / Rust checks

```bash
flutter analyze
flutter test
cargo test --manifest-path rust/Cargo.toml
```

#### Generate audio fixtures

```bash
python3 tool/generate_test_audio_assets.py --output-dir test-assets/generated
```

#### Run local HTTP fixture server

```bash
python3 tool/audio_test_server.py --port 8765 --directory test-assets
```

#### Android Patrol native audio test

This is the closest local equivalent of Android native audio E2E coverage:

```bash
patrol test \
  --target patrol_test/native_audio_controls_test.dart \
  --device <adb-device-id> \
  --no-label \
  --show-flutter-logs
```

The Android test covers:

- audio session activation
- playback continuity
- foreground notification presence
- media button dispatch
- next / previous / play-pause controls

### iOS

#### Native XCTest

Native iOS checks live in:

- `ios/RunnerTests/RunnerTests.swift`

Current native assertions:

- `UIBackgroundModes` includes `audio`
- `AVAudioSession` can be configured with `.playback`

#### Patrol native audio test

Patrol iOS test file:

- `patrol_test/native_audio_controls_test.dart`

Current iOS-specific behavior:

- uses local asset playback instead of HTTP for Patrol
- validates audio session config
- validates playback continues after `pressHome()`

#### Legacy iOS playback integration test

Flutter integration file:

- `integration_test/ios_playback_test.dart`

This test covers:

- bundled asset playback
- HTTP playback
- visualization frame activity
- pause / resume / next / previous

## Native Wiring Required For Tests

### Android

Key project wiring:

- `MainActivity` extends `AudioServiceActivity`
- debug-only `MethodChannel`:
  - `com.example.gst_audio_flutter/native_audio_test`
- debug methods:
  - `dispatchMediaKey`
  - `isMusicActive`
  - `activeNotifications`
  - `sdkInt`

Required Android permissions include:

- `INTERNET`
- `WAKE_LOCK`
- `FOREGROUND_SERVICE`
- `FOREGROUND_SERVICE_MEDIA_PLAYBACK`
- `POST_NOTIFICATIONS`

### iOS

Key project wiring:

- `Info.plist` includes `UIBackgroundModes = audio`
- `AppDelegate.swift` configures `AVAudioSession` as `.playback`
- `RunnerUITests` target is linked to Flutter pods
- `RunnerUITests.m` uses Patrol iOS runner macro

## Verification Script

Use:

```bash
python3 tool/verify_native_audio_patrol_setup.py
```

This checks:

- Android permissions
- AudioService / MediaButtonReceiver declarations
- Android Patrol runner setup
- iOS background audio declaration
- iOS audio session setup
- iOS RunnerUITests / Pod wiring
- `pubspec.yaml` package declarations

## Important Test Assets

### Stable assets

- `test-assets/tone.wav`
  - short local asset
- `test-assets/long-30s.wav`
  - long local asset used to stabilize iOS Patrol background playback checks

### Generated assets

- `test-assets/generated/long-30s.wav`
- `test-assets/generated/large-60s-stereo.wav`

Generated assets are not committed. Generate them with:

```bash
python3 tool/generate_test_audio_assets.py --output-dir test-assets/generated
```

## iOS CI Problems Encountered And Fixes

This section documents the actual failure chain encountered while bringing iOS native audio CI online.

### 1. Reused simulator boot instability

Symptom:

- `simctl boot` on a previously existing simulator would hang
- Flutter then failed to find an iOS device

Fix:

- Always create a fresh simulator in CI
- Boot that simulator by returned device ID
- Delete it at the end of the run

### 2. Patrol simulator selection instability

Symptom:

- `patrol` / `xcodebuild` would run against a cloned simulator unexpectedly
- Some runs died with `RunnerUITests.xctrunner` server failures

Fix:

- Build with Patrol first
- Then run:
  - `xcodebuild test-without-building`
  - destination pinned by simulator ID

### 3. Flutter device detection instability after Patrol

Symptom:

- Flutter could stop seeing the simulator after Patrol/XCTest activity

Fix:

- Reboot / reopen simulator visibility path
- Re-run `flutter devices`
- Wait until the created simulator ID is present before continuing

### 4. Rust / GStreamer callback panic

Symptom:

- `gst-audio-player` thread could panic during setup callbacks
- iOS runner aborted with `SIGABRT`

Fix:

- Wrap risky setup callback logic with `catch_unwind`
- remove unstable callback-based download buffer hook from the iOS path

### 5. iOS simulator crash in HTTP/GStreamer path during Patrol

Symptom:

- Patrol native audio test crashed in simulator while using HTTP source playback
- crash surfaced in GStreamer typefind / decodebin path

Fix:

- Patrol iOS path uses local asset playback
- Android Patrol path keeps HTTP playback

### 6. Short asset made background-playback assertion unstable

Symptom:

- iOS Patrol used `test-assets/tone.wav`
- the file is only about 1.86 seconds long
- after `pressHome()`, playback could already be finished, making the assertion flaky

Fix:

- add `test-assets/long-30s.wav`
- use it in the iOS Patrol path
- assert that playback position continues to advance after backgrounding

### 7. iOS playback timeout caused by expensive rebuilds

Symptom:

- `ios_playback_test.dart` could hit workflow timeout
- `flutter test -d <simulator>` triggers a fresh Xcode build
- Rust / GStreamer pods were rebuilt during that path

Fix:

- increase `ios-tests.yml` integration step timeout from `60` to `120` minutes

### 8. Over-aggressive arm64-only rollout broke iOS linking

Symptom:

- applying `ARCHS=arm64 ONLY_ACTIVE_ARCH=YES EXCLUDED_ARCHS=x86_64` too broadly
- broke simulator linking for:
  - `GCDAsyncSocket`
  - `CocoaAsyncSocket`
- errors included:
  - `Undefined symbol: _GCDAsyncSocketManuallyEvaluateTrust`
  - `Undefined symbol: _OBJC_CLASS_$_GCDAsyncSocket`

Fix:

- keep Patrol iOS builds universal
- keep Flutter iOS test builds universal
- do not force arm64-only on paths that still need universal simulator pod artifacts

## Tmate / SSH Debugging

When `debug_tmate` is enabled in `ios-tests.yml` or `ios-debug.yml`:

- the workflow publishes an SSH command into the PR comments
- format:

```bash
ssh <session>@<region>.tmate.io
```

Use this when:

- the failure only happens on GitHub macOS runners
- you need to inspect:
  - `flutter devices`
  - `xcrun simctl list devices`
  - current logs in `$RUNNER_TEMP`
  - live `xcodebuild` or `flutter test` processes

Notes:

- `tmate` sessions can keep the workflow alive after the main test step finishes
- a run may show `cancelled` or remain `in_progress` while waiting for the SSH session to end
- that does not necessarily mean the test body failed

## Current Known Good Runs

These runs are useful reference points:

- `25588438435`
  - `iOS Package`
  - success
- `25588438459`
  - PR `iOS Tests`
  - success
- `25588449058`
  - manual `iOS Tests` with `tmate`
  - success
- `25582072785`
  - `iOS Debug`
  - `Run selected iOS debug test` succeeded

## Recommended Operational Flow

### For Android

1. Run local Flutter/Rust checks
2. Generate fixtures
3. Start local audio test server
4. Run Patrol against a connected emulator/device

### For iOS

1. Use `iOS Package` for packaging validation
2. Use `iOS Tests` for simulator-native coverage
3. Use `iOS Debug` with `debug_tmate=true` when the failure is CI-only

### For CI-only iOS regressions

1. Start `iOS Debug`
2. Choose the smallest reproduction mode
3. Connect over `tmate`
4. Re-run the failing command manually on the runner
5. Check logs under:
   - `$RUNNER_TEMP/ios-debug-logs`
   - `$RUNNER_TEMP/ios-integration-logs`

## Maintenance Notes

- Treat simulator lifecycle as part of the test harness, not an implementation detail.
- Do not reuse long-lived simulator IDs in CI.
- Do not assume Patrol build settings and Flutter integration build settings can share the same architecture overrides.
- Keep SSH debug and production CI behavior separate whenever possible.
