import 'package:flutter/foundation.dart';
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

  testWidgets('plays an HTTP WAV stream through Rust GStreamer', (
    tester,
  ) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    await player.setPlaylist(
      inputs: const ['http://127.0.0.1:8765/tone.wav'],
      startIndex: 0,
    );
    await player.play();

    player.PlaybackState state = await player.getState();
    final deadline = DateTime.now().add(const Duration(seconds: 8));
    while (DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(const Duration(milliseconds: 250));
      state = await player.getState();
      if (state.lastError.isNotEmpty) {
        fail(state.lastError);
      }
      if (state.positionMs > 0 || state.isPlaying) {
        break;
      }
    }

    expect(state.lastError, isEmpty);
    expect(state.currentUri, 'http://127.0.0.1:8765/tone.wav');
    expect(state.bufferingPercent, inInclusiveRange(0, 100));
    expect(state.isBuffering, isFalse);
    expect(state.positionMs, greaterThan(0));
    final visualization = await _waitForVisualizationActivity();
    expect(visualization.isActive, isTrue);
    expect(visualization.magnitude.length, greaterThan(8));
    expect(visualization.normalized.length, visualization.magnitude.length);
    expect(visualization.rms, isNotEmpty);
    expect(visualization.peak, isNotEmpty);
    expect(visualization.decay, isNotEmpty);
    expect(visualization.rmsNormalized, greaterThan(0));
    expect(visualization.peakNormalized, greaterThan(0));
    expect(visualization.beatStrength, inInclusiveRange(0, 1));
    expect(visualization.pcm, isNotEmpty);
    expect(
      visualization.pcm.every((value) => value >= -1 && value <= 1),
      isTrue,
    );
    expect(visualization.waveform, isNotEmpty);
    expect(visualization.waveform.length, visualization.pcm.length);
    expect(
      visualization.waveform.every((value) => value >= -1 && value <= 1),
      isTrue,
    );
    expect(visualization.normalized.any((value) => value > 0), isTrue);

    await player.stop();
  });

  testWidgets('resumes from the Flutter play button after pause without seek', (
    tester,
  ) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    const uri = 'http://127.0.0.1:8765/generated/long-30s.wav';
    await player.setPlaylist(inputs: const [uri], startIndex: 0);
    await player.play();

    var state = await _waitForPlaybackPosition();
    expect(state.currentUri, uri);
    await _waitForTooltip(tester, 'Pause');
    final pausedPosition = state.positionMs;

    await _tapPlayPause(tester);
    await tester.pump();
    state = await _waitForPaused();
    expect(state.lastError, isEmpty);
    expect(state.isPlaying, isFalse);
    await _waitForTooltip(tester, 'Play');

    await _tapPlayPause(tester);
    await tester.pump();
    state = await _waitForPlaybackPosition(afterMs: pausedPosition);
    expect(state.lastError, isEmpty);
    expect(state.isPlaying, isTrue);
    expect(state.positionMs, greaterThan(pausedPosition));

    await player.stop();
  });
}

Future<void> _tapPlayPause(WidgetTester tester) async {
  final finder = find.byKey(const ValueKey('play-pause-button'));
  await tester.ensureVisible(finder);
  await tester.pump(const Duration(milliseconds: 300));
  await tester.tap(finder);
}

Future<player.VisualizationFrame> _waitForVisualizationActivity() async {
  var frame = await player.getVisualizationFrame();
  final deadline = DateTime.now().add(const Duration(seconds: 5));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 100));
    frame = await player.getVisualizationFrame();
    if (frame.isActive && frame.normalized.any((value) => value > 0)) {
      break;
    }
  }
  return frame;
}

Future<player.PlaybackState> _waitForPlaybackPosition({int afterMs = 0}) async {
  var state = await player.getState();
  final deadline = DateTime.now().add(const Duration(seconds: 6));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 150));
    state = await player.getState();
    if (state.lastError.isNotEmpty) {
      fail(state.lastError);
    }
    if (state.positionMs > afterMs) {
      break;
    }
  }

  expect(state.lastError, isEmpty);
  expect(state.positionMs, greaterThan(afterMs));
  return state;
}

Future<player.PlaybackState> _waitForPaused() async {
  var state = await player.getState();
  final deadline = DateTime.now().add(const Duration(seconds: 5));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 150));
    state = await player.getState();
    if (state.lastError.isNotEmpty) {
      fail(state.lastError);
    }
    if (!state.isPlaying) {
      break;
    }
  }

  expect(state.lastError, isEmpty);
  expect(state.isPlaying, isFalse);
  return state;
}

Future<void> _waitForTooltip(WidgetTester tester, String tooltip) async {
  final finder = find.byTooltip(tooltip);
  final deadline = DateTime.now().add(const Duration(seconds: 5));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 100));
    await tester.pump(const Duration(milliseconds: 100));
    if (finder.evaluate().isNotEmpty) {
      break;
    }
  }

  expect(finder, findsOneWidget);
}
