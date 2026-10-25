import 'dart:async';

import 'package:flutter/services.dart';

import 'src/rust/api/node.dart' as rust;

/// Android notification access bridge and FreeLife transport adapter.
class NotificationBridge {
  NotificationBridge([MethodChannel? channel])
    : _channel = channel ?? const MethodChannel(channelName) {
    _channel.setMethodCallHandler(_onNativeCall);
  }

  static const channelName = 'dev.freelife/notifications';

  final MethodChannel _channel;
  StreamSubscription<rust.NotificationCommand>? _commands;
  void Function(Object error)? _onError;

  Future<bool?> hasListenerAccess() async {
    try {
      final access = await _channel.invokeMethod<bool>('hasListenerAccess');
      await _channel.invokeMethod<void>('ready');
      return access;
    } on MissingPluginException {
      return null;
    }
  }

  Future<void> openListenerSettings() =>
      _channel.invokeMethod<void>('openListenerSettings');

  void start({required void Function(Object error) onError}) {
    _onError = onError;
    _commands ??= rust.notificationCommands().listen(
      _handleCommand,
      onError: (Object error) => _report(error),
    );
  }

  Future<Object?> _onNativeCall(MethodCall call) async {
    if (call.method != 'notificationPosted' &&
        call.method != 'notificationRemoved') {
      return null;
    }
    final args = Map<Object?, Object?>.from(call.arguments as Map);
    try {
      await rust.notificationPublish(
        id: args['id'] as String,
        appPackage: args['appPackage'] as String,
        appName: args['appName'] as String,
        title: args['title'] as String,
        body: args['body'] as String,
        timestamp: (args['timestamp'] as num).toInt(),
        iconBytes: args['iconBytes'] as Uint8List?,
        hasReplyAction: args['hasReplyAction'] as bool,
        replyToken: args['replyToken'] as String?,
        removed: args['removed'] as bool,
      );
    } on Object catch (error) {
      _report(error);
    }
    return null;
  }

  Future<void> _handleCommand(rust.NotificationCommand command) async {
    try {
      if (command.replyToken != null && command.text != null) {
        final submitted = await _channel.invokeMethod<bool>('reply', {
          'replyToken': command.replyToken,
          'text': command.text,
        });
        if (submitted != true) {
          _report(
            StateError('The notification reply action is no longer available.'),
          );
        }
      } else if (command.notificationId != null) {
        final dismissed = await _channel.invokeMethod<bool>('dismiss', {
          'id': command.notificationId,
        });
        if (dismissed != true) {
          _report(StateError('The notification could not be dismissed.'));
        }
      }
    } on Object catch (error) {
      _report(error);
    }
  }

  void _report(Object error) => _onError?.call(error);

  Future<void> dispose() async {
    _channel.setMethodCallHandler(null);
    await _commands?.cancel();
  }
}
