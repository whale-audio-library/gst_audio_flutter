# GStreamer 跨平台开发指南

本文档是本项目的 GStreamer 总入口。项目根目录是：

```text
/home/chrome-book/code/gst_audio_flutter
```

项目目标是跨平台 Flutter + Rust 音频播放器：Flutter 负责 UI，Rust 通过 `flutter_rust_bridge` 暴露 API，播放队列和 GStreamer 控制逻辑都在 Rust 中完成。

## 平台状态

| 平台 | 当前状态 | GStreamer 接入方式 |
| --- | --- | --- |
| Linux | 已验证 | 通过系统 `pkg-config` 动态链接 GLib/GStreamer |
| Android | 已验证 | 使用 GStreamer Android SDK，静态链接核心库和插件到 `libgst_audio_core.so` |
| macOS | 工程骨架存在，未完成打包验证 | 需要安装 GStreamer SDK，并处理 `.dylib`/framework 运行时分发 |
| Windows | 已加入构建/打包适配，未在 Windows 机器实测 | 使用 MSVC 版 GStreamer SDK，Cargokit 注入 `pkg-config` 环境，CMake 复制 DLL/插件 |
| iOS | 工程骨架存在，未完成 GStreamer 接入验证 | 需要 iOS GStreamer SDK、静态插件注册和链接配置 |
| Web | 不适用 native GStreamer | Flutter Web 不能直接加载本项目的 native Rust + GStreamer FFI 后端 |

注意：跨平台工程不等于每个平台已经完成同等打包能力。当前真实跑通过的是 Linux 和 Android；Windows 已补构建和打包配置但还需要在 Windows 主机实测；macOS、iOS 还需要按本指南补齐平台 SDK、链接和运行时库分发。

## 代码结构

核心文件：

- `lib/main.dart`：Flutter 播放器 UI，调用 FRB 生成的 Dart API。
- `rust/src/api/player.rs`：播放器核心逻辑，包含播放/暂停、上一首/下一首、播放模式、seek、音量、静音、淡入淡出、倍速和输出设备接口。
- `rust/Cargo.toml`：Rust crate 依赖，使用 `gstreamer-rs`、`glib`、`gio`。
- `rust/build.rs`：Android 专用静态插件注册和链接逻辑。
- `rust_builder/`：Flutter FFI plugin 和 Cargokit 构建桥接。
- `docs/gstreamer-android-guide.md`：Android SDK、APK 静态链接、插件选择和常见问题。

## Rust 播放链路

播放链路：

```text
Flutter UI
  -> flutter_rust_bridge generated Dart API
  -> rust/src/api/player.rs
  -> gstreamer-rs
  -> GStreamer playbin + audio sink
  -> 平台音频输出
```

Rust 侧使用 GStreamer `playbin`，队列和播放模式由应用代码维护，不依赖 GStreamer playlist 插件。这样做可以避免 Android 静态链接 GStreamer Rust 插件时的 allocator 符号兼容问题。

输入支持：

- `http://...`
- `https://...`
- `file://...`
- 桌面平台本地文件路径

Android 上不能直接播放宿主机路径。测试 Android 本地资源时，可以使用 HTTP 服务、`adb reverse`，或者把文件放到设备可访问路径后传入 `file://` URI。

## Flutter API 使用

Flutter 侧导入 FRB 生成的 API：

```dart
import 'package:gst_audio_flutter/src/rust/api/player.dart' as player;
import 'package:gst_audio_flutter/src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  runApp(const AudioPlayerApp());
}
```

常用播放调用：

```dart
await player.setPlaylist(
  inputs: [
    'https://example.com/audio.mp3',
    '/home/user/Music/local-file.flac',
  ],
  startIndex: 0,
);

await player.play();
await player.pause();
await player.togglePlayPause();
await player.previous();
await player.next();
await player.playIndex(index: 2);
await player.seekMs(positionMs: 30000);
await player.setVolume(volume: 0.8);
await player.setMuted(muted: false);
await player.fadeIn(durationMs: 1500);
await player.fadeOut(durationMs: 1500);
await player.setSpeed(speed: 1.5);
await player.setShuffle(enabled: true);
await player.setRepeatMode(mode: player.RepeatMode.all);
final state = await player.getState();
final devices = await player.listOutputDevices();
```

