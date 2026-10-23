const _units = ['B', 'KB', 'MB', 'GB', 'TB'];

String formatBytes(int bytes) {
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1024 && unit < _units.length - 1) {
    value /= 1024;
    unit++;
  }
  final digits = unit == 0 || value >= 100 ? 0 : 1;
  return '${value.toStringAsFixed(digits)} ${_units[unit]}';
}

String formatSpeed(int bytesPerSecond) => '${formatBytes(bytesPerSecond)}/s';

String formatEta(int? secs) {
  if (secs == null) return 'estimating…';
  if (secs < 60) return '${secs}s left';
  final m = secs ~/ 60;
  if (m < 60) return '${m}m ${secs % 60}s left';
  return '${m ~/ 60}h ${m % 60}m left';
}

String basename(String path) => path.split(RegExp(r'[\\/]')).last;
