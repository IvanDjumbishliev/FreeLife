import 'package:flutter/material.dart';

import '../backend.dart';
import '../util/format.dart';

/// Bottom sheet for "Share via FreeLife": pick the paired device to send
/// `paths` to. Pops with the chosen device, or null.
class DevicePickerSheet extends StatelessWidget {
  const DevicePickerSheet({
    super.key,
    required this.devices,
    required this.paths,
  });

  final List<PairedDevice> devices;
  final List<String> paths;

  static Future<PairedDevice?> show(
    BuildContext context, {
    required List<PairedDevice> devices,
    required List<String> paths,
  }) => showModalBottomSheet<PairedDevice>(
    context: context,
    showDragHandle: true,
    builder: (_) => DevicePickerSheet(devices: devices, paths: paths),
  );

  @override
  Widget build(BuildContext context) {
    final what = paths.length == 1
        ? basename(paths.single)
        : '${paths.length} files';
    return SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(24, 0, 24, 8),
            child: Text(
              'Send $what to…',
              style: Theme.of(context).textTheme.titleMedium,
            ),
          ),
          if (devices.isEmpty)
            const Padding(
              padding: EdgeInsets.all(24),
              child: Text('No paired devices. Pair one from the main screen.'),
            ),
          for (final d in devices)
            ListTile(
              leading: Icon(_osIcon(d.device.os)),
              title: Text(d.device.name),
              subtitle: Text(d.connected ? 'Connected' : d.device.os),
              onTap: () => Navigator.of(context).pop(d),
            ),
        ],
      ),
    );
  }
}

IconData _osIcon(String os) => switch (os) {
  'Android' => Icons.phone_android,
  'MacOS' => Icons.laptop_mac,
  'Windows' => Icons.laptop_windows,
  _ => Icons.computer,
};
