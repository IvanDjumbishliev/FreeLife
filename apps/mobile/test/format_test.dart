import 'package:flutter_test/flutter_test.dart';
import 'package:freelife/util/format.dart';

void main() {
  test('formatBytes', () {
    expect(formatBytes(0), '0 B');
    expect(formatBytes(1023), '1023 B');
    expect(formatBytes(1536), '1.5 KB');
    expect(formatBytes(50 * 1024 * 1024), '50.0 MB');
    expect(formatBytes(150 * 1024 * 1024), '150 MB');
  });

  test('formatEta', () {
    expect(formatEta(null), 'estimating…');
    expect(formatEta(9), '9s left');
    expect(formatEta(125), '2m 5s left');
    expect(formatEta(3 * 3600 + 120), '3h 2m left');
  });

  test('basename handles both separators', () {
    expect(basename('/a/b/c.txt'), 'c.txt');
    expect(basename(r'C:\x\y.doc'), 'y.doc');
  });
}
