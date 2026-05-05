# GStreamer iOS 开发与打包指南

本文档记录本项目 iOS 端 GStreamer 接入、CI 构建、unsigned IPA 打包、iOS simulator integration test，以及真机排查路径。

项目根目录：

```text
/home/chrome-book/code/gst_audio_flutter
```

## 当前状态

- GitHub Actions `iOS Build` 会在 `push`、`pull_request` 和手动 `workflow_dispatch` 时运行。
- 已验证 `iphoneos` Release 无签名构建。
- 已生成并上传 unsigned IPA artifact：`gst-audio-flutter-ios-unsigned`。
- 已验证 iOS simulator integration test，覆盖 bundle WAV、HTTP WAV、播放、暂停、恢复、下一首、上一首。
- 最近验证通过的 iOS run：`25373344533`，commit `063aab5`。测试日志确认通过：

```text
plays bundled WAV through Rust GStreamer on iOS
plays HTTP WAV through Rust GStreamer on iOS
handles pause resume next and previous controls on iOS
```

仍未完成的部分：

- 没有 Apple Developer 签名、provisioning profile、TestFlight 或 App Store export 流程。
- unsigned IPA 不能直接安装到普通真机。真机安装需要额外签名或由后续签名流程处理。
- CI 的 iOS simulator 测试只覆盖当前 `test-assets/tone.wav` 和本地 HTTP WAV，不等价于所有真实媒体源、真机音频路由、蜂窝网络、后台播放或 App Store 环境。

## 参考文档

- GStreamer iOS 安装文档：<https://gstreamer.freedesktop.org/documentation/installing/for-ios-development.html>
- GStreamer iOS tutorials：<https://gstreamer.freedesktop.org/documentation/tutorials/ios/index.html>
- GStreamer iOS tutorial 4，`playbin` 基础播放器：<https://gstreamer.freedesktop.org/documentation/tutorials/ios/a-basic-media-player.html>
- Flutter iOS 部署：<https://docs.flutter.dev/deployment/ios>
- Flutter Rust Bridge Cargokit：<https://cjycode.com/flutter_rust_bridge/manual/integrate/cargokit>
- GStreamer iOS 1.28.2 包目录：<https://gstreamer.freedesktop.org/pkg/ios/1.28.2/>

官方 iOS 文档仍会讲到旧的 GStreamer SDK installer、`GStreamer.framework` 和 Xcode templates。当前项目没有使用模板工程，而是使用 GStreamer `1.28.2` 的 `GStreamer.xcframework` tar 包，并通过 Flutter + Rust + Cargokit 接入。

## 关键文件

```text
.github/workflows/ios-build.yml
integration_test/ios_playback_test.dart
ios/Runner/AppDelegate.swift
ios/Runner/Info.plist
rust/src/api/player.rs
rust/build.rs
rust_builder/ios/gst_audio_core.podspec
rust_builder/cargokit/build_tool/lib/src/ios_environment.dart
rust_builder/cargokit/build_tool/lib/src/build_pod.dart
rust_builder/cargokit/build_tool/lib/src/builder.dart
```

## 项目架构

播放链路：

```text
Flutter UI
  -> flutter_rust_bridge generated Dart API
  -> rust/src/api/player.rs
  -> gstreamer-rs
  -> GStreamer playbin
  -> osxaudiosink
  -> iOS audio output
```

官方 iOS tutorials 建议把 GStreamer 处理放到独立 backend 中，避免阻塞 UI。当前项目对应实现是 Rust 播放线程 `gst-audio-player`，Flutter 只通过 FRB 发送命令和读取状态。

iOS 默认音频 sink：

```text
osxaudiosink
```

Android 使用 `openslessink`，桌面使用 `autoaudiosink`。不要在 iOS 上复用 Android sink 或 Android 静态插件注册逻辑。

## GStreamer iOS SDK

当前 workflow 使用：

```text
https://gstreamer.freedesktop.org/pkg/ios/1.28.2/gstreamer-1.28.2-xcframework.tar.xz
```

SHA256：

```text
c2e538da4430c73ac0fdf20db635dc342142cd401cf9e9b00b94dcbb9ccba542
```

CI 缓存目录：

```text
~/.cache/gstreamer-ios
```

当前需要两个 slice：

```text
GStreamer.xcframework/ios-arm64
GStreamer.xcframework/<ios simulator slice>
```

`ios-arm64` 用于 `iphoneos` Release 构建和 unsigned IPA。simulator slice 用于 `flutter test integration_test/ios_playback_test.dart -d <simulator-id>`。

