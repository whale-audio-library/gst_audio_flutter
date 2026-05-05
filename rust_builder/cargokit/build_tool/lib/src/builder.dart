/// This is copied from Cargokit (which is the official way to use it currently)
/// Details: https://fzyzcjy.github.io/flutter_rust_bridge/manual/integrate/builtin

import 'dart:io';

import 'package:collection/collection.dart';
import 'package:logging/logging.dart';
import 'package:path/path.dart' as path;

import 'android_environment.dart';
import 'cargo.dart';
import 'environment.dart';
import 'ios_environment.dart';
import 'options.dart';
import 'rustup.dart';
import 'target.dart';
import 'util.dart';
import 'windows_environment.dart';

final _log = Logger('builder');

enum BuildConfiguration {
  debug,
  release,
  profile,
}

extension on BuildConfiguration {
  bool get isDebug => this == BuildConfiguration.debug;
  String get rustName => switch (this) {
        BuildConfiguration.debug => 'debug',
        BuildConfiguration.release => 'release',
        BuildConfiguration.profile => 'release',
      };
}

class BuildException implements Exception {
  final String message;

  BuildException(this.message);

  @override
  String toString() {
    return 'BuildException: $message';
  }
}

class BuildEnvironment {
  final BuildConfiguration configuration;
  final CargokitCrateOptions crateOptions;
  final String targetTempDir;
  final String manifestDir;
  final CrateInfo crateInfo;

  final bool isAndroid;
  final String? androidSdkPath;
  final String? androidNdkVersion;
  final int? androidMinSdkVersion;
  final String? javaHome;

  BuildEnvironment({
    required this.configuration,
    required this.crateOptions,
    required this.targetTempDir,
    required this.manifestDir,
    required this.crateInfo,
    required this.isAndroid,
    this.androidSdkPath,
    this.androidNdkVersion,
    this.androidMinSdkVersion,
    this.javaHome,
  });

  static BuildConfiguration parseBuildConfiguration(String value) {
    // XCode configuration adds the flavor to configuration name.
    final firstSegment = value.split('-').first;
    final buildConfiguration = BuildConfiguration.values.firstWhereOrNull(
      (e) => e.name == firstSegment,
    );
    if (buildConfiguration == null) {
      _log.warning('Unknown build configuraiton $value, will assume release');
      return BuildConfiguration.release;
    }
    return buildConfiguration;
  }

  static BuildEnvironment fromEnvironment({
    required bool isAndroid,
  }) {
    final buildConfiguration =
        parseBuildConfiguration(Environment.configuration);
    final manifestDir = Environment.manifestDir;
    final crateOptions = CargokitCrateOptions.load(
      manifestDir: manifestDir,
    );
    final crateInfo = CrateInfo.load(manifestDir);
    return BuildEnvironment(
      configuration: buildConfiguration,
      crateOptions: crateOptions,
      targetTempDir: Environment.targetTempDir,
      manifestDir: manifestDir,
      crateInfo: crateInfo,
      isAndroid: isAndroid,
      androidSdkPath: isAndroid ? Environment.sdkPath : null,
      androidNdkVersion: isAndroid ? Environment.ndkVersion : null,
      androidMinSdkVersion:
          isAndroid ? int.parse(Environment.minSdkVersion) : null,
      javaHome: isAndroid ? Environment.javaHome : null,
    );
  }
}

class RustBuilder {
  final Target target;
  final BuildEnvironment environment;

  RustBuilder({
    required this.target,
    required this.environment,
  });

  void prepare(
    Rustup rustup,
  ) {
    final toolchain = _toolchain;
    if (rustup.installedTargets(toolchain) == null) {
      rustup.installToolchain(toolchain);
    }
    if (toolchain == 'nightly') {
      rustup.installRustSrcForNightly();
    }
    if (!rustup.installedTargets(toolchain)!.contains(target.rust)) {
      rustup.installTarget(target.rust, toolchain: toolchain);
    }
  }

  CargoBuildOptions? get _buildOptions =>
      environment.crateOptions.cargo[environment.configuration];

  String get _toolchain => _buildOptions?.toolchain.name ?? 'stable';

