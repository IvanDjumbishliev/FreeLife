import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:freelife/main.dart';
import 'package:freelife/platform/device_channel.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  const channel = MethodChannel(DeviceChannel.channelName);

  setUp(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
      switch (call.method) {
        case 'getDeviceInfo':
          return {'manufacturer': 'Acme', 'model': 'Phone 1', 'osVersion': '15'};
        case 'getBatteryLevel':
          return 42;
      }
      return null;
    });
  });

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, null);
  });

  testWidgets('shows device info from the platform channel', (tester) async {
    await tester.pumpWidget(const FreeLifeApp());
    await tester.pumpAndSettle();

    expect(find.text('FreeLife'), findsOneWidget);
    expect(find.text('Acme Phone 1'), findsOneWidget);
    expect(find.text('OS 15 · battery 42%'), findsOneWidget);
    expect(find.text('1'), findsOneWidget); // protocol version
  });
}
