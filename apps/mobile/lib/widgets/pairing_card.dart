import 'package:flutter/material.dart';

import '../backend.dart';

/// Shown over the home screen while a pairing code awaits the user.
class PairingCard extends StatelessWidget {
  const PairingCard({super.key, required this.prompt, required this.onAnswer});

  final PairingUpdate prompt;
  final ValueChanged<bool> onAnswer;

  @override
  Widget build(BuildContext context) {
    final device = prompt.device!;
    final code = prompt.code!;
    final theme = Theme.of(context);
    return AlertDialog(
      title: Text('Pair with ${device.name}?'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          const Text(
            'Check that this code matches the one on the other device:',
          ),
          const SizedBox(height: 16),
          Text(
            '${code.substring(0, 3)} ${code.substring(3)}',
            key: const Key('pairing-code'),
            style: theme.textTheme.displaySmall?.copyWith(
              fontFeatures: const [FontFeature.tabularFigures()],
              letterSpacing: 4,
            ),
          ),
          const SizedBox(height: 8),
          Text(
            '${device.os} · key ${device.fingerprint}',
            style: theme.textTheme.bodySmall,
          ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => onAnswer(false),
          child: const Text("Doesn't match"),
        ),
        FilledButton(
          onPressed: () => onAnswer(true),
          child: const Text('Codes match'),
        ),
      ],
    );
  }
}
