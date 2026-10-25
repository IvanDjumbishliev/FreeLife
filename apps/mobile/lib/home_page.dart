import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'backend.dart';
import 'clipboard_sync.dart';
import 'webcam_controller.dart';
import 'platform/airshare_channel.dart';
import 'platform/device_channel.dart';
import 'util/format.dart';
import 'widgets/device_picker_sheet.dart';
import 'widgets/pairing_card.dart';
import 'widgets/transfer_tile.dart';

class HomePage extends StatefulWidget {
  const HomePage({
    super.key,
    required this.backend,
    required this.device,
    required this.airshare,
    required this.clipboard,
    required this.webcam,
  });

  final FreeLifeBackend backend;
  final DeviceChannel device;
  final AirShareChannel airshare;
  final ClipboardSyncController clipboard;
  final WebcamController webcam;

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  FreeLifeBackend get _backend => widget.backend;

  DeviceSummary? _self;
  String? _startError;
  List<PairedDevice> _paired = const [];
  List<NearbyDevice> _nearby = const [];
  bool _scanning = false;
  String? _pairingWith;
  bool _clipboardOn = false;

  /// Pairing codes waiting for an answer; the first one is on screen.
  final _prompts = <PairingUpdate>[];

  /// Latest update per transfer, newest transfer first.
  final _transfers = <String, TransferUpdate>{};

  final _subscriptions = <StreamSubscription<Object?>>[];
  Timer? _refreshTimer;

  @override
  void initState() {
    super.initState();
    _start();
  }

  @override
  void dispose() {
    for (final s in _subscriptions) {
      s.cancel();
    }
    _refreshTimer?.cancel();
    super.dispose();
  }

  Future<void> _start() async {
    // Best effort: neither is fatal, and both are missing off Android.
    await _ignoreMissingPlatform(widget.airshare.requestNotificationPermission);
    await _ignoreMissingPlatform(widget.airshare.acquireMulticastLock);

    try {
      final dirs = await widget.airshare.storageDirs();
      final name = await _deviceName();
      final self = await _backend.start(
        dataDir: dirs.dataDir,
        downloadDir: dirs.downloadDir,
        deviceName: name,
      );
      if (!mounted) return;
      setState(() => _self = self);
    } on Object catch (e) {
      if (mounted) setState(() => _startError = '$e');
      return;
    }

    await widget.clipboard.start();
    await widget.webcam.init();
    if (mounted) {
      setState(() => _clipboardOn = widget.clipboard.engine.enabled);
    }

    _subscriptions
      ..add(_backend.transferEvents().listen(_onTransfer))
      ..add(_backend.pairingEvents().listen(_onPairing))
      ..add(widget.airshare.sharedFiles.listen(_shareFiles));
    _refreshPaired();
    // Connection state isn't evented yet, so poll gently.
    _refreshTimer = Timer.periodic(
      const Duration(seconds: 5),
      (_) => _refreshPaired(),
    );

    final launched = await widget.airshare.takeSharedFiles();
    if (launched.isNotEmpty) await _shareFiles(launched);
  }

  Future<String> _deviceName() async {
    try {
      return (await widget.device.getDeviceInfo()).displayName;
    } on Object {
      return 'FreeLife phone';
    }
  }

  void _refreshPaired() {
    if (!mounted || _self == null) return;
    setState(() => _paired = _backend.pairedDevices());
  }

  void _onTransfer(TransferUpdate u) {
    final isNew = !_transfers.containsKey(u.id);
    setState(() {
      if (isNew) {
        final older = Map.of(_transfers);
        _transfers
          ..clear()
          ..[u.id] = u
          ..addAll(older);
      } else {
        _transfers[u.id] = u;
      }
    });

    final receiving = u.direction == TransferDirection.receive;
    switch (u.kind) {
      case TransferKind.complete when receiving && u.savedPath != null:
        _ignoreMissingPlatform(
          () => widget.airshare.showFileReceived(
            device: u.peerName,
            filename: u.filename,
            path: u.savedPath!,
          ),
        );
        _snack('Received ${u.filename} from ${u.peerName}');
      case TransferKind.complete:
        _snack('Sent ${u.filename} to ${u.peerName}');
      case TransferKind.failed:
        _snack('${u.filename}: ${u.error}', error: true);
      case TransferKind.progress:
        break;
    }
  }

  void _onPairing(PairingUpdate u) {
    setState(() {
      if (u.code != null) {
        _prompts.add(u);
      } else {
        _prompts.removeWhere((p) => p.requestId == u.requestId);
      }
    });
    _refreshPaired();
  }

  void _answerPrompt(PairingUpdate prompt, bool accept) {
    setState(
      () => _prompts.removeWhere((p) => p.requestId == prompt.requestId),
    );
    _backend.respondPairing(prompt.requestId, accept);
  }

  Future<void> _shareFiles(List<String> paths) async {
    if (paths.isEmpty || !mounted) return;
    final target = await DevicePickerSheet.show(
      context,
      devices: _backend.pairedDevices(),
      paths: paths,
    );
    if (target == null) return;
    for (final path in paths) {
      // Progress is shown from transferEvents(); this stream only needs
      // draining, and failures arrive as `failed` updates there too.
      _backend
          .sendFile(target.device.id, path)
          .listen(
            null,
            onError: (Object e) => _snack('${basename(path)}: $e', error: true),
          );
    }
  }

  Future<void> _scan() async {
    setState(() => _scanning = true);
    try {
      final found = await _backend.discover();
      if (mounted) setState(() => _nearby = found);
    } on Object catch (e) {
      _snack('Scan failed: $e', error: true);
    } finally {
      if (mounted) setState(() => _scanning = false);
    }
  }