`PlaybackState` 中包含当前队列、当前索引、播放状态、当前位置、总时长、音量、静音、倍速、随机播放、循环模式、输出设备和最后错误信息。

## Linux 开发

Linux 走系统动态链接。需要安装 GLib、GStreamer 和常用插件开发包，并确保 `pkg-config` 能找到：

```bash
pkg-config --modversion glib-2.0 gstreamer-1.0 gstreamer-audio-1.0
```

Ubuntu/Debian 常见依赖：

```bash
sudo apt-get update
sudo apt-get install -y \
  pkg-config \
  libglib2.0-dev \
  libgstreamer1.0-dev \
  libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad \
  gstreamer1.0-plugins-ugly \
  gstreamer1.0-libav
```

构建和运行：

```bash
cd /home/chrome-book/code/gst_audio_flutter
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter run -d linux
```

Release：

```bash
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter build linux
./build/linux/x64/release/bundle/gst_audio_flutter
```

Linux 产物不是全静态二进制。它依赖系统 Flutter、GLib、GStreamer 和插件运行时。发布时需要明确目标系统上安装了匹配的 GStreamer 包，或者额外制作包含运行时库和插件的发行包。

## Android 开发

Android 使用官方 GStreamer Android universal SDK。当前验证路径：

```text
/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2
```

构建 x86_64 模拟器 APK：

```bash
cd /home/chrome-book/code/gst_audio_flutter

PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter build apk --debug --target-platform android-x64
```

构建 arm64 真机 APK：

```bash
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter build apk --debug --target-platform android-arm64
```

Android 的详细 SDK 下载、静态插件列表、APK 依赖检查、HTTP 播放测试和常见错误见 [GStreamer Android 开发指南](gstreamer-android-guide.md)。

## macOS 开发

macOS 需要先安装 GStreamer SDK 或 Homebrew 包，让 Cargo 的 `pkg-config` 能找到 GStreamer：

```bash
pkg-config --modversion glib-2.0 gstreamer-1.0 gstreamer-audio-1.0
```

如果使用官方 `.pkg` 安装路径，通常需要设置：

```bash
export PKG_CONFIG_PATH=/Library/Frameworks/GStreamer.framework/Versions/1.0/lib/pkgconfig:$PKG_CONFIG_PATH
```

如果使用 Homebrew，根据机器架构设置到 Homebrew 的 `pkgconfig` 目录，例如 `/opt/homebrew/lib/pkgconfig` 或 `/usr/local/lib/pkgconfig`。

构建命令：

```bash
PATH=/path/to/flutter/bin:$PATH flutter build macos
```

当前项目的 macOS Flutter/Cargokit 骨架已存在，但还没有完成 GStreamer `.dylib` 或 framework 的 app bundle 分发配置。发布前需要检查：

- `build/macos/Build/Products/Release/gst_audio_flutter.app` 能否在无开发环境机器上启动。
- GStreamer 动态库和插件是否被打入 app bundle，或目标机器是否安装了同版本 GStreamer。
- `otool -L` 输出中是否存在只能在开发机上找到的绝对路径。

## Windows 开发

Windows 使用 MSVC 版 GStreamer SDK，并保持 Rust toolchain、Flutter Windows 构建和 GStreamer SDK 同为 MSVC ABI。当前项目已补 Windows 构建适配：

- `rust_builder/cargokit/build_tool/lib/src/windows_environment.dart`：给 Cargo 注入 `PKG_CONFIG_PATH`、`PKG_CONFIG_LIBDIR`、`PATH` 和 `PKG_CONFIG`。
- `rust_builder/cargokit/cmake/cargokit.cmake`：把 `GSTREAMER_ROOT_WINDOWS` 传入 Cargokit 构建环境。
- `windows/CMakeLists.txt`：把 GStreamer `bin/*.dll`、`lib/gstreamer-1.0/*.dll`、GIO modules 和 `gst-plugin-scanner.exe` 安装到 Flutter Windows bundle。
- `rust/src/api/player.rs`：Windows 启动 GStreamer 前设置应用目录内的 `PATH`、`GST_PLUGIN_PATH`、`GST_PLUGIN_PATH_1_0`、`GIO_EXTRA_MODULES` 和 `GST_PLUGIN_SCANNER`。

