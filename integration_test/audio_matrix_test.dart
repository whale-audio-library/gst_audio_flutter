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

  testWidgets('plays supported HTTP audio formats', (tester) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    const cases = [
      _AudioCase(
        label: 'wav-small',
        uri: 'http://127.0.0.1:8765/tone.wav',
        minDurationMs: 500,
      ),
      _AudioCase(
        label: 'flac-small',
        uri: 'http://127.0.0.1:8765/formats/tone.flac',
        minDurationMs: 500,
      ),
      _AudioCase(
        label: 'ogg-vorbis-small',
        uri: 'http://127.0.0.1:8765/formats/tone-vorbis.ogg',
        minDurationMs: 500,
      ),
      _AudioCase(
        label: 'ogg-opus-small',
        uri: 'http://127.0.0.1:8765/formats/tone-opus.ogg',
        minDurationMs: 500,
      ),
    ];

    for (final testCase in cases) {
      final result = await _playAndObserve(testCase);
      final state = result.state;
      expect(state.currentUri, testCase.uri, reason: testCase.label);
      expectTrue(
        result.sawPlaybackActivity,
        '${testCase.label} should start decoding without stalling',
      );
      if (state.durationMs > 0) {
        expectAtLeast(state.durationMs, testCase.minDurationMs, testCase.label);
      }
      expectPercent(state.bufferingPercent, testCase.label);
      await player.stop();
    }
  });

  testWidgets('starts larger and adverse HTTP streams without corrupting state', (
    tester,
  ) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    const cases = [
      _AudioCase(
        label: 'wav-long-30s',
        uri: 'http://127.0.0.1:8765/generated/long-30s.wav',
        minDurationMs: 25000,
      ),
      _AudioCase(
        label: 'wav-large-60s-stereo',
        uri: 'http://127.0.0.1:8765/generated/large-60s-stereo.wav',
        minDurationMs: 50000,
      ),
      _AudioCase(
        label: 'wav-large-slow-http',
        uri:
            'http://127.0.0.1:8765/slow/generated/large-60s-stereo.wav?chunk=4096&delay=0.02',
        minDurationMs: 50000,
        expectPartialBuffer: true,
      ),
    ];

    for (final testCase in cases) {
      final result = await _playAndObserve(testCase);
      final state = result.state;
      expect(state.currentUri, testCase.uri, reason: testCase.label);
      expect(state.currentIndex, 0, reason: testCase.label);
      expect(state.lastError, isEmpty, reason: testCase.label);
      expectPercent(state.bufferingPercent, testCase.label);
      if (state.durationMs > 0) {
        expectAtLeast(state.durationMs, testCase.minDurationMs, testCase.label);
      }
      if (testCase.expectPartialBuffer) {
        expectTrue(
          result.sawPartialBuffer,
          '${testCase.label} should expose partial HTTP buffering. Samples: ${result.bufferSamples}',
        );
      }
      await player.stop();
    }
  });

  testWidgets('recovers after interrupted HTTP transfer attempt', (
    tester,
  ) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    await player.setPlaylist(
      inputs: const [
        'http://127.0.0.1:8765/drop/generated/large-60s-stereo.wav',
      ],
      startIndex: 0,
    );
    await player.play();

    var interruptedState = await player.getState();
    final deadline = DateTime.now().add(const Duration(seconds: 3));
    while (DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(const Duration(milliseconds: 250));
      interruptedState = await player.getState();
      if (interruptedState.lastError.isNotEmpty ||
          interruptedState.positionMs > 0) {
        break;
      }
    }
    expectTrue(
      interruptedState.currentUri.contains('/drop/'),
      'interrupted transfer should remain on drop fixture',
    );
    expectPercent(interruptedState.bufferingPercent, 'interrupted transfer');
    await player.stop();

    final stoppedState = await player.getState();
    expectTrue(!stoppedState.isBuffering, 'stopped player should not buffer');

    const recoveryCase = _AudioCase(
      label: 'recovery-wav',
      uri: 'http://127.0.0.1:8765/tone.wav',
      minDurationMs: 500,
    );
    final recoveredState = await _playAndWaitForProgress(recoveryCase);
    expect(recoveredState.lastError, isEmpty);
    expectTrue(
      _hasPlaybackActivity(recoveredState, recoveryCase.uri),
      'recovery fixture should report playback activity: '
      '${_stateSample(recoveredState)}',
    );
  });

  testWidgets('survives rapid playlist switches across HTTP formats', (
    tester,
  ) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    const inputs = [
      'http://127.0.0.1:8765/tone.wav',
      'http://127.0.0.1:8765/formats/tone.flac',
      'http://127.0.0.1:8765/formats/tone-vorbis.ogg',
      'http://127.0.0.1:8765/formats/tone-opus.ogg',
      'http://127.0.0.1:8765/generated/long-30s.wav',
    ];

    await player.setPlaylist(inputs: inputs, startIndex: 0);
    await player.play();

    for (var index = 1; index < inputs.length; index += 1) {
      await Future<void>.delayed(const Duration(milliseconds: 180));
      final state = await player.playIndex(index: index);
      expect(state.lastError, isEmpty);
      expect(state.currentIndex, index);
    }

    final finalState = await _observeCurrentUri(inputs.last);
    expect(finalState.currentUri, inputs.last);
    expect(finalState.lastError, isEmpty);
  });
}

