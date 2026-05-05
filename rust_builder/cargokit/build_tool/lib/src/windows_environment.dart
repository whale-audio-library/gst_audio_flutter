/// Windows-specific GStreamer environment for Cargokit Cargo builds.

import 'dart:io';

import 'package:path/path.dart' as path;

import 'environment.dart';
import 'target.dart';

class WindowsGStreamerEnvironment {
  WindowsGStreamerEnvironment({
    required this.target,
  });

  final Target target;

  Map<String, String> buildEnvironment() {
    if (!target.rust.contains('-windows-')) {
      return {};
    }

    final root = Environment.gstreamerRootWindows;
    if (root == null) {
      return {};
    }

    final rootDir = Directory(root);
    if (!rootDir.existsSync()) {
      throw Exception('GStreamer Windows SDK directory does not exist: $root');
    }

    final binDir = path.join(root, 'bin');
    final pkgConfigDir = path.join(root, 'lib', 'pkgconfig');
    if (!Directory(pkgConfigDir).existsSync()) {
      throw Exception(
        'GStreamer Windows SDK pkg-config directory does not exist: '
        '$pkgConfigDir',
      );
    }

    final environment = <String, String>{
      'GSTREAMER_ROOT_WINDOWS': root,
      'GSTREAMER_1_0_ROOT_MSVC_X86_64': root,
      'PKG_CONFIG_PATH': _prependPaths('PKG_CONFIG_PATH', [pkgConfigDir]),
      'PKG_CONFIG_LIBDIR': pkgConfigDir,
    };

    if (Directory(binDir).existsSync()) {
      environment['PATH'] = _prependPaths('PATH', [binDir]);
      final pkgConfigExe = File(path.join(binDir, 'pkg-config.exe'));
      if (pkgConfigExe.existsSync()) {
        environment['PKG_CONFIG'] = pkgConfigExe.path;
      }
    }

    return environment;
  }

  String _prependPaths(String key, List<String> values) {
    final entries = [
      ...values.where((value) => value.isNotEmpty),
      if ((Platform.environment[key] ?? '').isNotEmpty)
        Platform.environment[key]!,
    ];
    return entries.join(';');
  }
}
