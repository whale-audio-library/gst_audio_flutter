import 'dart:async';

import 'package:audio_service/audio_service.dart' as audio_service;
import 'package:audio_session/audio_session.dart';
import 'package:flutter/foundation.dart';
import 'package:gst_audio_flutter/src/rust/api/player.dart' as player;

Future<audio_service.AudioHandler?> initializeAudioSystem() async {
  if (kIsWeb ||
      (defaultTargetPlatform != TargetPlatform.android &&
          defaultTargetPlatform != TargetPlatform.iOS &&
          defaultTargetPlatform != TargetPlatform.macOS)) {
    return null;
  }

  final audioHandler = await audio_service.AudioService.init(
    builder: GstAudioHandler.new,
    config: const audio_service.AudioServiceConfig(
      androidNotificationChannelId: 'com.example.gst_audio_flutter.audio',
      androidNotificationChannelName: 'Audio playback',
      androidNotificationChannelDescription:
          'Playback controls for GStreamer audio.',
      androidStopForegroundOnPause: false,
    ),
  );

  final session = await AudioSession.instance;
  await session.configure(const AudioSessionConfiguration.music());
  return audioHandler;
}

class GstAudioHandler extends audio_service.BaseAudioHandler
    with audio_service.SeekHandler {
  GstAudioHandler() {
    _stateTimer = Timer.periodic(
      const Duration(milliseconds: 500),
      (_) => refreshFromPlayer(),
    );
    refreshFromPlayer();
  }

  Timer? _stateTimer;

  Future<void> refreshFromPlayer() async {
    try {
      final state = await player.getState();
      _broadcastState(state);
    } catch (error) {
      playbackState.add(
        playbackState.value.copyWith(
          processingState: audio_service.AudioProcessingState.error,
          playing: false,
          errorMessage: error.toString(),
        ),
      );
    }
  }

  @override
  Future<void> play() async {
    final state = await player.play();
    _broadcastState(state);
  }

  @override
  Future<void> pause() async {
    final state = await player.pause();
    _broadcastState(state);
  }

  @override
  Future<void> stop() async {
    final state = await player.stop();
    _broadcastState(state);
    await super.stop();
  }

  @override
  Future<void> skipToNext() async {
    final state = await player.next();
    _broadcastState(state);
  }

  @override
  Future<void> skipToPrevious() async {
    final state = await player.previous();
    _broadcastState(state);
  }

  @override
  Future<void> skipToQueueItem(int index) async {
    final state = await player.playIndex(index: index);
    _broadcastState(state);
  }

  @override
  Future<void> seek(Duration position) async {
    final state = await player.seekMs(positionMs: position.inMilliseconds);
    _broadcastState(state);
  }

  @override
  Future<void> click([
    audio_service.MediaButton button = audio_service.MediaButton.media,
  ]) async {
    if (button == audio_service.MediaButton.media) {
      final state = await player.togglePlayPause();
      _broadcastState(state);
      return;
    }
    await super.click(button);
  }

  void _broadcastState(player.PlaybackState state) {
    final items = state.playlist.map(_mediaItemForTrack).toList();
    queue.add(items);

    final currentIndex = state.currentIndex >= 0 ? state.currentIndex : null;
    final currentItem = currentIndex != null && currentIndex < items.length
        ? items[currentIndex]
        : null;
    mediaItem.add(currentItem);

    playbackState.add(
      audio_service.PlaybackState(
        controls: [
          audio_service.MediaControl.skipToPrevious,
          if (state.isPlaying)
            audio_service.MediaControl.pause
          else
            audio_service.MediaControl.play,
          audio_service.MediaControl.stop,
          audio_service.MediaControl.skipToNext,
        ],
        androidCompactActionIndices: const [0, 1, 3],
        systemActions: const {
          audio_service.MediaAction.seek,
          audio_service.MediaAction.seekForward,
          audio_service.MediaAction.seekBackward,
        },
        processingState: _processingState(state),
        playing: state.isPlaying,
        updatePosition: _durationFromMs(state.positionMs),
        bufferedPosition: _bufferedPosition(state),
        speed: state.speed,
        queueIndex: currentIndex,
        errorMessage: state.lastError.isEmpty ? null : state.lastError,
      ),
    );
  }

  audio_service.AudioProcessingState _processingState(
    player.PlaybackState state,
  ) {
    if (state.lastError.isNotEmpty) {
      return audio_service.AudioProcessingState.error;
    }
    if (state.isBuffering) {
      return audio_service.AudioProcessingState.buffering;
    }
    if (state.currentUri.isEmpty) {
      return audio_service.AudioProcessingState.idle;
    }
    return audio_service.AudioProcessingState.ready;
  }

  audio_service.MediaItem _mediaItemForTrack(player.Track track) {
    return audio_service.MediaItem(
      id: track.uri,
      title: track.title.isEmpty ? track.uri : track.title,
    );
  }

  Duration _bufferedPosition(player.PlaybackState state) {
    final duration = _durationFromMs(state.durationMs);
    if (duration == Duration.zero) {
      return Duration.zero;
    }
    final percent = state.bufferingPercent.clamp(0, 100);
    return Duration(
      milliseconds: (duration.inMilliseconds * percent / 100).round(),
    );
  }

  Duration _durationFromMs(int milliseconds) {
    if (milliseconds <= 0) {
      return Duration.zero;
    }
    return Duration(milliseconds: milliseconds);
  }

  @override
  Future<dynamic> customAction(
    String name, [
    Map<String, dynamic>? extras,
  ]) async {
    if (name == 'refresh') {
      await refreshFromPlayer();
      return null;
    }
    return super.customAction(name, extras);
  }

  void dispose() {
    _stateTimer?.cancel();
  }
}
