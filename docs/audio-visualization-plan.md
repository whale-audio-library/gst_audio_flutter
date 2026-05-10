# 音频可视化实现记录

本文档记录当前音频可视化实现、验证步骤和后续待办。实现已经接入 Flutter UI、Rust 播放器和 GStreamer sink 链路。

项目根目录：

```text
/home/chrome-book/code/gst_audio_flutter
```

## 当前实现

播放链路仍由 Flutter -> FRB -> Rust -> GStreamer `playbin` 负责。可视化接在解码后的音频 sink bin 内：

```text
playbin audio output
  -> volume
  -> audioconvert
  -> audioresample
  -> level
  -> spectrum
  -> tee
      -> queue
      -> platform sink
      -> queue(leaky)
      -> audioconvert
      -> audioresample
      -> capsfilter(audio/x-raw, F32LE, mono)
      -> appsink
```

平台 sink：

- Linux/Desktop：默认 `autoaudiosink`；测试可用 `GST_AUDIO_FLUTTER_AUDIO_SINK=fakesink`。
- Android：`openslessink`。
- iOS：`osxaudiosink`。

`level` 和 `spectrum` 都插在 `volume` 后，对最终输出音频做分析。因此音量、静音和淡入淡出会反映到可视化里；静音时可视化会归零。

## Rust API

新增 `VisualizationFrame`：

```text
timestamp_ms: i64
pcm: Vec<f64>
magnitude: Vec<f64>
rms: Vec<f64>
peak: Vec<f64>
decay: Vec<f64>
rms_normalized: f64
peak_normalized: f64
beat: bool
beat_strength: f64
waveform: Vec<f64>
normalized: Vec<f64>
is_active: bool
```

新增 public API：

```text
get_visualization_frame() -> Result<VisualizationFrame, String>
```

实现细节：

- `rust/src/api/player.rs` 解析 GStreamer `spectrum` element message。
- `magnitude` 从 `GValueArray`/`GstList` 读取，表示各频段的 dB 能量。
- Rust 侧把每个频段的 dB 归一化到 `0.0..1.0`，写入 `normalized`。
- `normalized` 会做 attack/release smoothing，避免频谱柱抖动过硬。
- `rust/src/api/player.rs` 同时解析 GStreamer `level` element message。
- `rms`、`peak`、`decay` 由 GStreamer `level` 直接产出。
- `rms_normalized` 和 `peak_normalized` 是 `level` dB 数据归一化后的单值强度。
- `beat` 和 `beat_strength` 是基于滚动 RMS 能量的轻量 beat 检测；纯音或平稳音频不保证触发 beat。
- `appsink` 旁路读取解码后的 F32LE mono PCM，并降采样成固定点数。
- `pcm` 和 `waveform` 都是当前最新 PCM 窗口的降采样波形，范围是 `-1.0..1.0`。
- `appsink` 分支使用 leaky queue 和 `max-buffers=1`，可视化处理慢时丢弃旧 buffer，不阻塞播放分支。
- 当前范围是 `-60 dB..0 dB`，低于 `-55 dB` 视作静音。
- 当前频谱带数是 48，Flutter 固定绘制 48 根柱，一根柱对应一个真实频段，不做插值扩展或压缩。
- 播放线程只保存最新帧，不积压历史帧。
- 停止、暂停、错误会清理或衰减可视化帧。
- `poll_bus` 同时保留 HTTP `http-headers`/download total 处理以及 `level`/`spectrum` 消息处理。

## Flutter UI

`lib/main.dart` 已新增：

- 独立 100ms 可视化轮询 timer。
- `_AudioVisualization` widget。
- `_AudioVisualizationPainter`，使用 `CustomPaint` 绘制固定 48 根镜像柱状图。
- `ValueKey('audio-visualization')`，供集成测试定位。

现有 HTTP buffer UI 未改 key：

```text
ValueKey('http-buffer-progress')
```

可视化位于当前曲目信息和 seek slider 之间，避免和 HTTP buffer progress 互相遮挡。

## 平台插件

Android 静态插件列表已加入：

```text
spectrum
level
app
```

iOS 静态注册已加入：

```text
gst_plugin_spectrum_register
gst_plugin_level_register
gst_plugin_app_register
```

HTTP 边下边播时，appsink 只能拿到已经下载、解码并流经播放链路的 PCM，不能提前得到未下载部分。

## 格式支持

可视化在解码后工作，不直接解析音频容器或编码。因此：

```text
只要当前 GStreamer 播放链路能成功解码并播放，该音频就能产生可视化数据。
```

当前测试资产覆盖：

- WAV：`test-assets/tone.wav`
- FLAC：`test-assets/formats/tone.flac`
- OGG/Vorbis：`test-assets/formats/tone-vorbis.ogg`
- OGG/Opus：`test-assets/formats/tone-opus.ogg`

## 测试覆盖

新增/扩展测试：

