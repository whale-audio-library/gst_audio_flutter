import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:gst_audio_flutter/src/audio_system_integration.dart';
import 'package:gst_audio_flutter/src/rust/api/player.dart' as player;
import 'package:gst_audio_flutter/src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  await initializeAudioSystem();
  runApp(const AudioPlayerApp());
}

class AudioPlayerApp extends StatelessWidget {
  const AudioPlayerApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      title: 'GStreamer Audio',
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xff26706f),
          brightness: Brightness.light,
        ),
        useMaterial3: true,
        sliderTheme: const SliderThemeData(
          showValueIndicator: ShowValueIndicator.onDrag,
        ),
      ),
      home: const PlayerPage(),
    );
  }
}

class PlayerPage extends StatefulWidget {
  const PlayerPage({super.key});

  @override
  State<PlayerPage> createState() => _PlayerPageState();
}

class _PlayerPageState extends State<PlayerPage> {
  final _queueController = TextEditingController();
  final _newItemController = TextEditingController();
  final _manualSeekController = TextEditingController(text: '0');
  final _queueFocus = FocusNode();

  player.PlaybackState? _state;
  player.VisualizationFrame? _visualizationFrame;
  List<player.AudioOutputDevice> _devices = const [];
  Timer? _pollTimer;
  Timer? _visualizationPollTimer;
  bool _busy = false;
  bool _draggingSeek = false;
  bool _refreshingVisualization = false;
  double _seekMs = 0;
  String _error = '';

  @override
  void initState() {
    super.initState();
    _refreshAll();
    _pollTimer = Timer.periodic(const Duration(milliseconds: 500), (_) {
      if (!_busy && !_draggingSeek) {
        _refreshState();
      }
    });
    _visualizationPollTimer = Timer.periodic(
      const Duration(milliseconds: 100),
      (_) {
        final shouldPoll =
            _state?.isPlaying == true ||
            _state?.isBuffering == true ||
            _visualizationFrame?.isActive == true;
        if (shouldPoll) {
          _refreshVisualizationFrame();
        }
      },
    );
  }

  @override
  void dispose() {
    _pollTimer?.cancel();
    _visualizationPollTimer?.cancel();
    _queueController.dispose();
    _newItemController.dispose();
    _manualSeekController.dispose();
    _queueFocus.dispose();
    super.dispose();
  }

  Future<void> _run(Future<player.PlaybackState> Function() action) async {
    setState(() {
      _busy = true;
      _error = '';
    });
    try {
      final nextState = await action();
      if (!mounted) return;
      setState(() {
        _state = nextState;
        _seekMs = nextState.positionMs.toDouble();
      });
    } catch (error) {
      if (!mounted) return;
      setState(() => _error = error.toString());
    } finally {
      if (mounted) {
        setState(() => _busy = false);
      }
    }
  }

  Future<void> _refreshAll() async {
    await _refreshState();
    try {
      final devices = await player.listOutputDevices();
      if (mounted) {
        setState(() => _devices = devices);
      }
    } catch (error) {
      if (mounted) {
        setState(() => _error = error.toString());
      }
    }
  }

  Future<void> _refreshState() async {
    try {
      final nextState = await player.getState();
      if (!mounted) return;
      setState(() {
        _state = nextState;
        if (!_draggingSeek) {
          _seekMs = nextState.positionMs.toDouble();
        }
      });
    } catch (error) {
      if (mounted) {
        setState(() => _error = error.toString());
      }
    }
  }

  Future<void> _refreshVisualizationFrame() async {
    if (_refreshingVisualization) return;
    _refreshingVisualization = true;
    try {
      final nextFrame = await player.getVisualizationFrame();
      if (!mounted) return;
      setState(() => _visualizationFrame = nextFrame);
    } catch (_) {
      // Playback state owns user-visible errors; visualization should fail soft.
    } finally {
      _refreshingVisualization = false;
    }
  }

  List<String> _queueItems() {
    return _queueController.text
        .split('\n')
        .map((line) => line.trim())
        .where((line) => line.isNotEmpty)
        .toList(growable: false);
  }

