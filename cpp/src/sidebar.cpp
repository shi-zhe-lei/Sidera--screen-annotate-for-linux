// Sidera - 侧边栏尺寸/样式/收缩展开
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
#include "app.h"
#include "widget.h"

// ============================================================
// 7. 侧边栏尺寸参数（线性缩放，基准为 64px 宽 / 38px 按钮）
// ============================================================
int sbWidth()  { return int(56 * g.sbScale); }
int sbBtn()    { return int(34 * g.sbScale); }
int sbIcon()   { return int(24 * g.sbScale); }
int sbDot()    { return int(19 * g.sbScale); }
// 高度 = 固定 margins/spacing + 8 个按钮（间距 7 个 + 上下边距 18/14）
// 高度 = 上下边距(12+10) + 11 个普通按钮 + 10 个间距(6)
int sbHeight() { return 22 + 11 * sbBtn() + 10 * 6; }

void updateSidebarStyles() {
  auto style = [](int m) {
    if (g.currentMode == m) {
      // 选中模式：半透明蒂芙尼蓝 + 加亮边框
      return QString("QPushButton{background:rgba(10,186,181,0.40);border:2px solid #7fe9e4;border-radius:19px;}"
                     "QPushButton:hover{background:rgba(10,186,181,0.55);}");
    }
    return QString("QPushButton{background:transparent;border:2px solid transparent;border-radius:19px;}"
                   "QPushButton:hover{background:rgba(255,255,255,0.1);}");
  };
  if (g.cursorBtn) g.cursorBtn->setStyleSheet(style(0));
  if (g.penBtn)    g.penBtn->setStyleSheet(style(1));
  if (g.eraserBtn) g.eraserBtn->setStyleSheet(style(2));
  if (g.cursorBtnR) g.cursorBtnR->setStyleSheet(style(0));
  if (g.penBtnR)    g.penBtnR->setStyleSheet(style(1));
  if (g.eraserBtnR) g.eraserBtnR->setStyleSheet(style(2));
}
// ============================================================
// 白板模式：整窗不透明（白底）且全屏可输入；再点还原透明可穿透
// ============================================================
void updateWhiteboardButtonStyles() {
  auto paint = [](QPushButton* b) {
    if (!b) return;
    if (g.whiteboard)
      b->setStyleSheet(QString("QPushButton{background:#f4f4f4;color:#111;border:2px solid #ff9800;border-radius:%1px;font-weight:bold;font-size:%2px;}"
                       "QPushButton:hover{background:#ffffff;}").arg(int(sbBtn()*0.35)).arg(sbBtn()*16/38));
    else
      b->setStyleSheet(QString("QPushButton{background:#555;color:#eee;border:2px solid #999;border-radius:%1px;font-size:%2px;}"
                       "QPushButton:hover{background:#777;}").arg(int(sbBtn()*0.35)).arg(sbBtn()*16/38));
  };
  paint(g.wbBtnL);
  paint(g.wbBtnR);
}

// 退出放映键：普通模式=退出放映；白板模式=背景颜色切换键（并显示当前背景色）
void updateExitButtons() {
  auto paint = [](QPushButton* b) {
    if (!b) return;
    if (g.whiteboard) {
      QColor c = whiteboardBgColor();
      QString fg = (c.lightness() > 128) ? "#111111" : "#ffffff";
      b->setText(QString::fromUtf8("背景\n颜色"));
      b->setStyleSheet(QString(
        "QPushButton{background:%1;color:%2;border:2px solid #ffffff;border-radius:4px;font-weight:bold;font-size:%3px;}"
        "QPushButton:hover{background:%1;}").arg(c.name()).arg(fg).arg(sbBtn()*11/34));
    } else {
      b->setText(QString::fromUtf8("退出\n放映"));
      b->setStyleSheet(QString(
        "QPushButton{background:#d33a3a;color:#ffffff;border:2px solid #ff8080;"
        "border-radius:4px;font-weight:bold;font-size:%1px;}"
        "QPushButton:hover{background:#ff4d4d;}").arg(sbBtn()*12/34));
    }
  };
  paint(g.exitBtnL);
  paint(g.exitBtnR);
}

void toggleWhiteboard() {
  saveCurrentPage();
  if (!g.whiteboard) {
    // 进入白板：保存普通/放映页，恢复上次白板页及笔迹。
    g.savedSlide = g.currentSlide;     // 记住普通模式页码
    g.whiteboard = true;
    g.currentSlide = g.whiteboardSlide;
    loadPage(g.currentSlide);
  } else {
    // 退出白板：两套缓存独立保留，恢复普通/放映页。
    g.whiteboardSlide = g.currentSlide;
    g.whiteboard = false;
    if (g.cursorAfterWhiteboard) {
      g.cursorAfterWhiteboard = false;
      if (g.currentMode != 0) switchToCursorMode();
    }
    g.currentSlide = g.savedSlide;
    if (g.canvas) g.canvas->fill(Qt::transparent);
    loadPage(g.currentSlide);          // 从 slideCache 恢复
  }
  updateWhiteboardButtonStyles();
  if (g.wpsBridge) g.wpsBridge->setWhiteboard(g.whiteboard);
  updateExitButtons();
  clearUndo();
  if (g.mainWidget) {
    if (g.whiteboard) resetInputShape();          // 全屏可输入
    else if (g.currentMode == 0) setInputShapeToSidebar();
    else resetInputShape();
    g.mainWidget->update();                       // 触发白底/透明重绘
  }
  qDebug() << (g.whiteboard ? "[INFO] 白板模式开启（恢复白板笔迹）"
                            : "[INFO] 白板模式关闭（两套笔迹均保留）");
}

