#include "app.h"
#include "backend_wayland.h"
#include <QProcess>
#include <stdexcept>
#include <cstring>

AppState g;
static void check(bool value, const char* message) { if (!value) throw std::runtime_error(message); }
static void waitFor(const std::function<bool()>& ready) {
  QElapsedTimer timer; timer.start();
  while (!ready() && timer.elapsed() < 5000) { QCoreApplication::processEvents(); QThread::msleep(5); }
  check(ready(), "protocol update timed out");
}
static void cachePages() {
  PageCache cache(1); // Force private disk spill, including translucent pixels.
  for (int page = 1; page <= 45; ++page) {
    QPixmap pixmap(32, 24); pixmap.fill(QColor(page, 140, 60, 97)); cache.insert(page, pixmap);
  }
  for (int page = 45; page >= 1; --page) {
    QPixmap expected(32, 24); expected.fill(QColor(page, 140, 60, 97));
    check(cache.value(page).toImage() == expected.toImage(), "page lost or changed after spill");
  }
  cache.remove(20); check(!cache.contains(20), "clear resurrects cached ink");
  cache.clear(); check(cache.size() == 0, "clear retains pages");
  g.canvas = new QPixmap(64, 48);
  g.canvas->fill(Qt::red); g.pageHasInk = true; g.currentSlide = 3;
  initCanvas();
  check(g.canvas->toImage().pixelColor(5, 5) == QColor(Qt::red), "legacy canvas resize erased ink");
  toggleWhiteboard(); g.canvas->fill(Qt::blue); g.pageHasInk = true;
  toggleWhiteboard(); check(g.currentSlide == 3, "wrong restored overlay page");
  check(g.canvas->toImage().pixelColor(5, 5) == QColor(Qt::red), "overlay ink lost");
  toggleWhiteboard(); check(g.canvas->toImage().pixelColor(5, 5) == QColor(Qt::blue), "whiteboard ink lost");
  setWpsDebug(true, false);
  check(g.wpsBridge != nullptr, "legacy WPS bridge did not start");
  g.currentMode = 2;
  g.wpsBridge->onSlideshowEnd();
  check(g.currentMode == 2 && g.canvas->toImage().pixelColor(5, 5) == QColor(Qt::blue), "show end interrupted the whiteboard");
  toggleWhiteboard();
  check(g.currentMode == 0, "leaving the board restored an ended show's tool");
  for (int mode : {1, 2}) {
    g.currentMode = mode;
    g.wpsBridge->onSlideshowEnd();
    check(g.currentMode == 0, "legacy overlay kept the drawing tool after show end");
  }
  stopWpsApiServer();
  g.pageHasInk = false; saveCurrentPage();
  delete g.canvas; g.canvas = nullptr;
}
static void nativeGeometry() {
  check(qgetenv("SIDERA_TEST_ISOLATED") == "1", "private compositor required");
  WlBackend backend;
  check(backend.init(), "native C++ backend failed to initialize");
  if (backend.wps_) backend.wps_->stop();
  backend.ink_.fill(Qt::red);
  backend.renderAndCommit();
  waitFor([&]() { return !backend.bufs_[0].busy || !backend.bufs_[1].busy; });
  // Hold both slots on the client side without dispatching compositor events.
  // No storage is overwritten while held, even if the requested geometry changes.
  bool busy0 = backend.bufs_[0].busy, busy1 = backend.bufs_[1].busy;
  backend.bufs_[0].busy = backend.bufs_[1].busy = true;
  const QByteArray oldBytes(static_cast<const char*>(backend.shmData_), int(backend.shmSize_));
  const int oldWidth = backend.logicalW_;
  backend.logicalW_ += 64;
  backend.renderAndCommit();
  check(backend.pendingRender_, "busy geometry update was discarded");
  check(QByteArray(static_cast<const char*>(backend.shmData_), int(backend.shmSize_)) == oldBytes, "busy storage was overwritten");
  backend.logicalW_ = oldWidth;
  backend.bufs_[0].busy = busy0; backend.bufs_[1].busy = busy1;
  backend.renderAndCommit();
  for (const auto& mode : {QString("1024x768"), QString("640x480"), QString("800x600")}) {
    qInfo() << "Checking private output resize" << mode;
    QProcess process; process.start("swaymsg", {"output", "HEADLESS-1", "mode", mode});
    check(process.waitForFinished(3000) && process.exitCode() == 0, "headless resize failed");
    const auto dimensions = mode.split('x');
    const int width = dimensions[0].toInt(), height = dimensions[1].toInt();
    waitFor([&]() { return backend.logicalW_ == width && backend.logicalH_ == height &&
        backend.bufferW_ == backend.width_ && backend.bufferH_ == backend.height_ && !backend.pendingRender_; });
    check(backend.shmSize_ == size_t(backend.bufferW_) * backend.bufferH_ * 8, "buffer geometry/storage disagree");
    check(backend.ink_.size() == QSize(width, height), "ink size did not follow output");
    check(backend.ink_.toImage().pixelColor(width / 2, height / 2) == QColor(Qt::red), "resize erased ink");
  }
  for (const auto& scale : {QString("1.25"), QString("2"), QString("1")}) {
    QProcess process; process.start("swaymsg", {"output", "HEADLESS-1", "scale", scale});
    check(process.waitForFinished(3000) && process.exitCode() == 0, "headless scale failed");
    const int width = qRound(800 / scale.toDouble()), height = qRound(600 / scale.toDouble());
    waitFor([&]() { return backend.logicalW_ == width && backend.logicalH_ == height &&
        backend.bufferW_ == 800 && backend.bufferH_ == 600 && !backend.pendingRender_; });
    check(backend.ink_.toImage().pixelColor(width / 2, height / 2) == QColor(Qt::red), "scale erased ink");
  }
  backend.touchId_ = 4; backend.drawing_ = true; backend.draggingSlider_ = 2;
  backend.cancelInput();
  check(backend.touchId_ == -1 && !backend.drawing_ && backend.draggingSlider_ == 0, "touch cancellation left active input");
  for (int mode : {1, 2}) {
    backend.setMode(mode);
    backend.togglePopup(mode);
    backend.drawing_ = true;
    backend.wps_->onSlideshowEnd();
    check(backend.mode_ == 0 && !backend.drawing_ && !backend.popupType_, "show end kept the drawing tool active");
  }
  backend.setMode(2);
  backend.toggleWhiteboard();
  backend.wps_->onSlideshowEnd();
  check(backend.mode_ == 2, "show end interrupted the whiteboard eraser");
  backend.toggleWhiteboard();
  check(backend.mode_ == 0, "leaving the board restored an ended show's eraser");
}
int main(int argc, char** argv) {
  QApplication application(argc, argv);
  try { cachePages(); nativeGeometry(); qInfo() << "PASS C++ cache, whiteboard, busy storage, resize and cancel"; }
  catch (const std::exception& error) { qCritical() << error.what(); return 1; }
  return 0;
}
