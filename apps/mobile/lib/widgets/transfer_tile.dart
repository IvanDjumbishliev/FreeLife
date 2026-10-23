import 'package:flutter/material.dart';

import '../backend.dart';
import '../util/format.dart';

class TransferTile extends StatelessWidget {
  const TransferTile({super.key, required this.update});

  final TransferUpdate update;

  @override
  Widget build(BuildContext context) {
    final u = update;
    final sending = u.direction == TransferDirection.send;
    final total = u.totalBytes;
    final fraction = total == 0 ? 1.0 : u.bytesSent / total;
    final theme = Theme.of(context);

    final String status;
    switch (u.kind) {
      case TransferKind.progress:
        status =
            '${formatBytes(u.bytesSent)} of ${formatBytes(total)} · '
            '${formatSpeed(u.speedBps)} · ${formatEta(u.etaSecs)}';
      case TransferKind.complete:
        status = sending
            ? 'Sent ${formatBytes(total)}'
            : 'Saved · ${formatBytes(total)}';
      case TransferKind.failed:
        status = u.error ?? 'Failed';
    }

    return ListTile(
      leading: Icon(
        u.kind == TransferKind.failed
            ? Icons.error_outline
            : sending
            ? Icons.upload
            : Icons.download,
        color: u.kind == TransferKind.failed ? theme.colorScheme.error : null,
      ),
      title: Text(u.filename, overflow: TextOverflow.ellipsis),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('${sending ? 'To' : 'From'} ${u.peerName}'),
          if (u.kind == TransferKind.progress)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 4),
              child: LinearProgressIndicator(value: fraction),
            ),
          Text(
            status,
            style: u.kind == TransferKind.failed
                ? TextStyle(color: theme.colorScheme.error)
                : null,
          ),
        ],
      ),
    );
  }
}
