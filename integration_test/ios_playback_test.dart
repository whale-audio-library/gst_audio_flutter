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
    await Future<void>.delayed(const Duration(seconds: 1));
  });

  testWidgets('plays bundled WAV through Rust GStreamer on iOS', (
    tester,
  ) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    final result = await _playAndWait(const ['asset:///test-assets/tone.wav']);
    final state = result.state;
    expect(state.currentUri, startsWith('file://'));
    expect(result.visualization.isActive, isTrue);
    expect(result.visualization.magnitude.length, greaterThan(8));
    expect(result.visualization.rms, isNotEmpty);
    expect(result.visualization.peak, isNotEmpty);
    expect(result.visualization.decay, isNotEmpty);
    expect(result.visualization.rmsNormalized, greaterThan(0));
    expect(result.visualization.peakNormalized, greaterThan(0));
    expect(result.visualization.beatStrength, inInclusiveRange(0, 1));
    expect(result.visualization.pcm, isNotEmpty);
    expect(
      result.visualization.pcm.every((value) => value >= -1 && value <= 1),
      isTrue,
    );
    expect(result.visualization.waveform, isNotEmpty);
    expect(
      result.visualization.waveform.length,
      result.visualization.pcm.length,
    );
    expect(
      result.visualization.waveform.every((value) => value >= -1 && value <= 1),
      isTrue,
    );
    expect(
      result.visualization.normalized.length,
      result.visualization.magnitude.length,
    );
    expect(result.visualization.normalized.any((value) => value > 0), isTrue);
  });

  testWidgets('plays HTTP WAV through Rust GStreamer on iOS', (tester) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    const uri = 'http://127.0.0.1:8765/generated/long-30s.wav';
    final result = await _playAndWait(const [uri]);
    final state = result.state;
    expect(state.currentUri, uri);
    expect(state.bufferingPercent, inInclusiveRange(0, 100));
    expect(state.isBuffering, isFalse);
    expect(result.visualization.isActive, isTrue);
    expect(result.visualization.magnitude.length, greaterThan(8));
    expect(result.visualization.rms, isNotEmpty);
    expect(result.visualization.peak, isNotEmpty);
    expect(result.visualization.decay, isNotEmpty);
    expect(result.visualization.rmsNormalized, greaterThan(0));
    expect(result.visualization.peakNormalized, greaterThan(0));
    expect(result.visualization.beatStrength, inInclusiveRange(0, 1));
    expect(result.visualization.pcm, isNotEmpty);
    expect(
      result.visualization.pcm.every((value) => value >= -1 && value <= 1),
      isTrue,
    );
    expect(result.visualization.waveform, isNotEmpty);
    expect(
      result.visualization.waveform.length,
      result.visualization.pcm.length,
    );
    expect(
      result.visualization.waveform.every((value) => value >= -1 && value <= 1),
      isTrue,
    );
    expect(
      result.visualization.normalized.length,
      result.visualization.magnitude.length,
    );
    expect(result.visualization.normalized.any((value) => value > 0), isTrue);
  });

  testWidgets('handles pause resume next and previous controls on iOS', (
    tester,
  ) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    await player.setPlaylist(
      inputs: const [
        'asset:///test-assets/tone.wav',
        'http://127.0.0.1:8765/generated/long-30s.wav',
      ],
      startIndex: 0,
    );

    var state = await player.play();
    state = await _waitForPlaybackActivity(state);
    expect(state.currentIndex, 0);
    expect(state.currentUri, startsWith('file://'));

    await player.pause();
    await Future<void>.delayed(const Duration(milliseconds: 300));
    state = await player.getState();
    expect(state.lastError, isEmpty);
    expect(state.isPlaying, isFalse);

    state = await player.play();
    state = await _waitForPlaybackActivity(state);
    expect(state.currentIndex, 0);

    state = await player.next();
    state = await _waitForPlaybackActivity(state);
    expect(state.currentIndex, 1);
    expect(state.currentUri, 'http://127.0.0.1:8765/generated/long-30s.wav');

    state = await player.previous();
    state = await _waitForPlaybackActivity(state);
    expect(state.currentIndex, 0);
    expect(state.currentUri, startsWith('file://'));

    await player.stop();
    await Future<void>.delayed(const Duration(seconds: 2));
  });
}

class _PlaybackResult {
  const _PlaybackResult({required this.state, required this.visualization});

  final player.PlaybackState state;
  final player.VisualizationFrame visualization;
}

Future<_PlaybackResult> _playAndWait(List<String> inputs) async {
  await player.setPlaylist(inputs: inputs, startIndex: 0);
  await player.play();

  final state = await _waitForPlaybackActivity(await player.getState());
  final visualization = await _waitForVisualizationActivity();
  await player.stop();
  await Future<void>.delayed(const Duration(seconds: 2));
  return _PlaybackResult(state: state, visualization: visualization);
}

Future<player.VisualizationFrame> _waitForVisualizationActivity() async {
  var frame = await player.getVisualizationFrame();
  final samples = <String>[];
  final deadline = DateTime.now().add(const Duration(seconds: 5));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 100));
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

Future<player.PlaybackState> _waitForPlaybackActivity(
  player.PlaybackState state,
) async {
  final samples = <String>[];
  final deadline = DateTime.now().add(const Duration(seconds: 12));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 250));
    state = await player.getState();
    samples.add(_stateSample(state));
    if (state.lastError.isNotEmpty) {
      fail(state.lastError);
    }
    if (_hasPlaybackActivity(state)) {
      break;
    }
  }

  expect(state.lastError, isEmpty);
  expect(
    _hasPlaybackActivity(state),
    isTrue,
    reason: 'playback did not report activity. Samples: $samples',
  );
  return state;
}

bool _hasPlaybackActivity(player.PlaybackState state) {
  return state.isPlaying || state.positionMs > 0 || state.durationMs > 0;
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