class _AudioCase {
  const _AudioCase({
    required this.label,
    required this.uri,
    required this.minDurationMs,
    this.expectPartialBuffer = false,
  });

  final String label;
  final String uri;
  final int minDurationMs;
  final bool expectPartialBuffer;
}

extension on player.PlaybackState {
  bool get sawPartialBuffer => isBuffering || bufferingPercent < 100;
}

class _ObservedState {
  const _ObservedState({
    required this.state,
    required this.sawPartialBuffer,
    required this.sawPlaybackActivity,
    required this.bufferSamples,
  });

  final player.PlaybackState state;
  final bool sawPartialBuffer;
  final bool sawPlaybackActivity;
  final List<int> bufferSamples;
}

Future<player.PlaybackState> _playAndWaitForProgress(
  _AudioCase testCase,
) async {
  await player.setPlaylist(inputs: [testCase.uri], startIndex: 0);
  await player.play();
  return _waitForPlaybackProgress(await player.getState());
}

Future<_ObservedState> _playAndObserve(_AudioCase testCase) async {
  await player.setPlaylist(inputs: [testCase.uri], startIndex: 0);
  await player.play();

  var bestState = await player.getState();
  var sawPartialBuffer =
      bestState.bufferingPercent > 0 && bestState.bufferingPercent < 100;
  var sawPlaybackActivity = _hasPlaybackActivity(bestState, testCase.uri);
  final bufferSamples = <int>[];
  final deadline = DateTime.now().add(const Duration(seconds: 6));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 250));
    final state = await player.getState();
    if (state.lastError.isNotEmpty) {
      fail('${testCase.label}: ${state.lastError}');
    }
    if (state.currentUri == testCase.uri) {
      bufferSamples.add(state.bufferingPercent);
      bestState = _moreInformativeState(bestState, state);
      sawPartialBuffer =
          sawPartialBuffer ||
          state.bufferingPercent > 0 && state.bufferingPercent < 100;
      sawPlaybackActivity =
          sawPlaybackActivity || _hasPlaybackActivity(state, testCase.uri);
    }
    if (state.durationMs >= testCase.minDurationMs &&
        (!testCase.expectPartialBuffer || sawPartialBuffer)) {
      break;
    }
  }

  return _ObservedState(
    state: bestState,
    sawPartialBuffer: sawPartialBuffer,
    sawPlaybackActivity: sawPlaybackActivity,
    bufferSamples: bufferSamples,
  );
}

Future<player.PlaybackState> _observeCurrentUri(String uri) async {
  var bestState = await player.getState();
  final deadline = DateTime.now().add(const Duration(seconds: 6));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 250));
    final state = await player.getState();
    if (state.lastError.isNotEmpty) {
      fail(state.lastError);
    }
    if (state.currentUri == uri) {
      bestState = _moreInformativeState(bestState, state);
    }
  }
  return bestState;
}

Future<player.PlaybackState> _waitForPlaybackProgress(
  player.PlaybackState state,
) async {
  final deadline = DateTime.now().add(const Duration(seconds: 12));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 250));
    state = await player.getState();
    if (state.lastError.isNotEmpty) {
      fail(state.lastError);
    }
    if ((state.positionMs > 0 || state.isPlaying) && state.durationMs > 0) {
      break;
    }
  }

  expect(state.lastError, isEmpty);
  expectTrue(
    state.positionMs > 0 || state.isPlaying,
    'playback should either advance or report playing',
  );
  expectTrue(state.durationMs > 0, 'duration should be known');
  return state;
}

void expectTrue(bool value, String reason) {
  expect(value, true, reason: reason);
}

void expectAtLeast(int actual, int minimum, String reason) {
  expect(
    actual >= minimum,
    true,
    reason: '$reason: expected $actual to be >= $minimum',
  );
}

void expectPercent(int value, String reason) {
  expect(
    value >= 0 && value <= 100,
    true,
    reason: '$reason: expected buffering percent in 0..100, got $value',
  );
}

player.PlaybackState _moreInformativeState(
  player.PlaybackState current,
  player.PlaybackState candidate,
) {
  if (candidate.durationMs > current.durationMs) {
    return candidate;
  }
  if (candidate.positionMs > current.positionMs) {
    return candidate;
  }
  if (candidate.sawPartialBuffer && !current.sawPartialBuffer) {
    return candidate;
  }
  if (current.currentUri.isEmpty && candidate.currentUri.isNotEmpty) {
    return candidate;
  }
  return current;
}

bool _hasPlaybackActivity(player.PlaybackState state, String uri) {
  return state.currentUri == uri &&
      (state.isPlaying ||
          state.positionMs > 0 ||
          state.durationMs > 0 ||
          state.sawPartialBuffer);
}

String _stateSample(player.PlaybackState state) {
  return 'uri=${state.currentUri} index=${state.currentIndex} '
      'playing=${state.isPlaying} buffering=${state.isBuffering} '
      'position=${state.positionMs} duration=${state.durationMs} '
      'buffer=${state.bufferingPercent} error=${state.lastError}';
}