// 退出放映：先关白板、切回光标模式（让 PPT 自带控件可用），再发 ESC
// 当前模式归位光标、笔迹不清空 —— 见 collapseSidebars
static int collapsedH() { return sbBtn() * 3 + 12; }   // 竖胶囊高度（容纳 5 个竖排字）

void updateCollapseButtons() {
  QString t = g.collapsed ? QString::fromUtf8("展\n开") : QString::fromUtf8("收起");
  if (g.collapseBtnL) g.collapseBtnL->setText(t);
  if (g.collapseBtnR) g.collapseBtnR->setText(t);
}

void collapseSidebars() {
  if (g.collapsed || !g.sidebarArea) return;
  g.collapsed = true;
  if (g.currentMode != 0) switchToCursorMode();   // 收缩时归位光标模式，笔迹不清空
  QWidget* sbs[2] = { g.sidebarArea, g.sidebarAreaRight };
  for (QWidget* sb : sbs) {
    if (!sb) continue;
    for (QWidget* w : sb->findChildren<QWidget*>()) {
      if (w == g.collapseBtnL || w == g.collapseBtnR) continue;
      w->hide();
    }
    if (auto* lay = qobject_cast<QVBoxLayout*>(sb->layout())) {
      lay->setContentsMargins(4, 6, 4, 6); lay->setSpacing(0);
    }
    sb->setFixedSize(sbWidth(), collapsedH());
  }
  if (g.collapseBtnL) g.collapseBtnL->setFixedSize(sbWidth() - 8, collapsedH() - 12);
  if (g.collapseBtnR) g.collapseBtnR->setFixedSize(sbWidth() - 8, collapsedH() - 12);
  // 保持在屏幕内
  QRect scr = QGuiApplication::primaryScreen()->geometry();
  int y = qMin(qMax(g.sidebarScreenPos.y(), 0), qMax(0, scr.height() - collapsedH()));
  if (g.sidebarArea) g.sidebarArea->move(4, y);
  if (g.sidebarAreaRight) g.sidebarAreaRight->move(scr.width() - sbWidth() - 4, y);
  g.sidebarScreenPos = QPoint(4, y);
  g.sidebarScreenPosR = QPoint(scr.width() - sbWidth() - 4, y);
  updateCollapseButtons();
  if (g.mainWidget) g.mainWidget->update();
  if (g.currentMode == 0) setInputShapeToSidebar();
  if (g.sidebarIdleTimer) g.sidebarIdleTimer->start(2 * 60 * 1000);   // 收缩态：2 分钟后自动展开
  qDebug() << "[INFO] 侧边栏已收缩";
}

void expandSidebars() {
  if (!g.collapsed || !g.sidebarArea) return;
  g.collapsed = false;
  if (g.sidebarIdleTimer) g.sidebarIdleTimer->stop();
  QWidget* sbs[2] = { g.sidebarArea, g.sidebarAreaRight };
  for (QWidget* sb : sbs) {
    if (!sb) continue;
    for (QWidget* w : sb->findChildren<QWidget*>()) w->show();
    if (auto* lay = qobject_cast<QVBoxLayout*>(sb->layout())) {
      lay->setContentsMargins(8, 12, 8, 10); lay->setSpacing(6);
    }
    sb->setFixedSize(sbWidth(), sbHeight());
  }
  if (g.collapseBtnL) g.collapseBtnL->setFixedSize(sbBtn(), sbBtn());
  if (g.collapseBtnR) g.collapseBtnR->setFixedSize(sbBtn(), sbBtn());
  QRect scr = QGuiApplication::primaryScreen()->geometry();
  int y = qMin(qMax(g.sidebarScreenPos.y(), 0), qMax(0, scr.height() - sbHeight()));
  g.sidebarArea->move(4, y);
  if (g.sidebarAreaRight) g.sidebarAreaRight->move(scr.width() - sbWidth() - 4, y);
  g.sidebarScreenPos  = QPoint(4, y);
  g.sidebarScreenPosR = QPoint(scr.width() - sbWidth() - 4, y);
  updateCollapseButtons();
  if (g.mainWidget) g.mainWidget->update();
  if (g.currentMode == 0) setInputShapeToSidebar();
  qDebug() << "[INFO] 侧边栏已展开";
}

void toggleSidebarCollapse() {
  if (g.collapsed) expandSidebars(); else collapseSidebars();
}

void rebuildSidebars() {
  if (!g.mainWidget) return;
  int keepY = g.sidebarScreenPos.y();
  MainWidget* mw = static_cast<MainWidget*>(g.mainWidget);
  // 删除旧侧边栏
  delete g.sidebarArea; g.sidebarArea = nullptr;
  delete g.sidebarAreaRight; g.sidebarAreaRight = nullptr;
  g.cursorBtn = g.penBtn = g.eraserBtn = nullptr;
  g.cursorBtnR = g.penBtnR = g.eraserBtnR = nullptr;
  g.prevBtn = g.nextBtn = nullptr;
  g.wbBtnL = g.wbBtnR = nullptr;
  g.collapseBtnL = g.collapseBtnR = nullptr;
  g.collapsed = false;              // 重建后回到展开态
  // 重建
  mw->buildSidebar();
  // 恢复贴边位置（Y 保留原值，约束在新高度内）
  QRect scr = QGuiApplication::primaryScreen()->geometry();
  int iy = qMin(qMax(keepY, 0), scr.height() - sbHeight());
  g.sidebarArea->move(4, iy);
  g.sidebarAreaRight->move(scr.width() - sbWidth() - 4, iy);
  g.sidebarScreenPos  = QPoint(4, iy);
  g.sidebarScreenPosR = QPoint(scr.width() - sbWidth() - 4, iy);
  if (g.currentMode == 0) setInputShapeToSidebar();
}

// 开机自启动：检测 .desktop 是否存在于 autostart