尚未在真实 Windows 主机上执行 `flutter build windows` 和播放测试，所以该平台状态是“已适配，待实测”。

常见环境变量：

```bat
set GSTREAMER_ROOT_WINDOWS=C:\gstreamer\1.0\msvc_x86_64
set GSTREAMER_1_0_ROOT_MSVC_X86_64=%GSTREAMER_ROOT_WINDOWS%
```

构建命令：

```bat
flutter build windows
```

如果从 Visual Studio 或手动 CMake 构建，可以通过 CMake cache 变量传入 SDK：

```bat
cmake -S windows -B build\windows\x64 -DGSTREAMER_ROOT_WINDOWS=C:\gstreamer\1.0\msvc_x86_64
```

构建时 Cargokit 会把下面路径交给 Cargo：

```text
PKG_CONFIG_PATH=<sdk>\lib\pkgconfig
PKG_CONFIG_LIBDIR=<sdk>\lib\pkgconfig
PATH=<sdk>\bin;%PATH%
PKG_CONFIG=<sdk>\bin\pkg-config.exe
```

打包后的 Windows bundle 预期结构：

```text
build\windows\x64\runner\Release\
  gst_audio_flutter.exe
  gst_audio_core.dll
  gstreamer-1.0-0.dll
  glib-2.0-0.dll
  ...
  lib\gstreamer-1.0\gstplayback.dll
  lib\gstreamer-1.0\gstcoreelements.dll
  lib\gio\modules\*.dll
  libexec\gstreamer-1.0\gst-plugin-scanner.exe
```

发布前需要在 Windows 主机检查：

- `gst_audio_core.dll` 能否找到 GStreamer 运行时 DLL。
- 插件目录是否被复制到 `lib\gstreamer-1.0`。
- `gst-plugin-scanner.exe` 是否被复制到 `libexec\gstreamer-1.0`。
- MP3/AAC/FLAC/OGG/HTTP/HTTPS 等实际业务格式能否播放。
- 在没有安装 GStreamer SDK 的干净 Windows 机器上能否启动。

Windows 常见问题：

- 如果 `glib-2.0.pc` 或 `gstreamer-1.0.pc` 找不到，检查 `GSTREAMER_ROOT_WINDOWS` 是否指向 SDK 根目录，而不是 `bin` 目录。
- 如果启动时报缺 DLL，检查 release 目录旁边是否有 GStreamer `bin` 里的 DLL。
- 如果 `playbin` 创建失败或格式无法播放，检查 `lib\gstreamer-1.0` 是否有插件 DLL，并打开 `GST_DEBUG=2` 看插件加载日志。
- 如果 HTTPS 失败，检查 GIO TLS module 是否复制到了 `lib\gio\modules`。

## iOS 开发

iOS 不能复用 Android 的 `openslessink` 和 Android 静态插件注册逻辑。需要单独接入 iOS GStreamer SDK，并补齐：

- per-architecture 的 GStreamer SDK 路径。
- iOS 可用的音频 sink。
- 静态插件声明和注册。
- Podspec 中 GStreamer 静态库、系统 framework 和 linker flag。
- 真机和模拟器的架构切片验证。

当前 iOS Flutter/Podspec 骨架存在，但 GStreamer iOS 播放后端尚未完成验证。不要把 Android APK 的静态链接结果直接理解为 iOS 已可发布。

## Cerbero 环境

如果使用 Cerbero 自己构建 GStreamer，不要求项目代码感知 Cerbero；关键是把每个平台的 GStreamer install prefix 暴露给 Cargo 和链接器。

桌面平台需要保证：

```bash
pkg-config --cflags --libs gstreamer-1.0 glib-2.0
```

能输出 Cerbero prefix 下的 include 和 lib 路径。

Android 当前 Cargokit 逻辑默认接收官方 universal SDK 根目录：

```bash
GSTREAMER_ROOT_ANDROID=/path/to/gstreamer-1.0-android-universal-1.28.2
```

如果 Cerbero 输出不是 `armv7/ arm64/ x86/ x86_64/` 这种布局，可以按 ABI 直接传：

