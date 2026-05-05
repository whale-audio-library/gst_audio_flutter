import 'package:flutter_test/flutter_test.dart';
import 'package:gst_audio_flutter/main.dart';
import 'package:gst_audio_flutter/src/rust/frb_generated.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => await RustLib.init());
  testWidgets('shows player shell', (WidgetTester tester) async {
    await tester.pumpWidget(const AudioPlayerApp());
    await tester.pumpAndSettle();
    expect(find.text('GStreamer Audio'), findsWidgets);
    expect(find.text('Queue'), findsOneWidget);
  });
}
