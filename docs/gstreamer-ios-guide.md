# GStreamer iOS 打包指南

本文档记录本项目当前 iOS 无签名构建和 unsigned IPA 打包流程，以及接入 GStreamer iOS SDK、Flutter Rust Bridge/Cargokit 时遇到的问题。

当前状态：

- 已在 GitHub Actions `iOS Build` workflow 验证 `iphoneos` Release 无签名构建。
- 已生成并上传 unsigned IPA artifact。
- 未做 Apple Developer 签名、provisioning profile、TestFlight 或 App Store 导出。

相关文件：

- `.github/workflows/ios-build.yml`
- `rust/build.rs`
- `rust_builder/ios/gst_audio_core.podspec`
- `rust_builder/cargokit/build_tool/lib/src/builder.dart`
- `rust_builder/cargokit/build_tool/lib/src/build_pod.dart`

## 参考文档

- Flutter iOS 部署：<https://docs.flutter.dev/deployment/ios>
- Flutter Rust Bridge Cargokit：<https://cjycode.com/flutter_rust_bridge/manual/integrate/cargokit>
- GStreamer iOS 开发：<https://gstreamer.freedesktop.org/documentation/installing/for-ios-development.html>
- GStreamer iOS 1.28.2 包目录：<https://gstreamer.freedesktop.org/pkg/ios/1.28.2/>

## 当前 CI 入口

iOS 打包使用手动 workflow：

```text
Actions -> iOS Build -> Run workflow -> branch: main
```

命令行触发：

```bash
gh workflow run ios-build.yml --ref main
```

查看最近运行：

```bash
gh run list --workflow ios-build.yml --limit 5
```

查看某次运行：

```bash
gh run view <run-id> --json status,conclusion,url,jobs
```

最近验证过的成功运行：

```text
https://github.com/whale-audio-library/gst_audio_flutter/actions/runs/25360266007
```

## 构建流程

workflow 分为几个阶段：

1. 安装 Rust stable。
2. 安装 Flutter `3.41.6`。
3. 下载并缓存 GStreamer iOS SDK。
4. 注入 `system-deps` 环境变量，让 Rust `glib-sys`、`gio-sys`、`gstreamer-sys` 找到 GStreamer headers 和 `libGStreamer.a`。
5. 执行 `flutter build ios --release --config-only --no-codesign` 生成 iOS/Xcode 构建配置。
6. 执行 `xcodebuild ... CODE_SIGNING_ALLOWED=NO ... build` 做无签名真机 Release 构建。
7. 查找 `Runner.app`，按 `Payload/Runner.app` 结构打包为 unsigned IPA。
8. 上传 artifact `gst-audio-flutter-ios-unsigned`。

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
  build
```

CI 里还额外传入了 GStreamer link flags，完整配置见 `.github/workflows/ios-build.yml`。

## unsigned IPA 产物

无签名 `xcodebuild build` 默认只生成 `.app`，不会生成 `.ipa`。IPA 本质上是一个 zip，内部结构需要是：

```text
Payload/
  Runner.app/
```

当前 workflow 成功后会生成：

```text
build/ios/gst-audio-flutter-ios-unsigned.ipa
```

并上传 GitHub Actions artifact：

```text
gst-audio-flutter-ios-unsigned
```

注意：这个 IPA 是 unsigned。它适合下载检查包内容、交给后续签名流程处理，不能直接安装到普通真机。要真机安装、TestFlight 或发布 App Store，需要 Apple Developer 证书、provisioning profile，并改为 archive/export 或签名后的打包流程。

## GStreamer iOS SDK

从 GStreamer `1.28` 开始，iOS/tvOS 官方分发重点是 `.xcframework` archive。当前 workflow 使用：

```text
https://gstreamer.freedesktop.org/pkg/ios/1.28.2/gstreamer-1.28.2-xcframework.tar.xz
```

SHA256：

```text
c2e538da4430c73ac0fdf20db635dc342142cd401cf9e9b00b94dcbb9ccba542
```

CI 只解压真机构建需要的 slice：

```text
GStreamer.xcframework/Info.plist
GStreamer.xcframework/ios-arm64
```

这样能减少 GitHub Actions 磁盘压力。当前只验证 `iphoneos arm64`，没有验证 simulator slice。

Rust `system-deps` 环境变量大致为：

```text
SYSTEM_DEPS_GLIB_2_0_NO_PKG_CONFIG=1
SYSTEM_DEPS_GLIB_2_0_LIB=GStreamer
SYSTEM_DEPS_GLIB_2_0_SEARCH_NATIVE=<GStreamer.xcframework>/ios-arm64
SYSTEM_DEPS_GLIB_2_0_INCLUDE=<Headers paths>
```

`GOBJECT_2_0`、`GIO_2_0`、`GSTREAMER_1_0` 同理。

这里不能使用 `static=GStreamer`。如果让 Rust staticlib 静态内嵌整个 `libGStreamer.a`，最终 `libgst_audio_core.a` 会非常大，并且 Xcode 链接时会强制拉入大量无关插件和编解码器对象。

## Xcode 链接项

当前 iOS build 需要在 Xcode 链接阶段统一链接 GStreamer 和系统库：

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
-framework CoreMedia
-framework CoreVideo
-framework VideoToolbox
```

这些 flags 同时体现在：

