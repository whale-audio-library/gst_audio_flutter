# GStreamer Android 开发指南

本文档只说明 Android 平台的 GStreamer 接入、静态插件、APK 打包和测试问题。项目本身是跨平台 Flutter + Rust 工程，跨平台总入口见 [GStreamer 跨平台开发指南](gstreamer-cross-platform-guide.md)。

## 架构

播放链路：

```text
Flutter UI
  -> flutter_rust_bridge generated API
  -> rust/src/api/player.rs
  -> gstreamer-rs
  -> GStreamer playbin + audio sink
  -> Android OpenSL ES output
```

Flutter 只负责界面和调用 Rust API。播放队列、播放/暂停、上一首/下一首、循环模式、随机播放、seek、音量、静音、淡入淡出、倍速等逻辑都在 Rust 中处理。

核心文件：

- `rust/src/api/player.rs`：播放器业务逻辑，使用 `gstreamer-rs` 控制 `playbin`。
- `rust/build.rs`：Android 静态 GStreamer 插件注册和链接。
- `rust_builder/cargokit/build_tool/lib/src/android_environment.dart`：为 Android Rust 交叉编译配置 `pkg-config`、NDK、GStreamer SDK 路径。
- `rust_builder/cargokit/gradle/plugin.gradle`：从 Gradle 把 `GSTREAMER_ROOT_ANDROID` 传给 Cargokit。
- `integration_test/android_playback_test.dart`：Android 真机/模拟器 HTTP 播放验证。

## GStreamer SDK

Android 使用官方 GStreamer Android universal SDK。当前验证版本：

```text
GStreamer Android SDK: 1.28.2
SDK root: /home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2
```

下载和解压：

```bash
mkdir -p /home/chrome-book/code/.cache/gstreamer-android
cd /home/chrome-book/code/.cache/gstreamer-android

curl -LO https://gstreamer.freedesktop.org/pkg/android/1.28.2/gstreamer-1.0-android-universal-1.28.2.tar.xz
echo 'ba9f1c6e10f0736fbf48cd741634fa90e49d50c961a4454230040b2517490d2a  gstreamer-1.0-android-universal-1.28.2.tar.xz' | sha256sum -c -

mkdir -p gstreamer-1.0-android-universal-1.28.2
tar -xJf gstreamer-1.0-android-universal-1.28.2.tar.xz -C gstreamer-1.0-android-universal-1.28.2
```

不要使用 `--strip-components=1` 解压。官方包顶层包含四个 ABI 目录：

```text
armv7/
arm64/
x86/
x86_64/
```

如果错误地 strip 解压，会把不同 ABI 的库混到一起，可能出现 `cargo check` 看似通过、真实链接或运行失败的问题。

## Android 构建

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

安装和启动：

```bash
/home/chrome-book/Android/Sdk/platform-tools/adb install -r build/app/outputs/flutter-apk/app-debug.apk
/home/chrome-book/Android/Sdk/platform-tools/adb shell am start -W -n com.example.gst_audio_flutter/.MainActivity
```

## APK 中包含什么

Android APK 中不单独打包 `libgstreamer-1.0.so`、`libglib-2.0.so`、`libgstplayback.so` 等动态库。

当前做法是把 GStreamer、GLib 和选定插件的 `.a` 静态库链接进 Rust 产物：

```text
lib/<abi>/libgst_audio_core.so
```

用下面命令查看运行时依赖：

```bash
/home/chrome-book/Android/Sdk/ndk/28.2.13676358/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf \
  -d rust/target/x86_64-linux-android/debug/libgst_audio_core.so | grep NEEDED
```

预期只剩 Android 系统库：

```text
libOpenSLES.so
liblog.so
libdl.so
libm.so
libc.so
```

这些库由 Android 系统提供，不需要打进 APK。

## GStreamer 插件

Android 不能像 Linux 桌面版那样依赖系统插件扫描。本项目在 `rust/build.rs` 里生成一个 C 文件，显式声明并注册静态插件：

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

对应插件库在 SDK 中：

```text
<sdk>/<abi>/lib/gstreamer-1.0/libgst<plugin>.a
```

如果要支持新格式或协议，先确认 SDK 中有对应插件，再加到 `ANDROID_PLUGINS`。例如：

```bash
ls $GSTREAMER_ROOT_ANDROID/x86_64/lib/gstreamer-1.0/libgst*.a
```

添加插件后重新构建 APK，并跑 Android 播放测试。

## 播放 API 使用

Flutter 调用的是 FRB 生成的 Dart API：

```dart
import 'package:gst_audio_flutter/src/rust/api/player.dart' as player;

await player.setPlaylist(
  inputs: ['http://127.0.0.1:8765/tone.wav'],
  startIndex: 0,
);
await player.play();
await player.pause();
await player.seekMs(positionMs: 30000);
await player.setVolume(volume: 0.8);
await player.setMuted(muted: false);
await player.setSpeed(speed: 1.5);
await player.next();
```

队列输入支持：

- `http://...`
- `https://...`
- `file://...`
- 桌面平台本地文件路径