## 本地 macOS SDK 准备

只能在 macOS + Xcode 环境构建 iOS。Linux 本机不能完成 iOS 构建。

```bash
export GSTREAMER_IOS_VERSION=1.28.2
export GSTREAMER_IOS_XCFRAMEWORK_SHA256=c2e538da4430c73ac0fdf20db635dc342142cd401cf9e9b00b94dcbb9ccba542

mkdir -p "$HOME/.cache/gstreamer-ios"
cd "$HOME/.cache/gstreamer-ios"

curl --fail --location --retry 5 --retry-delay 10 \
  --output "gstreamer-${GSTREAMER_IOS_VERSION}-xcframework.tar.xz" \
  "https://gstreamer.freedesktop.org/pkg/ios/${GSTREAMER_IOS_VERSION}/gstreamer-${GSTREAMER_IOS_VERSION}-xcframework.tar.xz"

echo "${GSTREAMER_IOS_XCFRAMEWORK_SHA256}  gstreamer-${GSTREAMER_IOS_VERSION}-xcframework.tar.xz" \
  | shasum --algorithm 256 --check -

mkdir -p "gstreamer-${GSTREAMER_IOS_VERSION}-xcframework"
tar -xJf "gstreamer-${GSTREAMER_IOS_VERSION}-xcframework.tar.xz" \
  -C "gstreamer-${GSTREAMER_IOS_VERSION}-xcframework"

export GSTREAMER_IOS_XCFRAMEWORK="$HOME/.cache/gstreamer-ios/gstreamer-${GSTREAMER_IOS_VERSION}-xcframework/GStreamer.xcframework"
export GSTREAMER_IOS_DEVICE_LIBRARY_DIR="$GSTREAMER_IOS_XCFRAMEWORK/ios-arm64"
```

如果只想复用 Cargokit 的 slice 自动选择，设置：

```bash
export GSTREAMER_IOS_XCFRAMEWORK=/path/to/GStreamer.xcframework
```

如果要显式指定当前构建使用的 slice，设置：

```bash
export GSTREAMER_IOS_LIBRARY_DIR=/path/to/GStreamer.xcframework/ios-arm64
```

## system-deps 环境

iOS 上 `pkg-config` 不是主要入口。`rust_builder/cargokit/build_tool/lib/src/ios_environment.dart` 会把 GStreamer headers 和 `libGStreamer.a` 通过 `system-deps` 环境变量交给 Cargo：

```text
SYSTEM_DEPS_GLIB_2_0_NO_PKG_CONFIG=1
SYSTEM_DEPS_GLIB_2_0_LIB=GStreamer
SYSTEM_DEPS_GLIB_2_0_SEARCH_NATIVE=<selected GStreamer.xcframework slice>
SYSTEM_DEPS_GLIB_2_0_INCLUDE=<Headers paths>
```

`GOBJECT_2_0`、`GIO_2_0`、`GSTREAMER_1_0` 同理。

这里不要使用：

```text
SYSTEM_DEPS_*_LIB=static=GStreamer
```

否则 Rust staticlib 会尝试内嵌整个 `libGStreamer.a`，导致 `libgst_audio_core.a` 过大，并在 Xcode `-force_load` 时拉入大量不需要的插件和第三方编解码器对象。

## Xcode 链接项

iOS Xcode 链接阶段需要统一链接 GStreamer 和系统库：

```text
-L${GSTREAMER_IOS_LIBRARY_DIR}
-lGStreamer
-lresolv
-liconv
-lsqlite3
-lc++
-framework AVFoundation
-framework AssetsLibrary
-framework AudioToolbox
-framework CoreAudio
-framework CoreFoundation
-framework CoreMedia
-framework CoreVideo
-framework Foundation
-framework VideoToolbox
```

这些 flags 同时体现在：

```text
.github/workflows/ios-build.yml
rust_builder/ios/gst_audio_core.podspec
```

## 静态插件注册

iOS 静态链接 `libGStreamer.a` 时，不能依赖运行时扫描 `.so` 插件目录。当前在 `rust/src/api/player.rs` 中显式注册静态插件：

```text
coreelements
playback
typefindfunctions
audioconvert
audioparsers
audioresample
volume
autodetect
osxaudio
gio
wavparse
id3demux
isomp4
matroska
ogg
vorbis
opus
flac
icydemux
soup
```

这些插件覆盖当前测试所需的本地 WAV、HTTP WAV、常见容器和部分音频格式。真实业务媒体如果是 MP3、AAC、M4A、HLS 或其他格式，需要用实际文件验证，并按失败日志补充 demux/parser/decoder 插件注册。

