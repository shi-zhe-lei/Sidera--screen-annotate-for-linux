// Sidera - 弹窗/子菜单/截图
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
#include "app.h"

// 8. 弹窗基类（子控件，在 mainWidget 内）
// ============================================================
static QWidget* createPopup(int w, int h) {
  QWidget* pop = new QWidget(g.mainWidget);
  pop->setFixedSize(w, h);
  pop->setObjectName("popup");
  pop->setStyleSheet(
    "#popup { background-color: rgba(42,42,50,245); border: 2px solid #666666; border-radius: 14px; }"
    "QPushButton { background-color: #444444; color: #ffffff; border: 2px solid #666666; border-radius: 6px;"
    "font-size: 14px; font-weight: bold; padding: 8px; }"
    "QPushButton:hover { background-color: #555555; }"
    "QLabel { color: #cccccc; font-size: 12px; font-weight: bold; }"
  );
  pop->hide();
  return pop;
}

void showPenPopup() {
  if (g.penPopup) { g.penPopup->deleteLater(); g.penPopup = nullptr; }
  QWidget* pop = createPopup(200, 268);
  g.penPopup = pop;
  QVBoxLayout* lay = new QVBoxLayout(pop);
  lay->setContentsMargins(12,10,12,10); lay->setSpacing(8);

  lay->addWidget(new QLabel(QString::fromUtf8("画笔颜色")));
  // 颜色两行显示，每行 4 个
  {
    const int perRow = 4;
    QHBoxLayout* row = nullptr;
    for (int i = 0; i < g.colors.size(); i++) {
      if (i % perRow == 0) { row = new QHBoxLayout(); row->setSpacing(8); lay->addLayout(row); }
      QPushButton* cb = new QPushButton(); cb->setFixedSize(30,30);
      QString hex = g.colors[i].name();
      cb->setStyleSheet(QString("QPushButton{background:%1;border:%2px solid %3;border-radius:15px;}")
        .arg(hex).arg(i==g.curColor?3:2).arg(i==g.curColor?"#ffffff":"#666666"));
      QObject::connect(cb, &QPushButton::clicked, [i](){ g.curColor=i; showPenPopup(); repositionPopups(); });
      if (row) row->addWidget(cb);
    }
    if (g.colors.size() % perRow != 0) row->addStretch();
  }

  lay->addWidget(new QLabel(QString::fromUtf8("画笔粗细")));
  QHBoxLayout* sr = new QHBoxLayout(); sr->setSpacing(8);
  QStringList sl = {QString::fromUtf8("细 3"), QString::fromUtf8("中 6"), QString::fromUtf8("粗 10")};
  for (int i=0;i<g.penSizes.size();i++) {
    QPushButton* sb = new QPushButton(sl[i]); sb->setFixedHeight(36);
    if (i==g.curPen) sb->setStyleSheet("QPushButton{background:#3377cc;color:#fff;border:2px solid #5599ff;border-radius:6px;font-size:14px;font-weight:bold;}");
    QObject::connect(sb, &QPushButton::clicked, [i](){ g.curPen=i; showPenPopup(); repositionPopups(); });
    sr->addWidget(sb);
  }
  lay->addLayout(sr);
  pop->show(); pop->raise();
  repositionPopups();
  if (g.currentMode == 0) setInputShapeToSidebar();
}

void showEraserPopup() {
  if (g.eraserPopup) { g.eraserPopup->deleteLater(); g.eraserPopup = nullptr; }
  QWidget* pop = createPopup(220, 175);
  g.eraserPopup = pop;
  QVBoxLayout* lay = new QVBoxLayout(pop);
  lay->setContentsMargins(12,10,12,10); lay->setSpacing(8);

  lay->addWidget(new QLabel(QString::fromUtf8("橡皮擦大小")));
  QHBoxLayout* sr = new QHBoxLayout(); sr->setSpacing(8);
  QStringList sl = {QString::fromUtf8("小 12"), QString::fromUtf8("中 24"), QString::fromUtf8("大 48")};
  for (int i=0;i<g.eraserSizes.size();i++) {
    QPushButton* sb = new QPushButton(sl[i]); sb->setFixedHeight(36);
    if (i==g.curEraser) sb->setStyleSheet("QPushButton{background:#ff8800;color:#000;border:2px solid #ffaa00;border-radius:6px;font-size:14px;font-weight:bold;}");
    QObject::connect(sb, &QPushButton::clicked, [i](){ g.curEraser=i; showEraserPopup(); repositionPopups(); });
    sr->addWidget(sb);
  }
  lay->addLayout(sr);
  lay->addSpacing(6);
  QPushButton* cb = new QPushButton(QString::fromUtf8("清空当前页"));
  cb->setFixedHeight(38);
  cb->setStyleSheet("QPushButton{background:#555;font-size:14px;font-weight:bold;border-radius:6px;}QPushButton:hover{background:#666;}");
  QObject::connect(cb, &QPushButton::clicked, [](){ clearCanvas(); });
  lay->addWidget(cb);
  pop->show(); pop->raise();
  repositionPopups();
  if (g.currentMode == 0) setInputShapeToSidebar();
}

