// Sidera - 启动页
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
#include "app.h"

namespace {
constexpr int cardWidth = 560;
constexpr int cardHeight = 332;
constexpr qint64 localReportMs = 720;
constexpr qint64 linkReportMs = 210;
}

StartupSplash::StartupSplash()
    : artwork(QStringLiteral(":/sidera/startup-ink.png")), mark(sideraIconPixmap(48)) {
  setWindowFlags(Qt::Tool | Qt::FramelessWindowHint | Qt::WindowStaysOnTopHint | Qt::WindowDoesNotAcceptFocus);
  setAttribute(Qt::WA_TranslucentBackground);
  setAttribute(Qt::WA_ShowWithoutActivating);
  setAttribute(Qt::WA_DeleteOnClose);
  QScreen* screen = QGuiApplication::primaryScreen();
  const QRect available = screen->availableGeometry();
  QSize size(cardWidth, cardHeight);
  const QSize room(qMax(1, available.width() - 24), qMax(1, available.height() - 24));
  size.scale(size.boundedTo(room), Qt::KeepAspectRatio);
  setFixedSize(size);
  move(available.center() - rect().center());
  const QSize physical = screen->size() * screen->devicePixelRatio();
  displayLine = QStringLiteral("已连接桌面 · X11 · %1 × %2").arg(physical.width()).arg(physical.height());

  elapsed.start();
  auto* frames = new QTimer(this);
  connect(frames, &QTimer::timeout, this, [this]() {
    const qint64 now = elapsed.elapsed();
    if (readyAt >= 0) {
      const qint64 fade = now - qMax<qint64>(1100, readyAt + 250);
      if (fade >= 200) { close(); return; }
      if (fade > 0) { setWindowOpacity(1.0 - fade / 200.0); return; }
    }
    update(0, 240 * height() / cardHeight, width(), 50 * height() / cardHeight + 2);
  });
  frames->start(16);
}

void StartupSplash::reportReady() {
  if (readyAt < 0) readyAt = elapsed.elapsed();
}

void StartupSplash::paintEvent(QPaintEvent*) {
  QPainter painter(this);
  painter.setRenderHints(QPainter::Antialiasing | QPainter::SmoothPixmapTransform);
  painter.scale(width() / qreal(cardWidth), height() / qreal(cardHeight));
  const bool dark = palette().window().color().lightness() < 128;
  const QColor paper(dark ? "#202126" : "#fcfcfd"), ink(dark ? "#f2f2f5" : "#25262b");
  const QColor muted(dark ? "#bfc1ca" : "#62646d");
  painter.setPen(Qt::NoPen);
  painter.setBrush(paper);
  painter.drawRoundedRect(QRectF(0, 212, cardWidth, cardHeight - 212), 12, 12);
  painter.drawPixmap(QRectF(0, 0, cardWidth, 224), artwork, QRectF(artwork.rect()));

  const qint64 now = elapsed.elapsed();
  qreal progress = .74 * qBound(0.0, now / qreal(localReportMs), 1.0);
  if (readyAt >= 0) {
    progress += .26 * qBound(0.0, (now - qMax(readyAt, localReportMs)) / qreal(linkReportMs), 1.0);
  }
  const QRectF track(30, 246, 500, 4);
  painter.setBrush(QColor(dark ? "#4a4d58" : "#d5d7dd"));
  painter.drawRoundedRect(track, 2, 2);
  if (progress > 0) {
    const qreal fill = track.width() * progress;
    const qreal radius = qMin<qreal>(2.0, fill / 2);
    painter.setBrush(QColor(dark ? "#8cd6b5" : "#216f62"));
    painter.drawRoundedRect(QRectF(track.x(), track.y(), fill, track.height()), radius, radius);
  }

  QString status = displayLine;
  if (progress >= .74) {
    if (readyAt < 0) status = QStringLiteral("正在连接 WPS 演示");
    else if (!g.wpsDebug) status = QStringLiteral("工具栏已就绪");
    else if (!g.wpsBridge) status = QStringLiteral("WPS 联动暂不可用，可继续批注");
    else if (g.wpsConnected) status = QStringLiteral("已连接 WPS 演示");
    else status = QStringLiteral("工具栏已就绪，等待 WPS 演示");
  } else if (progress >= .32) {
    if (readyAt < 0) status = QStringLiteral("正在准备批注工具");
    else status = g.hotkeyOk ? QStringLiteral("批注工具已就绪 · Ctrl+Shift+D")
                            : QStringLiteral("批注工具已就绪，全局快捷键不可用");
  }
  QFont font = painter.font();
  font.setPixelSize(15);
  painter.setFont(font);
  painter.setPen(ink);
  painter.drawText(QPointF(30, 277), status);

  if (!mark.isNull()) painter.drawPixmap(QRectF(30, 298, 20, 20), mark, QRectF(mark.rect()));
  font.setPixelSize(11);
  painter.setFont(font);
  painter.setPen(muted);
  painter.drawText(QPointF(mark.isNull() ? 30 : 58, 313), QStringLiteral("项目作者 Carl_Jin"));
  const QString credits = QStringLiteral("定制开发 Gary_6Stones · 教程与反馈 ofkuyu");
  painter.drawText(QPointF(530 - painter.fontMetrics().horizontalAdvance(credits), 313), credits);
}