## iOS runtime 配置

`ios/Runner/AppDelegate.swift` 配置了 iOS 音频会话：

```text
AVAudioSessionCategoryPlayback
```

`ios/Runner/Info.plist` 当前允许 HTTP 明文请求：

```text
NSAppTransportSecurity
  NSAllowsArbitraryLoads = true
```

这是为了 CI 和本地 HTTP 测试。生产应用更建议改成按域名配置 ATS 例外，或全部使用 HTTPS。

Rust 侧 iOS runtime 会给 GStreamer/GLib 补齐可写目录环境变量：

```text
TMP
TEMP
TMPDIR
XDG_RUNTIME_DIR
XDG_CACHE_HOME
HOME
```

这些变量指向 iOS app 可用的临时目录，避免 GLib 尝试访问不可写的默认路径。

## CI 构建流程

workflow：

```text
.github/workflows/ios-build.yml
```

触发方式：

```text
push
pull_request
workflow_dispatch
```

命令行手动触发：

```bash
gh workflow run ios-build.yml --ref main
```

查看最近运行：

```bash
gh run list --workflow ios-build.yml --limit 5
```

监控运行：

```bash
gh run watch <run-id> --exit-status --interval 30
```

下载 artifact 和 integration logs：

```bash
mkdir -p /tmp/gst-audio-ios-run
gh run download <run-id> --dir /tmp/gst-audio-ios-run
find /tmp/gst-audio-ios-run -maxdepth 3 -type f -print
```

workflow 阶段：

1. Check out repository。
2. 安装 Rust stable。
3. 安装 Flutter `3.41.6`。
4. 下载并缓存 GStreamer iOS SDK。
5. 解压 `GStreamer.xcframework` 的 device 和 simulator slice。
6. `flutter pub get`。
7. `flutter build ios --release --config-only --no-codesign`。
8. `xcodebuild` 做 `iphoneos` Release 无签名构建。
9. 把 `Runner.app` 打成 unsigned IPA。
10. 上传 IPA artifact。
11. 启动 iOS simulator。
12. 启动本地 HTTP server。
13. 执行 `integration_test/ios_playback_test.dart`。
14. 上传 `flutter-test.log`、`simulator.log` 和 crash reports。

## 无签名 iOS build

关键命令：

```bash
flutter build ios --release --config-only --no-codesign

xcodebuild \
  -workspace ios/Runner.xcworkspace \
  -scheme Runner \
  -configuration Release \
  -sdk iphoneos \
  -destination "generic/platform=iOS" \
  CODE_SIGNING_ALLOWED=NO \
  CODE_SIGNING_REQUIRED=NO \
  CODE_SIGN_IDENTITY="" \
  CODE_SIGN_STYLE=Manual \
  DEVELOPMENT_TEAM="" \
  PROVISIONING_PROFILE="" \
  PROVISIONING_PROFILE_SPECIFIER="" \
  GSTREAMER_IOS_LIBRARY_DIR="${GSTREAMER_IOS_DEVICE_LIBRARY_DIR}" \
  LIBRARY_SEARCH_PATHS="$(inherited) ${GSTREAMER_IOS_DEVICE_LIBRARY_DIR}" \
  OTHER_LDFLAGS="$(inherited) -L${GSTREAMER_IOS_DEVICE_LIBRARY_DIR} -lGStreamer -lresolv -liconv -lsqlite3 -lc++ -framework AVFoundation -framework AssetsLibrary -framework AudioToolbox -framework CoreAudio -framework CoreFoundation -framework CoreMedia -framework CoreVideo -framework Foundation -framework VideoToolbox" \
  build
```

## unsigned IPA

无签名 `xcodebuild build` 默认只生成 `.app`，不会生成 `.ipa`。IPA 本质是 zip，内部结构必须是：

```text
Payload/
  Runner.app/
```

当前 workflow 生成：

```text
build/ios/gst-audio-flutter-ios-unsigned.ipa
```

打包命令：

```bash
artifact_root="$PWD/build/ios/unsigned-ipa"
payload_dir="$artifact_root/Payload"
ipa_path="$PWD/build/ios/gst-audio-flutter-ios-unsigned.ipa"

rm -rf "$artifact_root" "$ipa_path"
mkdir -p "$payload_dir"
cp -R /path/to/Runner.app "$payload_dir/Runner.app"
(cd "$artifact_root" && /usr/bin/zip -qry "$ipa_path" Payload)
```

注意：unsigned IPA 只是构建产物，不代表可安装。真机安装、TestFlight 或 App Store 发布需要签名后的 archive/export 流程。