  Future<void> _loadQueue({int startIndex = 0}) async {
    await _run(
      () => player.setPlaylist(inputs: _queueItems(), startIndex: startIndex),
    );
  }

  Future<void> _appendItem() async {
    final value = _newItemController.text.trim();
    if (value.isEmpty) return;
    final current = _queueController.text.trimRight();
    _queueController.text = current.isEmpty ? value : '$current\n$value';
    _newItemController.clear();
    await _loadQueue(startIndex: _state?.currentIndex ?? 0);
  }

  String _formatTime(num milliseconds) {
    final value = Duration(
      milliseconds: milliseconds.round().clamp(0, 1 << 62),
    );
    final hours = value.inHours;
    final minutes = value.inMinutes.remainder(60).toString().padLeft(2, '0');
    final seconds = value.inSeconds.remainder(60).toString().padLeft(2, '0');
    if (hours > 0) {
      return '$hours:$minutes:$seconds';
    }
    return '${value.inMinutes}:$seconds';
  }

  @override
  Widget build(BuildContext context) {
    final state = _state;
    final color = Theme.of(context).colorScheme;
    final durationMs = (state?.durationMs ?? 0)
        .toDouble()
        .clamp(0.0, double.infinity)
        .toDouble();
    final positionMs = _seekMs
        .clamp(0.0, durationMs > 0 ? durationMs : _seekMs)
        .toDouble();

    return Scaffold(
      appBar: AppBar(
        title: const Text('GStreamer Audio'),
        actions: [
          IconButton(
            tooltip: 'Refresh',
            onPressed: _refreshAll,
            icon: const Icon(Icons.refresh),
          ),
        ],
      ),
      body: SafeArea(
        child: LayoutBuilder(
          builder: (context, constraints) {
            final wide = constraints.maxWidth >= 900;
            final queuePane = _QueuePane(
              queueController: _queueController,
              newItemController: _newItemController,
              queueFocus: _queueFocus,
              playlist: state?.playlist ?? const [],
              currentIndex: state?.currentIndex ?? -1,
              onLoad: () => _loadQueue(),
              onAppend: _appendItem,
              onPlayIndex: (index) =>
                  _run(() => player.playIndex(index: index)),
            );

            final controlPane = _ControlPane(
              state: state,
              visualizationFrame: _visualizationFrame,
              devices: _devices,
              busy: _busy,
              error: _error.isNotEmpty ? _error : state?.lastError ?? '',
              positionMs: positionMs,
              durationMs: durationMs,
              manualSeekController: _manualSeekController,
              onPlayPause: () => _run(player.togglePlayPause),
              onStop: () => _run(player.stop),
              onPrevious: () => _run(player.previous),
              onNext: () => _run(player.next),
              onSeekStart: (_) => setState(() => _draggingSeek = true),
              onSeekChanged: (value) => setState(() => _seekMs = value),
              onSeekEnd: (value) async {
                setState(() => _draggingSeek = false);
                await _run(() => player.seekMs(positionMs: value.round()));
              },
              onManualSeek: () {
                final value = int.tryParse(_manualSeekController.text.trim());
                if (value != null) {
                  _run(() => player.seekMs(positionMs: value));
                }
              },
              onVolumeChanged: (value) =>
                  _run(() => player.setVolume(volume: value)),
              onMutedChanged: (value) =>
                  _run(() => player.setMuted(muted: value)),
              onFadeIn: () => _run(() => player.fadeIn(durationMs: 1200)),
              onFadeOut: () => _run(() => player.fadeOut(durationMs: 1200)),
              onSpeedChanged: (value) =>
                  _run(() => player.setSpeed(speed: value)),
              onShuffleChanged: (value) =>
                  _run(() => player.setShuffle(enabled: value)),
              onRepeatChanged: (value) =>
                  _run(() => player.setRepeatMode(mode: value)),
              onOutputChanged: (value) =>
                  _run(() => player.setOutputDevice(deviceId: value)),
              timeLabel:
                  '${_formatTime(positionMs)} / ${durationMs > 0 ? _formatTime(durationMs) : '0:00'}',
            );

            if (wide) {
              return Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(width: 360, child: queuePane),
                  VerticalDivider(width: 1, color: color.outlineVariant),
                  Expanded(child: SingleChildScrollView(child: controlPane)),
                ],
              );
            }

            return ListView(
              padding: EdgeInsets.zero,
              children: [
                SizedBox(height: 420, child: queuePane),
                Divider(height: 1, color: color.outlineVariant),
                controlPane,
              ],
            );
          },
        ),
      ),
    );
  }
}

