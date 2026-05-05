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

  testWidgets('shows HTTP buffering progress in the Flutter player', (
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
      await tester.pump(const Duration(milliseconds: 250));
      state = await player.getState();
      if (state.lastError.isNotEmpty) {
        fail(state.lastError);
      }
      if (find
          .byKey(const ValueKey('http-buffer-progress'))
          .evaluate()
          .isNotEmpty) {
        break;
      }
    }

    expect(state.lastError, isEmpty);
    expect(state.currentUri, 'http://127.0.0.1:8765/tone.wav');
    expect(state.bufferingPercent, inInclusiveRange(0, 100));
    final bufferProgress = find.byKey(const ValueKey('http-buffer-progress'));
    expect(bufferProgress, findsOneWidget);
    expect(
      find.descendant(
        of: bufferProgress,
        matching: find.byType(LinearProgressIndicator),
      ),
      findsOneWidget,
    );
  });

  testWidgets('reports HTTP buffering as downloaded media progress', (
    tester,
  ) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    const uri =
        'http://127.0.0.1:8765/slow/generated/large-60s-stereo.wav?chunk=4096&delay=0.02';
    await player.setPlaylist(inputs: const [uri], startIndex: 0);
    await player.play();

    final samples = <int>[];
    player.PlaybackState state = await player.getState();
    final deadline = DateTime.now().add(const Duration(seconds: 8));
    while (DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(const Duration(milliseconds: 500));
      await tester.pump(const Duration(milliseconds: 500));
      state = await player.getState();
      if (state.lastError.isNotEmpty) {
        fail(state.lastError);
      }
      if (state.currentUri == uri) {
        samples.add(state.bufferingPercent);
      }
      if (samples.length >= 4 &&
          samples.any((percent) => percent > 0 && percent < 100)) {
        break;
      }
    }

    expect(state.currentUri, uri);
    expect(state.bufferingPercent, inInclusiveRange(0, 100));
    expect(
      samples.any((percent) => percent > 0 && percent < 100),
      isTrue,
      reason:
          'slow HTTP download should expose partial media progress, not jump directly to 100%. Samples: $samples',
    );
    expect(
      samples.last,
      lessThan(100),
      reason:
          'slow fixture should still be downloading during the sample window. Samples: $samples',
    );

    final bufferProgress = find.byKey(const ValueKey('http-buffer-progress'));
    expect(bufferProgress, findsOneWidget);
  });
}
