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
    expect(state.positionMs, greaterThan(0));
  });
}