Android 上普通宿主机路径不能直接播放，因为它不是设备文件路径。测试时可以用 HTTP 或把文件放到 Android 可访问位置。

## HTTP 边下边播

HTTP/HTTPS 播放依赖：

- AndroidManifest 中的 `INTERNET` 权限。
- `rust/build.rs` 中的 `soup` 插件。
- GIO TLS 模块 `openssl`，用于 HTTPS。

测试 HTTP 播放：

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

该测试会在 Android 上通过 `http://127.0.0.1:8765/tone.wav` 调用 Rust 播放，并断言播放进度推进。

## 音频输出

Android 默认使用：

```text
openslessink
```

耳机、蓝牙、扬声器路由通常由 Android 系统音频策略控制，而不是像 Linux PulseAudio/PipeWire 那样由 GStreamer 暴露完整设备列表。项目保留了 `listOutputDevices` 和 `setOutputDevice` API，但 Android 上可选设备可能为空或只能使用系统默认路由。

## 常见问题

### `glib-sys` 或 `gstreamer-sys` 找不到 `.pc`

典型错误：

```text
pkg-config has not been configured to support cross-compilation
glib-2.0.pc not found
gstreamer-1.0.pc not found
```

原因是 Android 交叉编译时没有设置 GStreamer SDK 的 per-ABI `pkg-config` 路径。当前项目在 Cargokit 中设置：

```text
PKG_CONFIG_ALLOW_CROSS=1
PKG_CONFIG_PATH=<sdk>/<abi>/lib/pkgconfig:<sdk>/<abi>/lib/gstreamer-1.0/pkgconfig:<sdk>/<abi>/lib/gio/modules/pkgconfig
PKG_CONFIG_LIBDIR=<same paths>
SYSTEM_DEPS_LINK=static
```

开发者只需要在 Flutter/Gradle 构建时提供：

```bash
GSTREAMER_ROOT_ANDROID=/path/to/gstreamer-1.0-android-universal-1.28.2
```

### `dlopen failed: cannot locate symbol "gst_base_transform_get_type"`

原因是插件依赖的 GStreamer API 库没有被静态链接进 `libgst_audio_core.so`。之前的过滤逻辑错误地跳过了 `gstbase-1.0` 这类库。

当前 `rust/build.rs` 只跳过已经显式 whole-archive 的插件库，不再跳过 `gstbase-1.0`、`gstaudio-1.0`、`gsttag-1.0` 等 API 库。

### Rust 插件导致 `__rustc::__rdl_alloc` undefined

GStreamer Android SDK 中部分插件本身由 Rust 编写，例如 `uriplaylistbin`、`reqwest` 和一些 `rs*` 插件。它们的静态库可能暴露和当前应用 Rust toolchain 不兼容的 allocator 符号。

本项目有意不链接 `uriplaylistbin`。播放列表、随机播放、循环播放等逻辑在 `rust/src/api/player.rs` 中自己实现，不依赖 GStreamer playlist 插件。

添加新插件时，如果链接错误出现类似符号：

```text
__rustc::__rdl_alloc
__rustc::__rdl_dealloc
```

优先检查该插件是否为 GStreamer Rust 插件。能不用就不要加入最小播放集合。

### `cargo check` 通过但 APK 运行失败

`cargo check` 只验证 Rust 类型和 build script，大多数情况下不会完整链接 Android `.so`。必须至少跑：

```bash
GSTREAMER_ROOT_ANDROID=/path/to/sdk flutter build apk --debug --target-platform android-x64
```

并安装启动：

```bash
adb install -r build/app/outputs/flutter-apk/app-debug.apk
adb shell am start -W -n com.example.gst_audio_flutter/.MainActivity
adb logcat -d | grep -E 'flutter|GStreamer|dlopen|AndroidRuntime'
```

### 重复 shutdown 报 closed channel

Flutter 页面销毁和测试 `tearDownAll` 可能都会调用 `shutdown_player()`。播放器关闭逻辑已改成幂等，并支持关闭后再次 `ensure_runtime()` 重启。

### Debug 构建为什么会编多个 ABI

Cargokit debug 构建会额外加入模拟器 ABI，例如 `android-x86` 和 `android-x64`。即使命令使用了 `--target-platform android-x64`，日志里仍可能看到多个 Android Rust target。最终 APK 会按 Flutter 构建产物包含目标 ABI。

## 修改后建议跑的检查

Rust：

```bash
cargo check --manifest-path rust/Cargo.toml
cargo test --manifest-path rust/Cargo.toml
```

Flutter：

```bash
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter analyze
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH flutter test
```

Android：

```bash
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter build apk --debug --target-platform android-x64
```

Android 播放：

```bash
python3 -m http.server 8765 --bind 127.0.0.1 --directory test-assets
adb reverse tcp:8765 tcp:8765
PATH=/home/chrome-book/fvm/versions/3.41.6/bin:$PATH \
GSTREAMER_ROOT_ANDROID=/home/chrome-book/code/.cache/gstreamer-android/gstreamer-1.0-android-universal-1.28.2 \
flutter test integration_test/android_playback_test.dart -d emulator-5554
```