```bash
GSTREAMER_ANDROID_PREFIX=/path/to/cerbero/android/<abi-prefix>
```

或者修改 `rust_builder/cargokit/build_tool/lib/src/android_environment.dart` 中的 ABI 到 prefix 映射。

## HTTP/HTTPS 边下边播

桌面平台需要系统安装 HTTP/HTTPS 相关插件。Android 需要：

- `android/app/src/main/AndroidManifest.xml` 中有 `INTERNET` 权限。
- `rust/build.rs` 静态插件列表包含 `soup`。
- HTTPS 需要 GIO TLS 模块，例如当前 Android 配置中的 `openssl`。

HTTP 测试方法：

```bash
cd /home/chrome-book/code/gst_audio_flutter
mkdir -p test-assets
gst-launch-1.0 -q audiotestsrc num-buffers=80 wave=sine freq=440 ! audioconvert ! wavenc ! filesink location=test-assets/tone.wav
python3 -m http.server 8765 --bind 127.0.0.1 --directory test-assets
```

Android 模拟器再加：

```bash
/home/chrome-book/Android/Sdk/platform-tools/adb reverse tcp:8765 tcp:8765
```

然后队列中使用：

```text
http://127.0.0.1:8765/tone.wav
```

## 插件策略

桌面平台通常动态扫描系统插件，因此开发时更方便，但发布时要考虑目标机器插件是否齐全。

Android 使用静态插件，插件列表在 `rust/build.rs`：

```rust
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
```

新增格式或协议时，先确认目标平台 SDK 里存在对应插件，再更新插件列表或发布包内容。Android 添加静态插件后必须重新构建 APK 并做真机/模拟器播放测试。

## 常见问题

### 找不到 `gstreamer-1.0.pc` 或 `glib-2.0.pc`

这是 `pkg-config` 路径问题，不是 Rust 代码问题。先运行：

```bash
pkg-config --modversion glib-2.0 gstreamer-1.0
```

如果失败，进入对应 GStreamer/Cerbero 环境，或者设置 `PKG_CONFIG_PATH`、`PKG_CONFIG_LIBDIR`。Android 需要设置 `GSTREAMER_ROOT_ANDROID` 或 `GSTREAMER_ANDROID_PREFIX`。

### `cargo check` 通过但应用运行失败

`cargo check` 不能替代真实平台链接和运行。特别是 Android 静态链接，必须至少跑：

```bash
GSTREAMER_ROOT_ANDROID=/path/to/sdk flutter build apk --debug --target-platform android-x64
```

并安装启动或执行 integration test。

### 本地文件在 Android 播不了

Flutter/Rust 收到的宿主机路径不是 Android 设备路径。Android 用 HTTP URL、应用沙盒文件路径、MediaStore 可访问文件，或先把文件推到设备上。

### Android APK 里是否需要打包 `libOpenSLES.so`、`liblog.so`

不需要。这些是 Android 系统库，由系统提供。APK 需要包含的是本项目生成的 `libgst_audio_core.so`；GStreamer/GLib/插件当前静态链接进这个 `.so`。

### 输出设备切换在 Android 上为空

Android 的耳机、蓝牙、扬声器通常由系统音频策略路由。项目保留 `listOutputDevices` 和 `setOutputDevice` API，但 Android 上不一定能像 Linux PulseAudio/PipeWire 那样枚举完整设备列表。

### Web 为什么不支持

本项目播放后端依赖 native Rust FFI 和 GStreamer C 库。Flutter Web 不能直接加载这个后端。Web 需要单独实现浏览器音频后端。

## 修改后建议检查

通用检查：

```bash
cd /home/chrome-book/code/gst_audio_flutter
cargo check --manifest-path rust/Cargo.toml
cargo test --manifest-path rust/Cargo.toml
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter analyze
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter test
```

Linux 运行：

```bash
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter run -d linux
```

Android 构建：

```bash
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter build apk --debug --target-platform android-x64
```

Android 播放测试：

```bash
python3 -m http.server 8765 --bind 127.0.0.1 --directory test-assets
/home/chrome-book/Android/Sdk/platform-tools/adb reverse tcp:8765 tcp:8765
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter test integration_test/android_playback_test.dart -d emulator-5554
```