class _QueuePane extends StatelessWidget {
  const _QueuePane({
    required this.queueController,
    required this.newItemController,
    required this.queueFocus,
    required this.playlist,
    required this.currentIndex,
    required this.onLoad,
    required this.onAppend,
    required this.onPlayIndex,
  });

  final TextEditingController queueController;
  final TextEditingController newItemController;
  final FocusNode queueFocus;
  final List<player.Track> playlist;
  final int currentIndex;
  final VoidCallback onLoad;
  final VoidCallback onAppend;
  final ValueChanged<int> onPlayIndex;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              const Expanded(
                child: Text(
                  'Queue',
                  style: TextStyle(fontSize: 22, fontWeight: FontWeight.w700),
                ),
              ),
              FilledButton.icon(
                onPressed: onLoad,
                icon: const Icon(Icons.playlist_play),
                label: const Text('Load'),
              ),
            ],
          ),
          const SizedBox(height: 12),
          TextField(
            controller: newItemController,
            decoration: InputDecoration(
              prefixIcon: const Icon(Icons.link),
              suffixIcon: IconButton(
                tooltip: 'Add',
                onPressed: onAppend,
                icon: const Icon(Icons.add),
              ),
              border: const OutlineInputBorder(
                borderRadius: BorderRadius.all(Radius.circular(8)),
              ),
            ),
            onSubmitted: (_) => onAppend(),
          ),
          const SizedBox(height: 12),
          Expanded(
            flex: 2,
            child: TextField(
              controller: queueController,
              focusNode: queueFocus,
              expands: true,
              minLines: null,
              maxLines: null,
              textAlignVertical: TextAlignVertical.top,
              decoration: const InputDecoration(
                alignLabelWithHint: true,
                labelText: 'Paths / URLs',
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.all(Radius.circular(8)),
                ),
              ),
            ),
          ),
          const SizedBox(height: 12),
          Expanded(
            flex: 3,
            child: DecoratedBox(
              decoration: BoxDecoration(
                border: Border.all(
                  color: Theme.of(context).colorScheme.outlineVariant,
                ),
                borderRadius: BorderRadius.circular(8),
              ),
              child: ListView.separated(
                itemCount: playlist.length,
                separatorBuilder: (_, index) => const Divider(height: 1),
                itemBuilder: (context, index) {
                  final track = playlist[index];
                  final selected = index == currentIndex;
                  return ListTile(
                    dense: true,
                    selected: selected,
                    leading: Icon(
                      selected ? Icons.graphic_eq : Icons.music_note,
                    ),
                    title: Text(
                      track.title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    subtitle: Text(
                      track.uri,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    trailing: IconButton(
                      tooltip: 'Play',
                      onPressed: () => onPlayIndex(index),
                      icon: const Icon(Icons.play_arrow),
                    ),
                  );
                },
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _ControlPane extends StatelessWidget {
  const _ControlPane({
    required this.state,
    required this.visualizationFrame,
    required this.devices,
    required this.busy,
    required this.error,
    required this.positionMs,
    required this.durationMs,
    required this.manualSeekController,
    required this.onPlayPause,
    required this.onStop,
    required this.onPrevious,
    required this.onNext,
    required this.onSeekStart,
    required this.onSeekChanged,
    required this.onSeekEnd,
    required this.onManualSeek,
    required this.onVolumeChanged,
    required this.onMutedChanged,
    required this.onFadeIn,
    required this.onFadeOut,
    required this.onSpeedChanged,
    required this.onShuffleChanged,
    required this.onRepeatChanged,
    required this.onOutputChanged,
    required this.timeLabel,
  });

  final player.PlaybackState? state;
  final player.VisualizationFrame? visualizationFrame;
  final List<player.AudioOutputDevice> devices;
  final bool busy;
  final String error;
  final double positionMs;
  final double durationMs;
  final TextEditingController manualSeekController;
  final VoidCallback onPlayPause;
  final VoidCallback onStop;
  final VoidCallback onPrevious;
  final VoidCallback onNext;
  final ValueChanged<double> onSeekStart;
  final ValueChanged<double> onSeekChanged;
  final ValueChanged<double> onSeekEnd;
  final VoidCallback onManualSeek;
  final ValueChanged<double> onVolumeChanged;
  final ValueChanged<bool> onMutedChanged;
  final VoidCallback onFadeIn;
  final VoidCallback onFadeOut;
  final ValueChanged<double> onSpeedChanged;
  final ValueChanged<bool> onShuffleChanged;
  final ValueChanged<player.RepeatMode> onRepeatChanged;
  final ValueChanged<String> onOutputChanged;
  final String timeLabel;

  @override
  Widget build(BuildContext context) {
    final color = Theme.of(context).colorScheme;
    final activeTrack = state?.currentTitle.isNotEmpty == true
        ? state!.currentTitle
        : 'No track';
    final currentOutput = state?.outputDeviceId ?? '';
    final volume = (state?.volume ?? 1.0).clamp(0.0, 1.5);
    final speed = state?.speed ?? 1.0;
    final repeatMode = state?.repeatMode ?? player.RepeatMode.none;
    final showHttpBuffer = _isHttpUri(state?.currentUri ?? '');
    final bufferingPercent = (state?.bufferingPercent ?? 100)
        .clamp(0, 100)
        .toInt();
    final deviceItems = devices.isEmpty
        ? [const DropdownMenuItem(value: '', child: Text('System default'))]
        : devices
              .map(
                (device) => DropdownMenuItem(
                  value: device.id,
                  child: Text(device.name),
                ),
              )
              .toList();

    return Padding(
      padding: const EdgeInsets.all(20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      activeTrack,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(
                        fontSize: 24,
                        fontWeight: FontWeight.w800,
                      ),
                    ),
                    const SizedBox(height: 6),
                    Text(
                      state?.currentUri ?? '',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(color: color.onSurfaceVariant),
                    ),
                  ],
                ),
              ),
              if (busy)
                const SizedBox(
                  width: 24,
                  height: 24,
                  child: CircularProgressIndicator(strokeWidth: 2.5),
                ),
            ],
          ),
          const SizedBox(height: 18),
          _AudioVisualization(frame: visualizationFrame),
          const SizedBox(height: 14),
          Slider(
            value: positionMs,
            max: durationMs > 0 ? durationMs : 1,
            onChangeStart: onSeekStart,
            onChanged: onSeekChanged,
            onChangeEnd: onSeekEnd,
          ),
          if (showHttpBuffer) ...[
            const SizedBox(height: 4),
            _HttpBufferProgress(
              percent: bufferingPercent,
              buffering: state?.isBuffering ?? false,
            ),
          ],
          Row(
            children: [
              Expanded(child: Text(timeLabel)),
              SizedBox(
                width: 132,
                child: TextField(
                  controller: manualSeekController,
                  keyboardType: TextInputType.number,
                  decoration: const InputDecoration(
                    isDense: true,
                    suffixText: 'ms',
                    border: OutlineInputBorder(
                      borderRadius: BorderRadius.all(Radius.circular(8)),
                    ),
                  ),
                  onSubmitted: (_) => onManualSeek(),
                ),
              ),
              IconButton(
                tooltip: 'Seek',
                onPressed: onManualSeek,
                icon: const Icon(Icons.low_priority),
              ),
            ],
          ),
          const SizedBox(height: 16),
          Wrap(
            spacing: 10,
            runSpacing: 10,
            alignment: WrapAlignment.center,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              IconButton.filledTonal(
                tooltip: 'Previous',
                onPressed: onPrevious,
                icon: const Icon(Icons.skip_previous),
              ),
              IconButton.filled(
                key: const ValueKey('play-pause-button'),
                tooltip: state?.isPlaying == true ? 'Pause' : 'Play',
                iconSize: 34,
                onPressed: onPlayPause,
                icon: Icon(
                  state?.isPlaying == true ? Icons.pause : Icons.play_arrow,
                ),
              ),
              IconButton.filledTonal(
                tooltip: 'Next',
                onPressed: onNext,
                icon: const Icon(Icons.skip_next),
              ),
              IconButton.outlined(
                tooltip: 'Stop',
                onPressed: onStop,
                icon: const Icon(Icons.stop),
              ),
            ],
          ),
          const SizedBox(height: 24),
          _Section(
            child: Column(
              children: [
                Row(
                  children: [
                    IconButton(
                      tooltip: state?.muted == true ? 'Unmute' : 'Mute',
                      onPressed: () => onMutedChanged(!(state?.muted ?? false)),
                      icon: Icon(
                        state?.muted == true
                            ? Icons.volume_off
                            : Icons.volume_up,
                      ),
                    ),
                    Expanded(
                      child: Slider(
                        value: volume,
                        max: 1.5,
                        divisions: 30,
                        label: volume.toStringAsFixed(2),
                        onChanged: onVolumeChanged,
                      ),
                    ),
                    SizedBox(
                      width: 48,
                      child: Text('${(volume * 100).round()}%'),
                    ),
                    IconButton(
                      tooltip: 'Fade in',
                      onPressed: onFadeIn,
                      icon: const Icon(Icons.trending_up),
                    ),
                    IconButton(
                      tooltip: 'Fade out',
                      onPressed: onFadeOut,
                      icon: const Icon(Icons.trending_down),
                    ),
                  ],
                ),
                const SizedBox(height: 12),
                Wrap(
                  spacing: 12,
                  runSpacing: 12,
                  children: [
                    SegmentedButton<double>(
                      segments: const [
                        ButtonSegment(value: 0.5, label: Text('0.5x')),
                        ButtonSegment(value: 1.0, label: Text('1x')),
                        ButtonSegment(value: 1.5, label: Text('1.5x')),
                        ButtonSegment(value: 2.0, label: Text('2x')),
                      ],
                      selected: {speed},
                      onSelectionChanged: (values) =>
                          onSpeedChanged(values.first),
                    ),
                    FilterChip(
                      selected: state?.shuffle ?? false,
                      avatar: const Icon(Icons.shuffle),
                      label: const Text('Shuffle'),
                      onSelected: onShuffleChanged,
                    ),
                    SegmentedButton<player.RepeatMode>(
                      segments: const [
                        ButtonSegment(
                          value: player.RepeatMode.none,
                          icon: Icon(Icons.arrow_right_alt),
                        ),
                        ButtonSegment(
                          value: player.RepeatMode.one,
                          icon: Icon(Icons.repeat_one),
                        ),
                        ButtonSegment(
                          value: player.RepeatMode.all,
                          icon: Icon(Icons.repeat),
                        ),
                      ],
                      selected: {repeatMode},
                      onSelectionChanged: (values) =>
                          onRepeatChanged(values.first),
                    ),
                  ],
                ),
              ],
            ),
          ),
          const SizedBox(height: 16),
          _Section(
            child: DropdownButtonFormField<String>(
              initialValue:
                  deviceItems.any((item) => item.value == currentOutput)
                  ? currentOutput
                  : '',
              items: deviceItems,
              decoration: const InputDecoration(
                prefixIcon: Icon(Icons.speaker),
                labelText: 'Output',
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.all(Radius.circular(8)),
                ),
              ),
              onChanged: (value) => onOutputChanged(value ?? ''),
            ),
          ),
          if (error.isNotEmpty) ...[
            const SizedBox(height: 16),
            DecoratedBox(
              decoration: BoxDecoration(
                color: color.errorContainer,
                borderRadius: BorderRadius.circular(8),
              ),
              child: Padding(
                padding: const EdgeInsets.all(12),
                child: Text(
                  error,
                  style: TextStyle(color: color.onErrorContainer),
                ),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

class _HttpBufferProgress extends StatelessWidget {
  const _HttpBufferProgress({required this.percent, required this.buffering});

  final int percent;
  final bool buffering;

  @override
  Widget build(BuildContext context) {
    final color = Theme.of(context).colorScheme;
    final value = percent.clamp(0, 100) / 100.0;
    final status = buffering
        ? 'Buffering'
        : percent >= 100
        ? 'Buffered'
        : 'Downloading';

    return Semantics(
      label: 'HTTP buffer',
      value: '$percent%',
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 6),
        child: Column(
          key: const ValueKey('http-buffer-progress'),
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ClipRRect(
              borderRadius: BorderRadius.circular(2),
              child: LinearProgressIndicator(
                minHeight: 4,
                value: value,
                backgroundColor: color.surfaceContainerHighest,
              ),
            ),
            const SizedBox(height: 4),
            Row(
              children: [
                Expanded(
                  child: Text(
                    status,
                    style: TextStyle(
                      fontSize: 12,
                      color: color.onSurfaceVariant,
                    ),
                  ),
                ),
                Text(
                  '$percent%',
                  style: TextStyle(fontSize: 12, color: color.onSurfaceVariant),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class _AudioVisualization extends StatelessWidget {
  const _AudioVisualization({required this.frame});

  final player.VisualizationFrame? frame;

  @override
  Widget build(BuildContext context) {
    final color = Theme.of(context).colorScheme;
    final active = frame?.isActive == true;
    final rms = _metricValue(frame?.rmsNormalized);
    final peak = _metricValue(frame?.peakNormalized);
    final beat = _metricValue(frame?.beatStrength);
    final pcmReady = active && (frame?.pcm.isNotEmpty ?? false);

    return Semantics(
      label: 'Audio visualization',
      value: active ? 'Active' : 'Idle',
      child: DecoratedBox(
        key: const ValueKey('audio-visualization'),
        decoration: BoxDecoration(
          color: color.surfaceContainerHighest,
          border: Border.all(color: color.outlineVariant),
          borderRadius: BorderRadius.circular(8),
        ),
        child: Padding(
          padding: const EdgeInsets.all(10),
          child: Column(
            children: [
              SizedBox(
                key: const ValueKey('visualization-combined-canvas'),
                height: 112,
                child: CustomPaint(
                  painter: _AudioVisualizationPainter(
                    frame: frame,
                    colorScheme: color,
                  ),
                ),
              ),
              const SizedBox(height: 8),
              Row(
                children: [
                  Expanded(
                    child: _VisualizationMeter(
                      key: const ValueKey('visualization-rms'),
                      label: 'RMS',
                      value: rms,
                      color: color.primary,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: _VisualizationMeter(
                      key: const ValueKey('visualization-peak'),
                      label: 'Peak',
                      value: peak,
                      color: color.tertiary,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: _VisualizationMeter(
                      key: const ValueKey('visualization-beat'),
                      label: 'Beat',
                      value: beat,
                      color: frame?.beat == true
                          ? color.error
                          : color.secondary,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: _VisualizationMeter(
                      key: const ValueKey('visualization-pcm'),
                      label: 'PCM',
                      value: pcmReady ? 1.0 : 0.0,
                      color: color.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _VisualizationMeter extends StatelessWidget {
  const _VisualizationMeter({
    super.key,
    required this.label,
    required this.value,
    required this.color,
  });

  final String label;
  final double value;
  final Color color;

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    final normalized = value.clamp(0.0, 1.0).toDouble();

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          height: 4,
          child: ClipRRect(
            borderRadius: BorderRadius.circular(2),
            child: LinearProgressIndicator(
              value: normalized,
              minHeight: 4,
              color: color,
              backgroundColor: colorScheme.surface.withValues(alpha: 0.75),
            ),
          ),
        ),
        const SizedBox(height: 4),
        Text(
          label,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(fontSize: 11, color: colorScheme.onSurfaceVariant),
        ),
      ],
    );
  }
}

class _AudioVisualizationPainter extends CustomPainter {
  const _AudioVisualizationPainter({
    required this.frame,
    required this.colorScheme,
  });

  static const int _fallbackBandCount = 48;

  final player.VisualizationFrame? frame;
  final ColorScheme colorScheme;

  @override
  void paint(Canvas canvas, Size size) {
    final backgroundPaint = Paint()
      ..color = colorScheme.surfaceContainerHighest
      ..style = PaintingStyle.fill;
    final borderPaint = Paint()
      ..color = colorScheme.outlineVariant
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    final rect = Offset.zero & size;
    canvas.drawRect(rect, backgroundPaint);

    final spectrumValues = _spectrumValues(frame);
    final waveformValues = _waveformValues(frame);
    final centerY = size.height * 0.5;
    final guidePaint = Paint()
      ..color = colorScheme.outlineVariant.withValues(alpha: 0.42)
      ..strokeWidth = 1;
    canvas.drawLine(
      Offset(10, centerY),
      Offset(size.width - 10, centerY),
      guidePaint,
    );

    _paintWaveform(canvas, size, waveformValues);

    final barCount = spectrumValues.isEmpty
        ? _fallbackBandCount
        : spectrumValues.length;
    final gap = 3.0;
    final availableWidth = math.max(0.0, size.width - 20);
    final barWidth = math.max(
      1.0,
      (availableWidth - gap * (barCount - 1)) / barCount,
    );
    final activePaint = Paint()
      ..shader = LinearGradient(
        begin: Alignment.bottomCenter,
        end: Alignment.topCenter,
        colors: [colorScheme.primary, colorScheme.tertiary],
      ).createShader(rect);
    final idlePaint = Paint()
      ..color = colorScheme.onSurfaceVariant.withValues(alpha: 0.20);

    for (var index = 0; index < barCount; index += 1) {
      final value = spectrumValues.isEmpty ? 0.0 : spectrumValues[index];
      final shaped = math.pow(value.clamp(0.0, 1.0), 0.72).toDouble();
      final height = math.max(3.0, shaped * (size.height * 0.56));
      final left = 10 + index * (barWidth + gap);
      final top = size.height - 8 - height;
      final barRect = RRect.fromRectAndRadius(
        Rect.fromLTWH(left, top, barWidth, height),
        const Radius.circular(2),
      );
      canvas.drawRRect(barRect, value > 0.01 ? activePaint : idlePaint);
    }

    canvas.drawRect(rect.deflate(0.5), borderPaint);
  }

  void _paintWaveform(Canvas canvas, Size size, List<double> values) {
    if (values.length < 2) return;

    final path = Path();
    final left = 10.0;
    final width = math.max(1.0, size.width - 20);
    final centerY = size.height * 0.32;
    final amplitude = size.height * 0.22;
    for (var index = 0; index < values.length; index += 1) {
      final x = left + width * index / math.max(1, values.length - 1);
      final y = centerY - values[index].clamp(-1.0, 1.0) * amplitude;
      if (index == 0) {
        path.moveTo(x, y);
      } else {
        path.lineTo(x, y);
      }
    }

    final shadowPaint = Paint()
      ..color = colorScheme.surface.withValues(alpha: 0.7)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 5
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;
    final wavePaint = Paint()
      ..color = colorScheme.secondary
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;
    canvas.drawPath(path, shadowPaint);
    canvas.drawPath(path, wavePaint);
  }

  @override
  bool shouldRepaint(covariant _AudioVisualizationPainter oldDelegate) {
    return oldDelegate.frame != frame || oldDelegate.colorScheme != colorScheme;
  }
}

List<double> _spectrumValues(player.VisualizationFrame? frame) {
  final values = frame?.normalized;
  if (values == null || values.isEmpty || frame?.isActive != true) {
    return const [];
  }

  return values.map((value) => value.clamp(0.0, 1.0).toDouble()).toList();
}

List<double> _waveformValues(player.VisualizationFrame? frame) {
  final values = frame?.waveform;
  if (values == null || values.isEmpty || frame?.isActive != true) {
    return const [];
  }

  return values.map((value) => value.clamp(-1.0, 1.0).toDouble()).toList();
}

double _metricValue(double? value) {
  if (value == null || !value.isFinite) return 0;
  return value.clamp(0.0, 1.0).toDouble();
}

class _Section extends StatelessWidget {
  const _Section({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        border: Border.all(color: Theme.of(context).colorScheme.outlineVariant),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Padding(padding: const EdgeInsets.all(12), child: child),
    );
  }
}

bool _isHttpUri(String uri) {
  final lower = uri.toLowerCase();
  return lower.startsWith('http://') || lower.startsWith('https://');
}
