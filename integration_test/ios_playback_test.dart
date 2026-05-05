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

    final state = await _playAndWait(const ['asset:///test-assets/tone.wav']);
    expect(state.currentUri, startsWith('file://'));
  });

  testWidgets('plays HTTP WAV through Rust GStreamer on iOS', (tester) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    final state = await _playAndWait(
      const ['http://127.0.0.1:8765/tone.wav'],
    );
    expect(state.currentUri, 'http://127.0.0.1:8765/tone.wav');
  });

  testWidgets('handles pause resume next and previous controls on iOS', (
    tester,
  ) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    await player.setPlaylist(
      inputs: const [
        'asset:///test-assets/tone.wav',
        'http://127.0.0.1:8765/tone.wav',
      ],
      startIndex: 0,
    );

    var state = await player.play();
    state = await _waitForPlaybackPosition(state);
    expect(state.currentIndex, 0);
    expect(state.currentUri, startsWith('file://'));

    await player.pause();
    await Future<void>.delayed(const Duration(milliseconds: 300));
    state = await player.getState();
    expect(state.lastError, isEmpty);
    expect(state.isPlaying, isFalse);

    state = await player.play();
    state = await _waitForPlaybackPosition(state);
    expect(state.currentIndex, 0);

    state = await player.next();
    state = await _waitForPlaybackPosition(state);
    expect(state.currentIndex, 1);
    expect(state.currentUri, 'http://127.0.0.1:8765/tone.wav');

    state = await player.previous();
    state = await _waitForPlaybackPosition(state);
    expect(state.currentIndex, 0);
    expect(state.currentUri, startsWith('file://'));

    await player.stop();
    await Future<void>.delayed(const Duration(seconds: 2));
  });
}

Future<player.PlaybackState> _playAndWait(List<String> inputs) async {
  await player.setPlaylist(inputs: inputs, startIndex: 0);
  await player.play();

  var state = await _waitForPlaybackPosition(await player.getState());
  state = await _waitForStablePlayback(state);
  await player.stop();
  await Future<void>.delayed(const Duration(seconds: 2));
  return state;
}

Future<player.PlaybackState> _waitForPlaybackPosition(
  player.PlaybackState state,
) async {
  final deadline = DateTime.now().add(const Duration(seconds: 8));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 250));
    state = await player.getState();
    if (state.lastError.isNotEmpty) {
      fail(state.lastError);
    }
    if (state.positionMs > 0) {
      break;
    }
  }

  expect(state.lastError, isEmpty);
  expect(state.positionMs, greaterThan(0));
  return state;
}

Future<player.PlaybackState> _waitForStablePlayback(
  player.PlaybackState state,
) async {
  final deadline = DateTime.now().add(const Duration(seconds: 2));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 250));
    state = await player.getState();
    if (state.lastError.isNotEmpty) {
      fail(state.lastError);
    }
  }
  return state;
}