  /// Returns the path of directory containing build artifacts.
  Future<String> build() async {
    final extraArgs = _buildOptions?.flags ?? [];
    final manifestPath = _manifestPath();
    runCommand(
      'rustup',
      [
        'run',
        _toolchain,
        'cargo',
        if (target.darwinPlatform != null) 'rustc' else 'build',
        ...extraArgs,
        '--manifest-path',
        manifestPath,
        '-p',
        environment.crateInfo.packageName,
        if (!environment.configuration.isDebug) '--release',
        '--target',
        target.rust,
        '--target-dir',
        environment.targetTempDir,
        if (target.darwinPlatform != null) ...[
          '--',
          '--crate-type',
          'staticlib',
        ],
      ],
      environment: await _buildEnvironment(),
    );
    return path.join(
      environment.targetTempDir,
      target.rust,
      environment.configuration.rustName,
    );
  }

  String _manifestPath() {
    final manifestPath = path.join(environment.manifestDir, 'Cargo.toml');
    if (target.darwinPlatform == null) {
      return manifestPath;
    }

    final staticlibManifestDir = Directory(path.join(
      environment.targetTempDir,
      'cargokit',
      'staticlib_manifest',
      target.rust,
    ));
    staticlibManifestDir.createSync(recursive: true);

    final sourceManifest = File(manifestPath);
    final staticlibManifest = File(
      path.join(staticlibManifestDir.path, 'Cargo.toml'),
    );
    staticlibManifest.writeAsStringSync(_staticlibManifest(
      sourceManifest.readAsStringSync(),
      sourceDir: environment.manifestDir,
    ));

    final sourceLockfile =
        File(path.join(environment.manifestDir, 'Cargo.lock'));
    if (sourceLockfile.existsSync()) {
      sourceLockfile.copySync(path.join(staticlibManifestDir.path, 'Cargo.lock'));
    }

    return staticlibManifest.path;
  }

  String _staticlibManifest(String manifest, {required String sourceDir}) {
    final libPath = _tomlString(path.join(sourceDir, 'src', 'lib.rs'));
    final buildPath = _tomlString(path.join(sourceDir, 'build.rs'));
    final buildScript = File(path.join(sourceDir, 'build.rs')).existsSync()
        ? 'build = "$buildPath"\n'
        : '';
    final staticlibLibSection = [
      '[lib]',
      'path = "$libPath"',
      'crate-type = ["staticlib"]',
      '',
    ].join('\n');

    final packageSection = RegExp(
      r'(?ms)^\[package\]\s*$.*?(?=^\[|\z)',
    );
    final withBuildScript = manifest.replaceFirstMapped(
      packageSection,
      (match) {
        final section = match.group(0)!;
        if (buildScript.isEmpty || RegExp(r'(?m)^build\s*=').hasMatch(section)) {
          return section;
        }
        return section.replaceFirst('\n', '\n$buildScript');
      },
    );

    final libSection = RegExp(r'(?ms)^\[lib\]\s*$.*?(?=^\[|\z)');
    if (libSection.hasMatch(withBuildScript)) {
      return withBuildScript.replaceFirst(libSection, staticlibLibSection);
    }

    return '$withBuildScript\n$staticlibLibSection';
  }

  String _tomlString(String value) =>
      value.replaceAll('\\', r'\\').replaceAll('"', r'\"');

  Future<Map<String, String>> _buildEnvironment() async {
    if (target.android != null) {
      final sdkPath = environment.androidSdkPath;
      final ndkVersion = environment.androidNdkVersion;
      final minSdkVersion = environment.androidMinSdkVersion;
      if (sdkPath == null) {
        throw BuildException('androidSdkPath is not set');
      }
      if (ndkVersion == null) {
        throw BuildException('androidNdkVersion is not set');
      }
      if (minSdkVersion == null) {
        throw BuildException('androidMinSdkVersion is not set');
      }
      final env = AndroidEnvironment(
        sdkPath: sdkPath,
        ndkVersion: ndkVersion,
        minSdkVersion: minSdkVersion,
        targetTempDir: environment.targetTempDir,
        target: target,
      );
      if (!env.ndkIsInstalled() && environment.javaHome != null) {
        env.installNdk(javaHome: environment.javaHome!);
      }
      return env.buildEnvironment();
    }

    if (target.rust.contains('-windows-')) {
      return WindowsGStreamerEnvironment(target: target).buildEnvironment();
    }

    if (target.darwinPlatform != null && target.rust.contains('-ios')) {
      return IOSGStreamerEnvironment(target: target).buildEnvironment();
    }

    return {};
  }
}