- `integration_test/audio_visualization_test.dart`
  - Linux/Desktop 可视化集成测试。
  - 断言 UI 有 `audio-visualization`。
  - 播放 `asset:///test-assets/tone.wav` 后断言 `VisualizationFrame.isActive == true`、`magnitude` 是多频段数组，`normalized` 有非零值和频段差异，`rms/peak/decay` 非空，`pcm/waveform` 为 `-1..1` 范围内的 PCM 波形。
  - 停止后断言可视化回到非活跃。

- `integration_test/android_playback_test.dart`
  - HTTP WAV 播放成功后增加可视化帧断言。

- `integration_test/ios_playback_test.dart`
  - bundled WAV 和 HTTP WAV 播放成功后增加可视化帧断言。

- `integration_test/simple_test.dart`
  - 检查可视化 widget 在 shell UI 中存在。

GitHub Actions：

- `.github/workflows/ci.yml` 的 Linux checks 已加入：
  - `flutter build linux`
  - `xvfb-run -a flutter test integration_test/audio_visualization_test.dart -d linux`
  - 环境变量 `GST_AUDIO_FLUTTER_AUDIO_SINK=fakesink`

- `.github/workflows/ios-build.yml` 继续运行 `ios_playback_test.dart`，其中已包含可视化断言。

## 本地验证顺序

### 1. 静态检查

```bash
cd /home/chrome-book/code/gst_audio_flutter
cargo check --manifest-path rust/Cargo.toml
cargo test --manifest-path rust/Cargo.toml
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter analyze
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter test
```

### 2. Linux Desktop

```bash
cd /home/chrome-book/code/gst_audio_flutter
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter build linux
GST_AUDIO_FLUTTER_AUDIO_SINK=fakesink \
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
flutter test integration_test/audio_visualization_test.dart -d linux
```

如果有可用桌面音频设备，也可以不设置 `GST_AUDIO_FLUTTER_AUDIO_SINK`，使用默认 `autoaudiosink`。

### 3. Android

```bash
cd /home/chrome-book/code/gst_audio_flutter
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter build apk --debug --target-platform android-x64
```

HTTP 测试：

```bash
python3 tool/audio_test_server.py --port 8765 --directory test-assets
/home/chrome-book/Android/Sdk/platform-tools/adb reverse tcp:8765 tcp:8765

PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter test integration_test/android_playback_test.dart -d emulator-5554
```

回归 HTTP buffer 和格式矩阵：

```bash
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter test integration_test/http_buffer_progress_test.dart -d emulator-5554

PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter test integration_test/audio_matrix_test.dart -d emulator-5554
```

动态依赖检查：

```bash
/home/chrome-book/Android/Sdk/ndk/28.2.13676358/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf \
  -d rust/target/x86_64-linux-android/debug/libgst_audio_core.so | grep NEEDED
```

预期仍只包含 Android 系统库，不应出现未打包的 GStreamer 动态库依赖。

### 4. iOS

iOS 需要 macOS + Xcode 环境。Linux 工作站不能直接完成 iOS 构建或 simulator 测试。

在 macOS 上：

```bash
export GSTREAMER_IOS_XCFRAMEWORK=/path/to/GStreamer.xcframework
PATH=/path/to/flutter/bin:$PATH flutter build ios --simulator
PATH=/path/to/flutter/bin:$PATH flutter test integration_test/ios_playback_test.dart -d <simulator-id>
```

GitHub Actions 的 `.github/workflows/ios-build.yml` 会构建 unsigned iOS 包，并在 simulator 上运行 iOS 集成测试。

## 验收标准

- 播放 `test-assets/tone.wav` 时 Flutter 可视化有动态柱状图。
- 停止后可视化回到非活跃。
- 静音、淡出后可视化随音频链路输出归零。
- HTTP buffer progress 仍显示并通过原测试。
- Android APK 构建通过，`spectrum`/`level`/`app` 静态插件成功链接。
- iOS 构建阶段能找到 `gst_plugin_spectrum_register`、`gst_plugin_level_register`、`gst_plugin_app_register`。
- Linux/Android/iOS 集成测试按顺序通过。

## 后续待办

- [x] 设计 `VisualizationFrame` Rust/Dart API。
- [x] 在 Rust sink bin 中加入 `spectrum`。
- [x] 解析 GStreamer `spectrum` bus message。
- [x] 在 Rust sink bin 中加入 `tee + queue + appsink` PCM 旁路。
- [x] 读取并降采样 F32 PCM 波形。
- [x] 保留 HTTP download/header 的 `Element` bus message 处理。
- [x] 添加 Android `spectrum`/`level`/`app` 静态插件链接。
- [x] 添加 iOS `spectrum`/`level`/`app` 静态插件注册。
- [x] 运行 FRB codegen。
- [x] 在 Flutter 中添加 `CustomPaint` 可视化组件。
- [x] 添加 Linux Desktop visualization integration test。
- [x] 扩展 Android integration test。
- [x] 扩展 iOS integration test。
- [x] 在本机完成 Android emulator 测试。
- [ ] 通过 GitHub Actions 或 macOS 环境完成 iOS 测试。
- [ ] 视产品需要在 UI 中提供频谱/波形切换。
