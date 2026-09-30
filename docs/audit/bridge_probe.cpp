// Diagnostic harness: runs the repository's unmodified C++ WpsBridge.
#include "wps_bridge.h"
#include <QCoreApplication>
#include <QTimer>
#include <cstdio>

int main(int argc, char** argv) {
  QCoreApplication app(argc, argv);
  WpsBridge bridge;
  bridge.onSlideshowBegin = [](int pos) {
    std::printf("BEGIN %d\n", pos); std::fflush(stdout);
  };
  bridge.onRealPos = [](int pos) {
    std::printf("POS %d\n", pos); std::fflush(stdout);
  };
  for (int i = 0; i < 3; ++i) {
    if (!bridge.start()) return 2;
    if (i < 2) bridge.stop();
  }
  int active = 0;
  for (auto* timer : bridge.findChildren<QTimer*>()) {
    if (timer->isActive()) ++active;
  }
  std::printf("ACTIVE_TIMERS %d\n", active); std::fflush(stdout);
  bridge.enqueue("NEXT");
  QTimer::singleShot(30000, &app, &QCoreApplication::quit);
  return app.exec();
}
