import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'backend.dart';
import 'clipboard_sync.dart';
import 'notifications.dart';
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
    required this.notifications,
  });

  final FreeLifeBackend backend;
  final DeviceChannel device;
  final AirShareChannel airshare;
  final ClipboardSyncController clipboard;
  final WebcamController webcam;
  final NotificationBridge notifications;

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
  bool? _notificationAccess;
  bool _notificationsOn = true;
  bool _airshareOn = true;
  bool _handoffOn = true;
  bool _onboardingComplete = true;
  int _onboardingStep = 0;
  String? _settingsPath;

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
      _settingsPath = '${dirs.dataDir}/ui-settings.json';
      _loadUiSettings();
      widget.notifications.setHandoffEnabled(_handoffOn);
      final name = await _deviceName();
      final self = await _backend.start(
        dataDir: dirs.dataDir,
        downloadDir: dirs.downloadDir,
        deviceName: name,
      );
      if (!mounted) return;
      setState(() {
        _self = self;
        _paired = _backend.pairedDevices();
      });
    } on Object catch (e) {
      if (mounted) setState(() => _startError = '$e');
      return;
    }

    await widget.clipboard.start();
    await widget.webcam.init();
    _subscriptions
      ..add(_backend.transferEvents().listen(_onTransfer))
      ..add(_backend.pairingEvents().listen(_onPairing))
      ..add(widget.airshare.sharedFiles.listen(_shareFiles));
    _subscriptions.add(widget.airshare.sharedUrls.listen(_shareUrl));
    _refreshPaired();
    // Connection state isn't evented yet, so poll gently.
    _refreshTimer = Timer.periodic(const Duration(seconds: 5), (_) {
      _refreshPaired();
      _refreshNotificationAccess();
    });

    final launched = await widget.airshare.takeSharedFiles();
    if (launched.isNotEmpty) await _shareFiles(launched);
    final launchedUrls = await widget.airshare.takeSharedUrls();
    for (final url in launchedUrls) {
      await _shareUrl(url);
    }

    final notificationAccess = await widget.notifications.hasListenerAccess();
    if (notificationAccess != null && _notificationsOn) {
      widget.notifications.start(
        onError: (error) => _snack('Notification bridge: $error', error: true),
      );
    }
    if (mounted) {
      setState(() {
        _clipboardOn = widget.clipboard.engine.enabled;
        _notificationAccess = notificationAccess;
      });
    }
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

  Future<void> _refreshNotificationAccess() async {
    final access = await widget.notifications.hasListenerAccess();
    if (mounted && access != _notificationAccess) {
      setState(() => _notificationAccess = access);
    }
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
    if (!_airshareOn) {
      _snack('File sharing is turned off in Settings', error: true);
      return;
    }
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

  Future<void> _shareUrl(String url) async {
    if (!mounted) return;
    if (!_handoffOn) {
      _snack('Handoff is turned off in Settings', error: true);
      return;
    }
    final desktops = _backend
        .pairedDevices()
        .where((peer) => peer.device.os != 'Android')
        .toList();
    if (desktops.isEmpty) {
      _snack('Pair a desktop device before handing off links', error: true);
      return;
    }
    final target = await showModalBottomSheet<PairedDevice>(
      context: context,
      builder: (context) => SafeArea(
        child: ListView(
          shrinkWrap: true,
          children: [
            const ListTile(title: Text('Send link to')),
            for (final peer in desktops)
              ListTile(
                leading: const Icon(Icons.computer),
                title: Text(peer.device.name),
                subtitle: Text(
                  peer.connected ? 'Connected' : 'Connect when sending',
                ),
                onTap: () => Navigator.pop(context, peer),
              ),
          ],
        ),
      ),
    );
    if (target == null) return;
    try {
      await _backend.sendHandoffUrl(target.device.id, url);
      _snack('Link sent to ${target.device.name}');
    } on Object catch (error) {
      _snack('Could not send link: $error', error: true);
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
        if (!_onboardingComplete) _onboardingStep = 2;
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

  void _loadUiSettings() {
    final path = _settingsPath;
    if (path == null) return;
    try {
      final file = File(path);
      if (!file.existsSync()) {
        _onboardingComplete = false;
        return;
      }
      final decoded = jsonDecode(file.readAsStringSync());
      if (decoded is! Map<String, dynamic>) {
        throw const FormatException('Invalid FreeLife UI settings');
      }
      if (!mounted) return;
      setState(() {
        _onboardingComplete = decoded['onboardingComplete'] is bool
            ? decoded['onboardingComplete'] as bool
            : false;
        _airshareOn = decoded['airshareOn'] is bool
            ? decoded['airshareOn'] as bool
            : true;
        _handoffOn = decoded['handoffOn'] is bool
            ? decoded['handoffOn'] as bool
            : true;
        _notificationsOn = decoded['notificationsOn'] is bool
            ? decoded['notificationsOn'] as bool
            : true;
      });
    } on Object catch (error) {
      _onboardingComplete = false;
      _snack('Could not load settings: $error', error: true);
    }
  }

  Future<void> _saveUiSettings() async {
    final path = _settingsPath;
    if (path == null) return;
    try {
      await File(path).writeAsString(
        jsonEncode({
          'onboardingComplete': _onboardingComplete,
          'airshareOn': _airshareOn,
          'handoffOn': _handoffOn,
          'notificationsOn': _notificationsOn,
        }),
      );
    } on Object catch (error) {
      _snack('Could not save settings: $error', error: true);
    }
  }

  Future<void> _finishOnboarding() async {
    setState(() => _onboardingComplete = true);
    await _saveUiSettings();
  }

  Future<void> _setNotificationsOn(bool enabled) async {
    if (enabled && _notificationAccess != true) {
      try {
        await widget.notifications.openListenerSettings();
      } on Object catch (error) {
        _snack('Could not open notification settings: $error', error: true);
      }
      return;
    }
    setState(() => _notificationsOn = enabled);
    await widget.notifications.setEnabled(enabled);
    await _saveUiSettings();
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
              if (!_onboardingComplete)
                _OnboardingCard(
                  step: _onboardingStep,
                  scanning: _scanning,
                  nearby: _nearby,
                  pairing: _pairingWith != null,
                  paired: _paired.isNotEmpty,
                  clipboardOn: _clipboardOn,
                  webcamOn: widget.webcam.active.value,
                  airshareOn: _airshareOn,
                  handoffOn: _handoffOn,
                  notificationsOn: _notificationsOn,
                  onScan: _scan,
                  onNext: () => setState(() => _onboardingStep = 1),
                  onPair: (device) => _pair(device),
                  onClipboardChanged: (on) async {
                    setState(() => _clipboardOn = on);
                    await widget.clipboard.setEnabled(on);
                  },
                  onWebcamChanged: (on) =>
                      on ? widget.webcam.start() : widget.webcam.stop(),
                  onAirshareChanged: (on) async {
                    setState(() => _airshareOn = on);
                    await _saveUiSettings();
                  },
                  onHandoffChanged: (on) async {
                    setState(() => _handoffOn = on);
                    widget.notifications.setHandoffEnabled(on);
                    await _saveUiSettings();
                  },
                  onNotificationsChanged: _setNotificationsOn,
                  onFinish: _finishOnboarding,
                ),
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
              if (_paired.isNotEmpty)
                Card(
                  margin: const EdgeInsets.symmetric(horizontal: 16),
                  child: Padding(
                    padding: const EdgeInsets.symmetric(vertical: 8),
                    child: Column(
                      children: [
                        ListTile(
                          leading: const Icon(Icons.desktop_windows),
                          title: Text(_paired.first.device.name),
                          subtitle: Text(
                            '${_paired.first.device.os} · ${_paired.first.connected ? 'Connected · local network' : 'Paired · waiting for connection'}',
                          ),
                          trailing: PopupMenuButton<String>(
                            onSelected: (_) => _unpair(_paired.first),
                            itemBuilder: (_) => const [
                              PopupMenuItem(
                                value: 'unpair',
                                child: Text('Unpair'),
                              ),
                            ],
                            icon: Icon(
                              Icons.circle,
                              size: 12,
                              color: _paired.first.connected
                                  ? Colors.green
                                  : theme.disabledColor,
                            ),
                          ),
                        ),
                        SwitchListTile(
                          title: const Text('Clipboard'),
                          value: _clipboardOn,
                          onChanged: self == null
                              ? null
                              : (on) async {
                                  setState(() => _clipboardOn = on);
                                  await widget.clipboard.setEnabled(on);
                                },
                        ),
                        ValueListenableBuilder<bool>(
                          valueListenable: widget.webcam.active,
                          builder: (context, on, _) => SwitchListTile(
                            title: const Text('Webcam'),
                            value: on,
                            onChanged: self == null
                                ? null
                                : (value) => value
                                      ? widget.webcam.start()
                                      : widget.webcam.stop(),
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              if (_paired.isEmpty)
                const ListTile(
                  title: Text('No paired devices yet'),
                  subtitle: Text('Scan below to pair with your computer.'),
                ),
              for (final d in _paired.skip(1))
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
              ListTile(
                title: const Text('Clipboard sync'),
                subtitle: Text(_clipboardOn ? 'On' : 'Off'),
              ),
              for (final note in clipboardLimitations)
                Padding(
                  padding: const EdgeInsets.fromLTRB(72, 0, 16, 8),
                  child: Text(
                    note,
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                ),
              if (_notificationAccess != null) ...[
                const _Header('Notifications'),
                SwitchListTile(
                  title: const Text('Mirror notifications'),
                  value: _notificationsOn && _notificationAccess!,
                  onChanged: _notificationAccess! ? _setNotificationsOn : null,
                ),
                ListTile(
                  title: Text(
                    _notificationAccess!
                        ? 'Notification mirroring is on'
                        : 'Enable notification mirroring',
                  ),
                  subtitle: Text(
                    _notificationAccess!
                        ? 'Phone notifications are shared with paired desktops.'
                        : 'Allow FreeLife to read notifications in Android Settings.',
                  ),
                  trailing: _notificationAccess!
                      ? const Icon(Icons.check_circle, color: Colors.green)
                      : TextButton(
                          onPressed: () async {
                            try {
                              await widget.notifications.openListenerSettings();
                            } on Object catch (error) {
                              _snack(
                                'Could not open notification settings: $error',
                                error: true,
                              );
                            }
                          },
                          child: const Text('Open Settings'),
                        ),
                ),
              ],
              const _Header('Webcam'),
              ValueListenableBuilder<bool>(
                valueListenable: widget.webcam.active,
                builder: (context, on, _) => ListTile(
                  title: const Text('Webcam'),
                  subtitle: Text(on ? 'On' : 'Off'),
                ),
              ),
              const _Header('Settings'),
              SwitchListTile(
                title: const Text('AirShare file transfers'),
                subtitle: const Text('Allow sharing files to paired devices'),
                value: _airshareOn,
                onChanged: (value) async {
                  setState(() => _airshareOn = value);
                  await _saveUiSettings();
                },
              ),
              SwitchListTile(
                title: const Text('Handoff'),
                subtitle: const Text('Allow sending links to paired devices'),
                value: _handoffOn,
                onChanged: (value) async {
                  setState(() => _handoffOn = value);
                  widget.notifications.setHandoffEnabled(value);
                  await _saveUiSettings();
                },
              ),
              SwitchListTile(
                title: const Text('Notification mirroring'),
                value: _notificationsOn,
                onChanged: _setNotificationsOn,
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

class _OnboardingCard extends StatelessWidget {
  const _OnboardingCard({
    required this.step,
    required this.scanning,
    required this.nearby,
    required this.pairing,
    required this.paired,
    required this.clipboardOn,
    required this.webcamOn,
    required this.airshareOn,
    required this.handoffOn,
    required this.notificationsOn,
    required this.onScan,
    required this.onNext,
    required this.onPair,
    required this.onClipboardChanged,
    required this.onWebcamChanged,
    required this.onAirshareChanged,
    required this.onHandoffChanged,
    required this.onNotificationsChanged,
    required this.onFinish,
  });

  final int step;
  final bool scanning;
  final List<NearbyDevice> nearby;
  final bool pairing;
  final bool paired;
  final bool clipboardOn;
  final bool webcamOn;
  final bool airshareOn;
  final bool handoffOn;
  final bool notificationsOn;
  final VoidCallback onScan;
  final VoidCallback onNext;
  final ValueChanged<DeviceSummary> onPair;
  final ValueChanged<bool> onClipboardChanged;
  final ValueChanged<bool> onWebcamChanged;
  final ValueChanged<bool> onAirshareChanged;
  final ValueChanged<bool> onHandoffChanged;
  final ValueChanged<bool> onNotificationsChanged;
  final VoidCallback onFinish;

  @override
  Widget build(BuildContext context) {
    return Card(
      margin: const EdgeInsets.all(16),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: AnimatedSwitcher(
          duration: const Duration(milliseconds: 250),
          child: switch (step) {
            0 => Column(
              key: const ValueKey(0),
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'Step 1 of 3',
                  style: Theme.of(context).textTheme.labelMedium,
                ),
                const SizedBox(height: 8),
                Text(
                  'Searching for devices on your network...',
                  style: Theme.of(context).textTheme.titleMedium,
                ),
                const SizedBox(height: 12),
                if (scanning) const LinearProgressIndicator(),
                if (nearby.isNotEmpty) Text('${nearby.length} device(s) found'),
                Row(
                  mainAxisAlignment: MainAxisAlignment.end,
                  children: [
                    TextButton.icon(
                      onPressed: scanning ? null : onScan,
                      icon: const Icon(Icons.radar),
                      label: Text(scanning ? 'Searching...' : 'Search'),
                    ),
                    FilledButton(onPressed: onNext, child: const Text('Next')),
                  ],
                ),
              ],
            ),
            1 => Column(
              key: const ValueKey(1),
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'Step 2 of 3',
                  style: Theme.of(context).textTheme.labelMedium,
                ),
                const SizedBox(height: 8),
                Text(
                  pairing
                      ? 'Confirm that the six-digit PIN matches on both devices.'
                      : 'Pair with a desktop',
                  style: Theme.of(context).textTheme.titleMedium,
                ),
                for (final item in nearby.where((item) => !item.paired))
                  ListTile(
                    contentPadding: EdgeInsets.zero,
                    title: Text(item.device.name),
                    subtitle: Text(item.device.os),
                    trailing: TextButton(
                      onPressed: pairing ? null : () => onPair(item.device),
                      child: const Text('Pair'),
                    ),
                  ),
                if (nearby.isEmpty)
                  const Text('Search again to find a desktop on your network.'),
                if (paired)
                  Align(
                    alignment: Alignment.centerRight,
                    child: TextButton(
                      onPressed: onNext,
                      child: const Text('Continue'),
                    ),
                  ),
                Align(
                  alignment: Alignment.centerRight,
                  child: TextButton(
                    onPressed: onScan,
                    child: const Text('Search again'),
                  ),
                ),
              ],
            ),
            _ => Column(
              key: const ValueKey(2),
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'Step 3 of 3',
                  style: Theme.of(context).textTheme.labelMedium,
                ),
                const SizedBox(height: 8),
                Text(
                  'You’re connected! Choose which features to enable.',
                  style: Theme.of(context).textTheme.titleMedium,
                ),
                SwitchListTile(
                  contentPadding: EdgeInsets.zero,
                  title: const Text('AirShare file transfers'),
                  value: airshareOn,
                  onChanged: onAirshareChanged,
                ),
                SwitchListTile(
                  contentPadding: EdgeInsets.zero,
                  title: const Text('Clipboard sync'),
                  value: clipboardOn,
                  onChanged: onClipboardChanged,
                ),
                SwitchListTile(
                  contentPadding: EdgeInsets.zero,
                  title: const Text('Webcam'),
                  value: webcamOn,
                  onChanged: onWebcamChanged,
                ),
                SwitchListTile(
                  contentPadding: EdgeInsets.zero,
                  title: const Text('Notifications'),
                  value: notificationsOn,
                  onChanged: onNotificationsChanged,
                ),
                SwitchListTile(
                  contentPadding: EdgeInsets.zero,
                  title: const Text('Handoff'),
                  value: handoffOn,
                  onChanged: onHandoffChanged,
                ),
                Align(
                  alignment: Alignment.centerRight,
                  child: FilledButton(
                    onPressed: onFinish,
                    child: const Text('Finish'),
                  ),
                ),
              ],
            ),
          },
        ),
      ),
    );
  }
}
