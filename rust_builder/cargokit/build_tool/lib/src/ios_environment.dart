import 'dart:io';

import 'package:path/path.dart' as path;

import 'builder.dart';
import 'environment.dart';
import 'target.dart';

class IOSGStreamerEnvironment {
  IOSGStreamerEnvironment({required this.target});

  final Target target;

  Map<String, String> buildEnvironment() {
    final libraryDir = _libraryDir();
    final headersDir = path.join(libraryDir, 'Headers');
    if (!Directory(headersDir).existsSync()) {
      throw BuildException(
        'GStreamer iOS headers directory does not exist: $headersDir',
      );
    }

    final includePaths = [
      headersDir,
      path.join(headersDir, 'glib-2.0'),
      path.join(headersDir, 'gstreamer-1.0'),
      path.join(headersDir, target.darwinArch ?? ''),
    ].where((entry) => entry.isNotEmpty).join(':');

    final env = <String, String>{
      'GSTREAMER_IOS_LIBRARY_DIR': libraryDir,
    };

    for (final dep in ['GLIB_2_0', 'GOBJECT_2_0', 'GIO_2_0', 'GSTREAMER_1_0']) {
      env.addAll({
        'SYSTEM_DEPS_${dep}_NO_PKG_CONFIG': '1',
        'SYSTEM_DEPS_${dep}_LIB': 'GStreamer',
        'SYSTEM_DEPS_${dep}_SEARCH_NATIVE': libraryDir,
        'SYSTEM_DEPS_${dep}_INCLUDE': includePaths,
      });
    }

    return env;
  }

  String _libraryDir() {
    final explicitLibraryDir = Environment.gstreamerIosLibraryDir;
    if (explicitLibraryDir != null) {
      return _validateLibraryDir(explicitLibraryDir);
    }

    final xcframework = Environment.gstreamerIosXcframework;
    if (xcframework == null) {
      throw BuildException(
        'Set GSTREAMER_IOS_XCFRAMEWORK to GStreamer.xcframework, or '
        'GSTREAMER_IOS_LIBRARY_DIR to the selected xcframework slice.',
      );
    }

    final platform = target.darwinPlatform;
    final arch = target.darwinArch;
    final sliceCandidates = switch ((platform, arch)) {
      ('iphoneos', 'arm64') => ['ios-arm64'],
      ('iphonesimulator', 'arm64') => [
          'ios-arm64_x86_64-simulator',
          'ios-arm64-simulator',
        ],
      ('iphonesimulator', 'x86_64') => [
          'ios-arm64_x86_64-simulator',
          'ios-x86_64-simulator',
        ],
      _ => <String>[],
    };

    for (final slice in sliceCandidates) {
      final candidate = path.join(xcframework, slice);
      if (File(path.join(candidate, 'libGStreamer.a')).existsSync()) {
        return _validateLibraryDir(candidate);
      }
    }

    throw BuildException(
      'No GStreamer iOS xcframework slice found for ${target.rust} under '
      '$xcframework. Tried: ${sliceCandidates.join(', ')}',
    );
  }

  String _validateLibraryDir(String libraryDir) {
    if (!File(path.join(libraryDir, 'libGStreamer.a')).existsSync()) {
      throw BuildException(
        'GStreamer iOS static library was not found at '
        '${path.join(libraryDir, 'libGStreamer.a')}',
      );
    }
    return libraryDir;
  }
}