  Future<void> _pair(DeviceSummary device) async {
    setState(() => _pairingWith = device.id);
    try {
      final peer = await _backend.pair(device.id);
      _snack('Paired with ${peer.name}');
      if (!mounted) return;
      setState(() {
        _nearby = [
          for (final n in _nearby)
            n.device.id == peer.id
                ? NearbyDevice(device: n.device, paired: true)
                : n,
        ];
      });
      _refreshPaired();
    } on Object catch (e) {
      _snack('Pairing failed: $e', error: true);
    } finally {
      if (mounted) setState(() => _pairingWith = null);
    }
  }

  Future<void> _unpair(PairedDevice d) async {
    await _backend.unpair(d.device.id);
    _refreshPaired();
  }

  void _snack(String message, {bool error = false}) {
    if (!mounted) return;
    final theme = Theme.of(context);
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(message),
        backgroundColor: error ? theme.colorScheme.error : null,
      ),
    );
  }

  static Future<void> _ignoreMissingPlatform(
    Future<void> Function() call,
  ) async {
    try {
      await call();
    } on MissingPluginException {
      // Not implemented on this platform.
    } on PlatformException {
      // Best effort.
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final self = _self;
    return Stack(
      children: [
        Scaffold(
          appBar: AppBar(title: const Text('FreeLife')),
          body: ListView(
            padding: const EdgeInsets.symmetric(vertical: 8),
            children: [
              ListTile(
                leading: const Icon(Icons.phone_android),
                title: Text(self?.name ?? 'This device'),
                subtitle: Text(
                  _startError ??
                      (self == null ? 'Starting…' : 'Key ${self.fingerprint}'),
                  style: _startError != null
                      ? TextStyle(color: theme.colorScheme.error)
                      : null,
                ),
              ),
              const _Header('Paired devices'),
              if (_paired.isEmpty)
                const ListTile(
                  title: Text('No paired devices yet'),
                  subtitle: Text('Scan below to pair with your computer.'),
                ),
              for (final d in _paired)
                ListTile(
                  leading: Icon(
                    Icons.circle,
                    size: 12,
                    color: d.connected ? Colors.green : theme.disabledColor,
                  ),
                  title: Text(d.device.name),
                  subtitle: Text(d.device.os),
                  trailing: PopupMenuButton<String>(
                    onSelected: (_) => _unpair(d),
                    itemBuilder: (_) => const [
                      PopupMenuItem(value: 'unpair', child: Text('Unpair')),
                    ],
                  ),
                ),
              const _Header('Clipboard'),
              SwitchListTile(
                title: const Text('Clipboard sync'),
                subtitle: Text(_clipboardOn ? 'On' : 'Off'),
                value: _clipboardOn,
                onChanged: self == null
                    ? null
                    : (on) async {
                        setState(() => _clipboardOn = on);
                        await widget.clipboard.setEnabled(on);
                      },
              ),
              for (final note in clipboardLimitations)
                Padding(
                  padding: const EdgeInsets.fromLTRB(72, 0, 16, 8),
                  child: Text(
                    note,
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                ),
              const _Header('Webcam'),
              ValueListenableBuilder<bool>(
                valueListenable: widget.webcam.active,
                builder: (context, on, _) => SwitchListTile(
                  title: const Text('Start Webcam'),
                  subtitle: Text(
                    on
                        ? 'On. Paired computers can use this camera. '
                              'A notification stays up while it runs.'
                        : 'Off. Turn on to let a paired computer use the rear camera.',
                  ),
                  value: on,
                  onChanged: self == null
                      ? null
                      : (value) => value
                            ? widget.webcam.start()
                            : widget.webcam.stop(),
                ),
              ),
              const _Header('Transfers'),
              if (_transfers.isEmpty)
                const ListTile(
                  title: Text('No transfers yet'),
                  subtitle: Text(
                    'Share a file to FreeLife from any app to send it.',
                  ),
                ),
              for (final t in _transfers.values) TransferTile(update: t),
              _Header(
                'Nearby devices',
                trailing: TextButton.icon(
                  onPressed: self == null || _scanning ? null : _scan,
                  icon: _scanning
                      ? const SizedBox.square(
                          dimension: 16,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Icon(Icons.radar),
                  label: Text(_scanning ? 'Scanning' : 'Scan'),
                ),
              ),
              if (_nearby.isEmpty)
                const ListTile(title: Text('No devices found yet')),
              for (final n in _nearby)
                ListTile(
                  title: Text(n.device.name),
                  subtitle: Text(n.device.os),
                  trailing: n.paired
                      ? const Text('Paired')
                      : FilledButton.tonal(
                          onPressed: _pairingWith == null
                              ? () => _pair(n.device)
                              : null,
                          child: Text(
                            _pairingWith == n.device.id ? 'Pairing…' : 'Pair',
                          ),
                        ),
                ),
            ],
          ),
        ),
        if (_prompts.isNotEmpty) ...[
          const ModalBarrier(dismissible: false, color: Colors.black54),
          PairingCard(
            prompt: _prompts.first,
            onAnswer: (accept) => _answerPrompt(_prompts.first, accept),
          ),
        ],
      ],
    );
  }
}

class _Header extends StatelessWidget {
  const _Header(this.title, {this.trailing});

  final String title;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 16, 8, 0),
      child: Row(
        children: [
          Expanded(
            child: Text(
              title,
              style: Theme.of(context).textTheme.titleSmall?.copyWith(
                color: Theme.of(context).colorScheme.primary,
              ),
            ),
          ),
          ?trailing,
        ],
      ),
    );
  }
}