- `.github/workflows/ios-build.yml` 的 `xcodebuild OTHER_LDFLAGS`
- `rust_builder/ios/gst_audio_core.podspec` 的 `pod_target_xcconfig['OTHER_LDFLAGS']`

## Cargokit 调整

Cargokit 默认 Darwin 构建路径会收集 Rust 产物，并对静态库执行 `lipo -create`。本项目做了两个调整：

1. 单架构 iOS 构建时，如果只有一个 source `.a`，直接复制，不再执行 `lipo -create`。
2. Darwin 平台使用 `cargo rustc -- --crate-type staticlib`，只生成 pod 需要的 Rust staticlib。

相关文件：

```text
rust_builder/cargokit/build_tool/lib/src/build_pod.dart
rust_builder/cargokit/build_tool/lib/src/builder.dart
```

## 遇到的问题

### `flutter build ios --no-codesign` 仍触发签名校验

现象：直接跑 `flutter build ios --release --no-codesign` 时，Flutter/Xcode 层仍可能因为 signing 配置失败。

处理：CI 里先用：

```bash
flutter build ios --release --config-only --no-codesign
```

再直接调用 `xcodebuild`，并显式关闭签名：

```text
CODE_SIGNING_ALLOWED=NO
CODE_SIGNING_REQUIRED=NO
CODE_SIGN_IDENTITY=""
CODE_SIGN_STYLE=Manual
DEVELOPMENT_TEAM=""
PROVISIONING_PROFILE=""
PROVISIONING_PROFILE_SPECIFIER=""
```

### 旧 `.pkg` SDK 路径不适合当前 GStreamer 1.28

现象：按旧资料找 `/Library/Frameworks/GStreamer.framework` 或 `.pkg` SDK 路径，CI 上无法稳定得到需要的 pkg-config/header/lib 布局。

处理：按官方当前文档切换到 `gstreamer-1.28.2-xcframework.tar.xz`，直接使用 `GStreamer.xcframework/ios-arm64/libGStreamer.a` 和 headers。

### GitHub Actions 磁盘压力

现象：完整 GStreamer iOS archive 很大，完整解压容易耗时且占用大量磁盘。

处理：只解压 `Info.plist` 和 `ios-arm64` slice，并使用 Actions cache 缓存 `~/.cache/gstreamer-ios`。

### Rust 链接缺少 `res_9_*`

现象：

```text
Undefined symbols for architecture arm64:
  "_res_9_dn_expand"
  "_res_9_ndestroy"
  "_res_9_ninit"
  "_res_9_nquery"
```

原因：GIO resolver 对象需要 iOS `libresolv`。

处理：`rust/build.rs` 在 iOS target 下输出：

```rust
println!("cargo:rustc-link-lib=resolv");
```

同时 Xcode 链接 flags 中保留 `-lresolv`。

### `lipo` fat archive 4GB 限制

现象：

```text
fatal error: lipo: file too large to be in a fat file because the size field in struct fat_arch is only 32-bits
```

原因：单架构 Rust staticlib 已经超过 fat archive 结构限制，Cargokit 仍执行 `lipo -create`。

处理：修改 Cargokit build tool：当只有一个 source file 时直接复制，不执行 `lipo`。

### Rust staticlib 内嵌 GStreamer 后 Xcode 链接失败

现象：`libgst_audio_core.a` 内出现大量 GStreamer 插件和第三方编解码器对象，例如 `libSvtAv1Enc`、`libx265`、`libgstapplemedia`，并出现大量 undefined symbols。

原因：`SYSTEM_DEPS_*_LIB=static=GStreamer` 会让 Rust staticlib 静态内嵌整个 `libGStreamer.a`。pod 再 `-force_load libgst_audio_core.a` 时，Xcode 会强制拉入大量不需要的对象。

处理：把 iOS CI 的 `SYSTEM_DEPS_*_LIB` 改为：

```text
SYSTEM_DEPS_*_LIB=GStreamer
```

让 Rust 只知道依赖关系和 headers，最终由 Xcode 统一链接官方 `libGStreamer.a`。

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

处理：补齐：

```text
-lc++
-liconv
-lsqlite3
-framework AVFoundation
-framework AssetsLibrary
-framework AudioToolbox
-framework CoreAudio
-framework CoreMedia
-framework CoreVideo
-framework VideoToolbox
```

### Build 成功但看不到 IPA

现象：`xcodebuild build` 通过，但 Actions 页面没有 `.ipa`。

原因：无签名 build 默认只生成 `.app`，不自动 archive/export。

处理：workflow 增加 `Package unsigned IPA` 和 `Upload unsigned IPA`：

```bash
mkdir -p build/ios/unsigned-ipa/Payload
cp -R Runner.app build/ios/unsigned-ipa/Payload/Runner.app
(cd build/ios/unsigned-ipa && zip -qry ../gst-audio-flutter-ios-unsigned.ipa Payload)
```

然后通过 `actions/upload-artifact` 上传。

## 后续待办

- 增加签名构建流程：证书、provisioning profile、`xcodebuild archive`、`xcodebuild -exportArchive`。
- 验证真机安装和真实播放。
- 验证 simulator slice，必要时扩展 `GStreamer.xcframework/ios-arm64_x86_64-simulator`。
- 根据实际音频格式裁剪或明确 GStreamer 插件/编解码器依赖。
- 补充 release workflow，把 unsigned IPA 和未来 signed IPA 分开命名。