## iOS simulator integration test

当前测试文件：

```text
integration_test/ios_playback_test.dart
```

覆盖：

- bundle WAV 启动播放。
- HTTP WAV 启动播放。
- 播放控制：play、pause、resume、next、previous。

CI 命令核心：

```bash
xcrun simctl boot "$simulator_id" || true
xcrun simctl bootstatus "$simulator_id" -b

python3 -m http.server 8765 --directory test-assets &

GSTREAMER_IOS_LIBRARY_DIR="$GSTREAMER_IOS_SIMULATOR_LIBRARY_DIR" \
  flutter test integration_test/ios_playback_test.dart -d "$simulator_id"
```

本地 macOS 运行时，先确认 simulator slice 存在：

```bash
find "$GSTREAMER_IOS_XCFRAMEWORK" -maxdepth 2 -path '*/ios-*simulator/libGStreamer.a' -print
```

然后选择一个 simulator：

```bash
xcrun simctl list devices available
```

再运行测试：

```bash
export GSTREAMER_IOS_LIBRARY_DIR=/path/to/GStreamer.xcframework/ios-arm64_x86_64-simulator
python3 -m http.server 8765 --directory test-assets &
flutter test integration_test/ios_playback_test.dart -d <simulator-id>
```

## 验证 IPA 内容

下载 artifact 后可以检查二进制是否包含当前 iOS 插件注册符号和错误字符串：

```bash
mkdir -p /tmp/gst-audio-ios-ipa
gh run download <run-id> --name gst-audio-flutter-ios-unsigned --dir /tmp/gst-audio-ios-ipa
cd /tmp/gst-audio-ios-ipa
unzip -q gst-audio-flutter-ios-unsigned.ipa -d ipa

strings ipa/Payload/Runner.app/Runner | rg 'gst_plugin_(soup|osxaudio|wavparse|flac|ogg|vorbis|opus|isomp4|matroska)_register'
strings ipa/Payload/Runner.app/Runner | rg 'sending on a closed channel|player command channel closed' || true
```

如果用户手里的 IPA 仍包含旧错误字符串，或者缺少新增插件符号，先确认下载的是最新 run 的 artifact，或者签名流程没有使用旧包。

## 真机排查

### `failed to start playback: Element failed to change its state`

常见原因：

- 当前 IPA 不是最新 commit 构建。
- `osxaudiosink` 静态插件没有注册成功。
- `AVAudioSession` 没有按 playback 配置。
- 传入 URI 不可访问，或者 iOS ATS 阻止 HTTP。
- 媒体格式需要的 demux/parser/decoder 插件没有注册。
- pipeline 前一次状态没有完全停到 `NULL`，又切换了 URI。

排查步骤：

```bash
strings Runner.app/Runner | rg 'gst_plugin_osxaudio_register|AVAudioSession|audio\\+soft-volume\\+buffering'
```

在 Xcode Devices and Simulators 或 Console.app 抓取设备日志，重点搜索：

```text
GStreamer
gst_audio
osxaudiosink
playbin
not-linked
missing-plugin
no element
```

如果能控制启动环境，临时打开 GStreamer 日志：

```text
GST_DEBUG=2,playbin:4,uridecodebin:4,souphttpsrc:4,typefind:4
```

### `Internal data stream error ... GstSoupHTTPSrc ... reason not-linked (-1)`

这个错误说明 HTTP source 已经开始输出数据，但后续没有成功连接到可处理该媒体的 demux/parser/decoder。`soup` 只解决 HTTP 拉流，不负责解码。

处理路径：

1. 确认 URL 返回的是预期媒体，不是 HTML、重定向页或错误页。
2. 在桌面用 `gst-discoverer-1.0 <url>` 或 `gst-launch-1.0 playbin uri=<url>` 确认容器和编码。
3. 根据实际格式补充 iOS 静态插件注册。
4. 把同一媒体加入 `integration_test/ios_playback_test.dart`，让 CI 覆盖。

当前 CI 的 HTTP 测试是 WAV：

```text
http://127.0.0.1:8765/tone.wav
```

如果真机失败的媒体是 MP3、M4A、AAC、FLAC、OGG、HLS 或远端 HTTPS，不能只用 WAV 测试代表。

### `failed to send player command: sending on a closed channel`

这个错误表示 Rust 播放线程已经退出，Flutter 仍在发命令。当前代码通过全局 runtime 和 shutdown 控制避免 view 切换时过早关闭，并且 iOS integration test 已覆盖 pause/resume/next/previous 不触发该错误。

