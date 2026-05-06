import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gst_audio_flutter/main.dart';
import 'package:gst_audio_flutter/src/audio_system_integration.dart';
import 'package:gst_audio_flutter/src/rust/api/player.dart' as player;
import 'package:gst_audio_flutter/src/rust/frb_generated.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() async {
    await RustLib.init();
    await initializeAudioSystem();
  });

  tearDownAll(() async {
    await player.stop();
    await player.shutdownPlayer();
  });

  testWidgets('handles Android native media controls and foreground audio', (
    tester,
  ) async {
    if (defaultTargetPlatform != TargetPlatform.android) {
      return;
    }

    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();

    await player.setPlaylist(
      inputs: const [
        'http://127.0.0.1:8765/generated/long-30s.wav',
        'http://127.0.0.1:8765/generated/long-30s.wav',
      ],
      startIndex: 0,
    );
    await player.play();

    var state = await _waitForPlaying(index: 0);
    debugPrint('ANDROID_NATIVE_AUDIO_STAGE:playing');
    state = await _waitForPauseAfterNativeEvent(previous: state);
    expect(state.currentIndex, 0);

    debugPrint('ANDROID_NATIVE_AUDIO_STAGE:paused_by_media_key');
    state = await _waitForPlaying(index: 0);
    expect(state.currentIndex, 0);

    debugPrint('ANDROID_NATIVE_AUDIO_STAGE:resumed_by_media_key');
    state = await _waitForIndex(1);
    expect(state.isPlaying, isTrue);

    debugPrint('ANDROID_NATIVE_AUDIO_STAGE:next_by_media_key');
    state = await _waitForIndex(0);
    expect(state.isPlaying, isTrue);

    debugPrint('ANDROID_NATIVE_AUDIO_STAGE:previous_by_media_key');
    await player.stop();
  });
}

Future<player.PlaybackState> _waitForPlaying({required int index}) async {
  var state = await player.getState();
  final deadline = DateTime.now().add(const Duration(seconds: 10));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 200));
    state = await player.getState();
    if (state.lastError.isNotEmpty) {
      fail(state.lastError);
    }
    if (state.isPlaying &&
        state.currentIndex == index &&
        state.positionMs > 0) {
      return state;
    }
  }

  fail('Playback did not start. Last state: ${_stateSample(state)}');
}

Future<player.PlaybackState> _waitForPauseAfterNativeEvent({
  required player.PlaybackState previous,
}) async {
  var state = await player.getState();
  final deadline = DateTime.now().add(const Duration(seconds: 10));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 200));
    state = await player.getState();
    if (state.lastError.isNotEmpty) {
      fail(state.lastError);
    }
    if (!state.isPlaying && state.currentIndex == previous.currentIndex) {
      return state;
    }
  }

  fail(
    'Native media pause was not observed. Last state: ${_stateSample(state)}',
  );
}

Future<player.PlaybackState> _waitForIndex(int index) async {
  var state = await player.getState();
  final deadline = DateTime.now().add(const Duration(seconds: 12));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 200));
    state = await player.getState();
    if (state.lastError.isNotEmpty) {
      fail(state.lastError);
    }
    if (state.currentIndex == index && state.isPlaying) {
      return state;
    }
  }

  fail(
    'Native media skip to $index was not observed. Last state: ${_stateSample(state)}',
  );
}

String _stateSample(player.PlaybackState state) {
  return 'index=${state.currentIndex} uri=${state.currentUri} '
      'playing=${state.isPlaying} buffering=${state.isBuffering} '
      'position=${state.positionMs} error=${state.lastError}';
}
