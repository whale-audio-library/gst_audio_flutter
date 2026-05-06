import 'dart:io' show Platform;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gst_audio_flutter/main.dart';
import 'package:gst_audio_flutter/src/rust/api/player.dart' as player;
import 'package:gst_audio_flutter/src/rust/frb_generated.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() async {
    await RustLib.init();
  });

  tearDownAll(() async {
    await player.stop();
    await player.shutdownPlayer();
  });

  testWidgets('renders visualization and exposes live spectrum bands', (
    tester,
  ) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('audio-visualization')), findsOneWidget);
    expect(
      find.byKey(const ValueKey('visualization-combined-canvas')),
      findsOneWidget,
    );
    expect(find.byKey(const ValueKey('visualization-rms')), findsOneWidget);
    expect(find.byKey(const ValueKey('visualization-peak')), findsOneWidget);
    expect(find.byKey(const ValueKey('visualization-beat')), findsOneWidget);
    expect(find.byKey(const ValueKey('visualization-pcm')), findsOneWidget);

    await player.setPlaylist(inputs: [_bundledToneUri()], startIndex: 0);
    await player.play();

    final frame = await _waitForVisualizationActivity(tester);
    await tester.pump();
    expect(
      find.byKey(const ValueKey('visualization-combined-canvas')),
      findsOneWidget,
    );
    expect(find.text('RMS'), findsOneWidget);
    expect(find.text('Peak'), findsOneWidget);
    expect(find.text('Beat'), findsOneWidget);
    expect(find.text('PCM'), findsOneWidget);
    expect(frame.isActive, isTrue);
    expect(frame.magnitude.length, greaterThan(8));
    expect(frame.normalized.length, frame.magnitude.length);
    expect(frame.rms, isNotEmpty);
    expect(frame.peak, isNotEmpty);
    expect(frame.decay, isNotEmpty);
    expect(frame.rmsNormalized, greaterThan(0));
    expect(frame.peakNormalized, greaterThan(0));
    expect(frame.beatStrength, inInclusiveRange(0, 1));
    expect(frame.pcm, isNotEmpty);
    expect(frame.pcm.every((value) => value >= -1 && value <= 1), isTrue);
    expect(frame.waveform, isNotEmpty);
    expect(frame.waveform.length, frame.pcm.length);
    expect(frame.waveform.every((value) => value >= -1 && value <= 1), isTrue);
    expect(frame.normalized.any((value) => value > 0), isTrue);
    expect(
      frame.normalized.map((value) => (value * 100).round()).toSet().length,
      greaterThan(1),
    );

    await player.stop();
    final stoppedFrame = await _waitForVisualizationIdle();
    expect(stoppedFrame.isActive, isFalse);
  });

  testWidgets('resumes playback after pause without requiring seek', (
    tester,
  ) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    await player.setPlaylist(
      inputs: const ['http://127.0.0.1:8765/generated/long-30s.wav'],
      startIndex: 0,
    );
    await player.play();
    var state = await _waitForPlaybackActivity();
    final pausedPosition = state.positionMs;

    await player.pause();
    await Future<void>.delayed(const Duration(milliseconds: 300));
    state = await player.getState();
    expect(state.lastError, isEmpty);
    expect(state.isPlaying, isFalse);

    await player.play();
    state = await _waitForPlaybackActivity(afterMs: pausedPosition);
    final frame = await _waitForVisualizationActivity(tester);
    expect(state.lastError, isEmpty);
    expect(state.isPlaying, isTrue);
    expect(frame.isActive, isTrue);
    expect(frame.normalized.any((value) => value > 0), isTrue);

    await player.stop();
  });
}

Future<player.VisualizationFrame> _waitForVisualizationActivity(
  WidgetTester tester,
) async {
  var frame = await player.getVisualizationFrame();
  final samples = <String>[];
  final deadline = DateTime.now().add(const Duration(seconds: 6));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 100));
    await tester.pump(const Duration(milliseconds: 100));
    frame = await player.getVisualizationFrame();
    samples.add(_visualizationSample(frame));
    if (frame.isActive && frame.normalized.any((value) => value > 0)) {
      break;
    }
  }
  expect(
    frame.isActive && frame.normalized.any((value) => value > 0),
    isTrue,
    reason: 'visualization did not become active. Samples: $samples',
  );
  return frame;
}

Future<player.VisualizationFrame> _waitForVisualizationIdle() async {
  var frame = await player.getVisualizationFrame();
  final deadline = DateTime.now().add(const Duration(seconds: 2));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 100));
    frame = await player.getVisualizationFrame();
    if (!frame.isActive) {
      break;
    }
  }
  return frame;
}

Future<player.PlaybackState> _waitForPlaybackActivity({int afterMs = 0}) async {
  var state = await player.getState();
  final samples = <String>[];
  final deadline = DateTime.now().add(const Duration(seconds: 12));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 250));
    state = await player.getState();
    samples.add(_stateSample(state));
    if (state.lastError.isNotEmpty) {
      fail(state.lastError);
    }
    if (_hasPlaybackActivity(state, afterMs: afterMs)) {
      break;
    }
  }

  expect(state.lastError, isEmpty);
  expect(
    _hasPlaybackActivity(state, afterMs: afterMs),
    isTrue,
    reason: 'playback did not report activity. Samples: $samples',
  );
  return state;
}

bool _hasPlaybackActivity(player.PlaybackState state, {int afterMs = 0}) {
  return state.isPlaying || state.positionMs > afterMs || state.durationMs > 0;
}

String _stateSample(player.PlaybackState state) {
  return 'uri=${state.currentUri} index=${state.currentIndex} '
      'playing=${state.isPlaying} buffering=${state.isBuffering} '
      'position=${state.positionMs} duration=${state.durationMs} '
      'buffer=${state.bufferingPercent} error=${state.lastError}';
}

String _visualizationSample(player.VisualizationFrame frame) {
  return 'active=${frame.isActive} bands=${frame.normalized.length} '
      'spectrum=${frame.normalized.any((value) => value > 0)} '
      'rms=${frame.rmsNormalized} peak=${frame.peakNormalized} '
      'pcm=${frame.pcm.length}';
}

String _bundledToneUri() {
  return Platform.isIOS
      ? 'asset:///test-assets/tone.wav'
      : 'test-assets/tone.wav';
}