void closeAllPopups() {
  g.penPopupVisible = g.eraserPopupVisible = false;
  if (g.penPopup) { g.penPopup->hide(); g.penPopup->deleteLater(); g.penPopup = nullptr; }
  if (g.eraserPopup) { g.eraserPopup->hide(); g.eraserPopup->deleteLater(); g.eraserPopup = nullptr; }
  // 光标模式下重新计算输入区域（去掉弹窗矩形）
  if (g.currentMode == 0) setInputShapeToSidebar();
}

void repositionPopups() {
  QWidget* ref = g.popupOnRight ? g.sidebarAreaRight : g.sidebarArea;
  if (!ref) return;
  QPoint sb = ref->pos();
  int sbW = ref->width(), sbH = ref->height();
  int parentW = g.mainWidget ? g.mainWidget->width() : 1920;
  auto pos = [&](QWidget* p) {
    if (!p || !p->isVisible()) return;
    int y = sb.y() + (sbH - p->height())/2;
    y = qMax(0, qMin(y, (g.mainWidget?g.mainWidget->height():1080) - p->height()));
    int x = g.popupOnRight ? (sb.x() - p->width() - 8) : (sb.x() + sbW + 8);
    p->move(qMax(0, qMin(x, parentW - p->width())), y);
  };
  pos(g.penPopup); pos(g.eraserPopup);
}

static void doScreenshot() {
  QScreen* sc = QGuiApplication::primaryScreen();
  if (!sc) { qWarning() << "[WARN] 截图失败：无屏幕"; return; }
  QPixmap pm = sc->grabWindow(0);
  if (pm.isNull()) { qWarning() << "[WARN] 截图失败：抓取为空"; return; }
  QString dir = QStandardPaths::writableLocation(QStandardPaths::PicturesLocation);
  if (dir.isEmpty()) dir = QDir::homePath();
  QDir().mkpath(dir);
  QString fn = dir + "/sidera_" + QDateTime::currentDateTime().toString("yyyyMMdd_hhmmss") + ".png";
  if (pm.save(fn, "PNG")) {
    qDebug() << "[INFO] 截图已保存:" << fn;
    QGuiApplication::clipboard()->setImage(pm.toImage());
  } else {
    qWarning() << "[WARN] 截图保存失败:" << fn;
  }
}

// 清除当前页笔迹（清画布 + 丢弃当前页缓存）
void clearCurrentStrokes() {
  activeCache().remove(g.currentSlide);
  if (g.canvas) g.canvas->fill(Qt::transparent);
  g.pageHasInk = false;
  clearUndo();
  if (g.mainWidget) g.mainWidget->update();
  qDebug() << "[INFO] 已清除当前页笔迹";
}

// 更多功能子菜单（省略号按钮）——先放“截图”，后续可扩展
void showMoreMenu(QPushButton* b) {
  if (!b) return;
  QMenu m;
  QAction* shot = m.addAction(QString::fromUtf8("截图"));
  QAction* set  = m.addAction(QString::fromUtf8("设置"));
  m.setStyleSheet("QMenu{background:#2b2b33;color:#eee;} QMenu::item{padding:8px 24px;} QMenu::item:selected{background:#444;}");
  QAction* chosen = m.exec(b->mapToGlobal(QPoint(0, b->height())));
  if (chosen == shot) doScreenshot();
  else if (chosen == set) openSettings();
}