如果真机仍出现：

1. 确认 IPA 是 commit `ae2b9a9` 之后构建，最好是 `063aab5` 或更新。
2. 检查设备日志中播放线程退出前的第一条 GStreamer error。
3. 优先修导致线程退出的根因，不要只在 Dart 层吞掉 closed channel。

### CI 通过但真机失败

CI 当前保证：

- `iphoneos` Release 能链接成功。
- unsigned IPA 能产出。
- iOS simulator 能播放 bundle WAV 和 HTTP WAV。
- simulator 上播放控制链路不崩溃。

CI 当前不保证：

- 手工签名后的 IPA 和 CI artifact 是同一个包。
- 真机音频输出、静音开关、蓝牙、后台播放都可用。
- 真实业务 URL 和格式都有对应插件。
- HTTPS/TLS、重定向、鉴权、Range request、服务器 MIME type 都正确。

真机失败时，先把失败媒体缩小成一个可公开或可本地托管的测试文件，再加入 iOS integration test。这样后续每次 CI 都能复现。

## 常见构建问题

### `flutter build ios --no-codesign` 仍触发签名校验

处理：CI 不直接依赖 Flutter 完成完整 build，而是先生成配置：

```bash
flutter build ios --release --config-only --no-codesign
```

再直接调用 `xcodebuild`，显式关闭签名：

```text
CODE_SIGNING_ALLOWED=NO
CODE_SIGNING_REQUIRED=NO
CODE_SIGN_IDENTITY=""
CODE_SIGN_STYLE=Manual
DEVELOPMENT_TEAM=""
PROVISIONING_PROFILE=""
PROVISIONING_PROFILE_SPECIFIER=""
```

### 旧 `.pkg` SDK 路径不适合当前 CI

官方旧文档会提到：

```text
~/Library/Developer/GStreamer/iPhone.sdk
GStreamer.framework
Templates
```

当前 CI 使用：

```text
GStreamer.xcframework/<slice>/libGStreamer.a
GStreamer.xcframework/<slice>/Headers
```

不要把旧 framework 路径和当前 xcframework slice 混用。

### GitHub Actions 磁盘压力

完整 GStreamer iOS archive 很大。workflow 只解压 `Info.plist`、`ios-arm64` 和 simulator slice，并使用 Actions cache 缓存 `~/.cache/gstreamer-ios`。

### Rust 链接缺少 `res_9_*`

现象：

```text
Undefined symbols for architecture arm64:
  "_res_9_dn_expand"
  "_res_9_ndestroy"
  "_res_9_ninit"
  "_res_9_nquery"
```

原因：GIO resolver 需要 iOS `libresolv`。

处理：保留 `-lresolv`，并在 `rust/build.rs` iOS target 下输出：

```rust
println!("cargo:rustc-link-lib=resolv");
```

### `lipo` fat archive 4GB 限制

现象：

```text
fatal error: lipo: file too large to be in a fat file because the size field in struct fat_arch is only 32-bits
```

处理：Cargokit 已调整为单架构 iOS 构建时直接复制单个 `.a`，不执行 `lipo -create`。

### Rust staticlib 内嵌 GStreamer 后 Xcode 链接失败

现象：`libgst_audio_core.a` 内出现大量 GStreamer 插件和第三方编解码器对象，并在 Xcode 链接时报大量 undefined symbols。

处理：`SYSTEM_DEPS_*_LIB` 使用：

```text
GStreamer
```

不要使用：

```text
static=GStreamer
```

最终由 Xcode 统一链接官方 `libGStreamer.a`。

### 缺少 iOS 系统 framework 和运行库

现象：

```text
Undefined symbols for architecture arm64:
  "_AVAudioSessionCategoryPlayback"
  "_VTCompressionSessionCreate"
  "std::runtime_error::what() const"
  "_iconv"
  "_sqlite3_open"
  "_OBJC_CLASS_$_ALAssetsLibrary"
```

处理：补齐 `OTHER_LDFLAGS` 中的系统库和 framework。完整列表见本文 "Xcode 链接项"。

## 后续待办

- 增加签名构建流程：证书、provisioning profile、`xcodebuild archive`、`xcodebuild -exportArchive`。
- 把真机失败的实际媒体加入 iOS integration test。
- 根据真实业务格式补充 iOS 静态插件和解码器注册。
- 收紧 ATS 配置，避免生产环境使用全局 `NSAllowsArbitraryLoads`。
- 明确后台音频、锁屏控制、耳机/蓝牙路由的产品需求和测试覆盖。
