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
    await player.shutdownPlayer();
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
}

Future<player.PlaybackState> _playAndWait(List<String> inputs) async {
  await player.setPlaylist(inputs: inputs, startIndex: 0);
  await player.play();

  player.PlaybackState state = await player.getState();
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
  await player.stop();
  return state;
}
