#pragma once
// ============================================================
// Sidera - 侧边栏控件与主窗口类
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
// ============================================================
#include "app.h"

// ============================================================
// 6. 侧边栏磨砂背景画笔
// ============================================================
class SidebarPainter : public QObject {
  QWidget* w;
public:
  explicit SidebarPainter(QWidget* widget) : QObject(widget), w(widget) {}
protected:
  bool eventFilter(QObject* obj, QEvent* ev) override {
    if (ev->type() == QEvent::Paint && obj == w) {
      QPainter p(w);
      p.setRenderHint(QPainter::Antialiasing, true);
      QRectF r = w->rect().adjusted(1,1,-1,-1);
      qreal rad = r.width()/2.0;
      p.setBrush(QColor(42,42,50,g.sidebarAlpha));
      p.setPen(Qt::NoPen);
      p.drawRoundedRect(r, rad, rad);
      QLinearGradient g(r.topLeft(), QPointF(r.center().x(), r.top()+r.height()*0.45));
      g.setColorAt(0.0, QColor(255,255,255,20));
      g.setColorAt(0.3, QColor(255,255,255,8));
      g.setColorAt(1.0, QColor(255,255,255,0));
      p.setBrush(g); p.setPen(Qt::NoPen);
      p.drawRoundedRect(r, rad, rad);
      p.setBrush(Qt::NoBrush);
      p.setPen(QPen(QColor(90,90,100), 2.0));
      p.drawRoundedRect(r, rad, rad);
      p.end();
      return false;
    }
    return QObject::eventFilter(obj, ev);
  }
};

// ============================================================
// 7b. 侧边栏拖动（子控件，在父窗口内自由移动）
// ============================================================
class SidebarDragFilter : public QObject {
  bool dragging = false;
  bool moved = false;
  bool collapsedPress = false;
  int  dragStartY = 0;
  int  dragGlobalYStart = 0;
  bool isRight;
public:
  explicit SidebarDragFilter(QObject* parent, bool isRight) : QObject(parent), isRight(isRight) {}
protected:
  bool eventFilter(QObject* obj, QEvent* ev) override {
    QWidget* w = qobject_cast<QWidget*>(obj);
    if (ev->type() == QEvent::MouseButtonPress) {
      QMouseEvent* me = static_cast<QMouseEvent*>(ev);
      // 点击侧边栏时终止主画布的进行中笔画（鼠标画线拖到侧边栏上松开时，release 事件给侧边栏，主画布收不到）
      g.isDrawing   = false;
      collapsedPress = g.collapsed;
      // 展开态：点在按钮上交给按钮；收缩态：整块都允许“拖动/轻点展开”
      if (!collapsedPress && qobject_cast<QPushButton*>(w ? w->childAt(me->pos()) : nullptr)) return false;
      if (me->button() == Qt::LeftButton) {
        dragging = true;
        moved = false;
        dragStartY      = w ? w->y() : 0;
        dragGlobalYStart = me->globalY();
        if (w) w->grabMouse();
        return true;
      }
    } else if (ev->type() == QEvent::MouseMove && dragging) {
      QMouseEvent* me = static_cast<QMouseEvent*>(ev);
      int dy = me->globalY() - dragGlobalYStart;
      if (!moved && qAbs(dy) <= 4) return true;   // 未超过阈值：等待判定（轻点/拖动）
      moved = true;
      int newY = dragStartY + dy;
      int maxY = (g.mainWidget ? g.mainWidget->height() : 1080) - (w ? w->height() : sbHeight());
      newY = qMax(0, qMin(newY, maxY));
      int lockedX = isRight ? (g.mainWidget ? g.mainWidget->width() - sbWidth() - 4 : 2560-68) : 4;
      if (w) w->move(lockedX, newY);

      // 同步两侧边栏
      QWidget* other = isRight ? g.sidebarArea : g.sidebarAreaRight;
      if (other) {
        int otherX = isRight ? 4 : (g.mainWidget ? g.mainWidget->width() - sbWidth() - 4 : 2560-68);
        other->move(otherX, newY);
      }
      g.sidebarScreenPos  = QPoint(4, newY);
      g.sidebarScreenPosR = QPoint((g.mainWidget ? g.mainWidget->width() : 2560) - sbWidth() - 4, newY);
      repositionPopups();
      return true;
    } else if (ev->type() == QEvent::MouseButtonRelease && dragging) {
      dragging = false;
      if (w) w->releaseMouse();
      if (collapsedPress && !moved) { toggleSidebarCollapse(); return true; }  // 收缩态轻点=展开
      // 拖动后侧边栏位置变了，重新设置输入区域，避免点击穿透
      if (g.currentMode == 0) setInputShapeToSidebar();
      return true;
    }
    return QObject::eventFilter(obj, ev);
  }
};


// ============================================================
// 8. 单窗口类（两种形态）
// ============================================================
class MainWidget : public QWidget {
public:
  explicit MainWidget() {
    setAttribute(Qt::WA_TranslucentBackground, true);
    setAttribute(Qt::WA_AcceptTouchEvents, true);  // 接受触摸事件，手指触摸画线不依赖鼠标合成
    setWindowFlags(Qt::Window | Qt::FramelessWindowHint | Qt::WindowStaysOnTopHint
                   | Qt::WindowDoesNotAcceptFocus
                   | Qt::BypassWindowManagerHint);
    setMinimumSize(sbWidth(), sbHeight());
    // 初始全屏透明窗口，双侧边栏可见
    QRect scr = QGuiApplication::primaryScreen()->geometry();
    setGeometry(scr);

    QPalette pal = palette();
    pal.setColor(QPalette::Window, QColor(0,0,0,0));
    setPalette(pal);
    setAutoFillBackground(true);

    // 创建图标
    int isz = 28;
    g.iconCursor = new QPixmap(makeCursorIcon(isz));
    g.iconPen    = new QPixmap(makePenIcon(isz));
    g.iconEraser = new QPixmap(makeEraserIcon(isz));

    // 创建侧边栏（子控件）
    buildSidebar();

    // 画布缓存
    initCanvas();
  }

  QWidget* makeOneSidebar(QWidget* parent, bool isRight) {
    QWidget* sb = new QWidget(parent);
    sb->setFixedSize(sbWidth(), sbHeight());
    sb->setObjectName("sidebarArea");
    sb->setStyleSheet(
      "#sidebarArea { background: transparent; }"
    );
    sb->installEventFilter(new SidebarPainter(sb));

    QVBoxLayout* lay = new QVBoxLayout(sb);
    lay->setContentsMargins(8, 12, 8, 10);
    lay->setSpacing(6);

    // 顶部：收缩/展开工具栏（黄色文字）
    QPushButton* collapseB = new QPushButton(sb);
    collapseB->setFixedSize(sbBtn(), sbBtn());
    collapseB->setStyleSheet(QString(
      "QPushButton{background:#222436;color:#ffd54a;border:2px solid rgba(255,213,74,0.6);"
      "border-radius:%1px;font-weight:bold;font-size:%2px;}"
      "QPushButton:hover{background:#2f3250;}").arg(sbBtn()/2).arg(sbBtn()*13/38));
    QObject::connect(collapseB, &QPushButton::clicked, []() { toggleSidebarCollapse(); });
    lay->addWidget(collapseB);
    if (isRight) g.collapseBtnR = collapseB; else g.collapseBtnL = collapseB;

    QWidget* dragDot = new QWidget(sb);
    dragDot->setFixedSize(sbDot(), sbDot());
    dragDot->move((sb->width() - sbDot()) / 2, -sbDot()/4);
    dragDot->setStyleSheet(
      QString("background-color: rgba(80,85,100,240);"
      "border-radius: %1px;"
      "border: 1.5px solid #888888;").arg(sbDot()/2)
    );
    dragDot->show();

    auto mk = [](const QPixmap& icon) {
      QPushButton* b = new QPushButton();
      b->setFixedSize(sbBtn(), sbBtn());
      b->setIcon(QIcon(icon));
      b->setIconSize(QSize(sbIcon(), sbIcon()));
      b->setStyleSheet(QString("QPushButton{background:transparent;border:2px solid transparent;border-radius:%1px;}"
                       "QPushButton:hover{background:rgba(255,255,255,0.1);}").arg(sbBtn()/2));
      return b;
    };

    QPushButton* cursorB = mk(*g.iconCursor);
    QObject::connect(cursorB, &QPushButton::clicked, [this, isRight]() { g.popupOnRight = isRight; switchToCursorMode(); });
    lay->addWidget(cursorB);

    QPushButton* penB = mk(*g.iconPen);
    QObject::connect(penB, &QPushButton::clicked, [this, isRight]() { g.popupOnRight = isRight; MainWidget::onPenClicked(); });
    lay->addWidget(penB);

    QPushButton* eraserB = mk(*g.iconEraser);
    QObject::connect(eraserB, &QPushButton::clicked, [this, isRight]() { g.popupOnRight = isRight; MainWidget::onEraserClicked(); });
    lay->addWidget(eraserB);

    // 清除键：青色圆角矩形“清除”，清当前页笔迹并归位光标模式（位于橡皮与撤回之间）
    QPushButton* clearB = new QPushButton(QString::fromUtf8("清除"));
    clearB->setFixedSize(sbBtn(), sbBtn());
    clearB->setStyleSheet(QString("QPushButton{background:#00bcd4;color:#06343a;border:2px solid #4dd0e1;border-radius:%1px;font-weight:bold;font-size:%2px;}"
                         "QPushButton:hover{background:#26c6da;}").arg(int(sbBtn()*0.35)).arg(sbBtn()*16/38));
    QObject::connect(clearB, &QPushButton::clicked, []() {
      clearCurrentStrokes();
      switchToCursorMode();
    });
    lay->addWidget(clearB);

    // 撤回键：橙色文字“撤回”
    QPushButton* undoB = new QPushButton(QString::fromUtf8("撤销"));
    undoB->setFixedSize(sbBtn(), sbBtn());
    undoB->setStyleSheet(QString("QPushButton{background:transparent;color:#ff9a3c;border:2px solid rgba(255,154,60,0.7);border-radius:%1px;font-weight:bold;font-size:%2px;}"
                         "QPushButton:hover{background:rgba(255,154,60,0.18);}").arg(int(sbBtn()*0.35)).arg(sbBtn()*16/38));
    QObject::connect(undoB, &QPushButton::clicked, []() { undoLast(); });
    lay->addWidget(undoB);

    sb->installEventFilter(new SidebarDragFilter(sb, isRight));

    if (isRight) {
      g.cursorBtnR = cursorB;
      g.penBtnR = penB;
      g.eraserBtnR = eraserB;
      g.sidebarAreaRight = sb;
    } else {
      g.cursorBtn = cursorB;
      g.penBtn = penB;
      g.eraserBtn = eraserB;
      g.sidebarArea = sb;
    }

    return sb;
  }

  void buildSidebar() {
    // 左侧边栏
    QWidget* leftSb = makeOneSidebar(this, false);
    // 右侧边栏
    QWidget* rightSb = makeOneSidebar(this, true);

    // 定位到两边
    QRect scr = QGuiApplication::primaryScreen()->geometry();
    int iy = (scr.height() - sbHeight()) / 2;
    leftSb->move(4, iy);
    rightSb->move(scr.width() - sbWidth() - 4, iy);

    // nav 按钮——左侧边栏和右侧边栏各一份
    auto mkNav = [](const QString& arrow) {
      QPushButton* b = new QPushButton(arrow);
      b->setFixedSize(sbBtn(), sbBtn());
      b->setStyleSheet(QString("QPushButton{background:#3a3a4a;color:#ccccff;border:1.5px solid #6688cc;border-radius:%1px;font-size:%2px;font-weight:bold;}"
                       "QPushButton:hover{background:#5555aa;color:#ffffff;}").arg(sbBtn()/2).arg(sbBtn()*12/19));
      return b;
    };
    g.prevBtn = mkNav(QString::fromUtf8("\342\226\262"));
    g.nextBtn = mkNav(QString::fromUtf8("\342\226\274"));
    QObject::connect(g.prevBtn, &QPushButton::clicked, []() { goToPrevPage(); });
    QObject::connect(g.nextBtn, &QPushButton::clicked, []() { goToNextPage(); });
    g.prevBtn->setVisible(true);
    g.nextBtn->setVisible(true);
    qobject_cast<QVBoxLayout*>(leftSb->layout())->addWidget(g.prevBtn);
    qobject_cast<QVBoxLayout*>(leftSb->layout())->addWidget(g.nextBtn);

    QPushButton* prevB = mkNav(QString::fromUtf8("\342\226\262"));
    QPushButton* nextB = mkNav(QString::fromUtf8("\342\226\274"));
    QObject::connect(prevB, &QPushButton::clicked, []() { goToPrevPage(); });
    QObject::connect(nextB, &QPushButton::clicked, []() { goToNextPage(); });
    prevB->setVisible(true); nextB->setVisible(true);
    qobject_cast<QVBoxLayout*>(rightSb->layout())->addWidget(prevB);
    qobject_cast<QVBoxLayout*>(rightSb->layout())->addWidget(nextB);

    // 退出放映按钮：正方形、直角矩形、两行“退出/放映”，略大字号；点击先回光标再发 ESC
    auto mkBigExit = []() {
      QPushButton* b = new QPushButton(QString::fromUtf8("退出\n放映"));
      b->setFixedSize(sbBtn(), sbBtn());
      b->setStyleSheet(QString(
        "QPushButton{background:#d33a3a;color:#ffffff;border:2px solid #ff8080;"
        "border-radius:4px;font-weight:bold;font-size:%1px;}"
        "QPushButton:hover{background:#ff4d4d;}").arg(sbBtn()*12/34));
      return b;
    };
    QPushButton* exitL = mkBigExit();
    QPushButton* exitR = mkBigExit();
    g.exitBtnL = exitL;
    g.exitBtnR = exitR;
    QObject::connect(exitL, &QPushButton::clicked, []() { exitPresentation(); });
    QObject::connect(exitR, &QPushButton::clicked, []() { exitPresentation(); });
    qobject_cast<QVBoxLayout*>(leftSb->layout())->addWidget(exitL);
    qobject_cast<QVBoxLayout*>(rightSb->layout())->addWidget(exitR);

    // 白板按钮（圆角矩形文字按钮）——左右各一个
    auto mkWB = [&](bool isRight) {
      QPushButton* b = new QPushButton(QString::fromUtf8("白板"));
      b->setFixedSize(sbBtn(), sbBtn());
      QObject::connect(b, &QPushButton::clicked, []() { toggleWhiteboard(); });
      if (isRight) g.wbBtnR = b; else g.wbBtnL = b;
      return b;
    };
    QPushButton* wbL = mkWB(false);
    QPushButton* wbR = mkWB(true);
    qobject_cast<QVBoxLayout*>(leftSb->layout())->addWidget(wbL);
    qobject_cast<QVBoxLayout*>(rightSb->layout())->addWidget(wbR);

    // 更多功能（省略号）——放最底部
    auto mkMore = []() {
      QPushButton* b = new QPushButton(QString::fromUtf8("\342\213\257"));   // ⋯
      b->setFixedSize(sbBtn(), sbBtn());
      b->setStyleSheet(QString("QPushButton{background:#000000;border:2px solid #333333;border-radius:%1px;font-size:%2px;font-weight:bold;color:#ffffff;}"
                       "QPushButton:hover{background:#222222;}").arg(sbBtn()/2).arg(sbBtn()*16/38));
      QObject::connect(b, &QPushButton::clicked, [b]() { showMoreMenu(b); });
      return b;
    };
    QPushButton* moreL = mkMore();
    QPushButton* moreR = mkMore();
    qobject_cast<QVBoxLayout*>(leftSb->layout())->addWidget(moreL);
    qobject_cast<QVBoxLayout*>(rightSb->layout())->addWidget(moreR);

    // UI：所有按键在侧边栏内水平居中（修复靠左留大空隙）
    auto centerItems = [](QWidget* sb) {
      if (auto* l = qobject_cast<QVBoxLayout*>(sb->layout()))
        for (int i = 0; i < l->count(); ++i)
          if (QLayoutItem* it = l->itemAt(i)) it->setAlignment(Qt::AlignHCenter);
    };
    centerItems(leftSb);
    centerItems(rightSb);

    updateSidebarStyles();
    updateWhiteboardButtonStyles();
    updateCollapseButtons();
    updateExitButtons();
  }

  // ===== 按钮逻辑 =====
  static void onPenClicked() {
    if (g.currentMode == 1) {
      if (g.penPopupVisible) { closeAllPopups(); }
      else { closeAllPopups(); g.penPopupVisible = true; showPenPopup(); }
    } else {
      closeAllPopups();
      switchToDrawMode(1);
    }
  }
  static void onEraserClicked() {
    if (g.currentMode == 2) {
      if (g.eraserPopupVisible) { closeAllPopups(); }
      else { closeAllPopups(); g.eraserPopupVisible = true; showEraserPopup(); }
    } else {
      closeAllPopups();
      switchToDrawMode(2);
    }
  }

  // ===== 绘制 =====
protected:
  void paintEvent(QPaintEvent*) override {
    QPainter p(this);
    p.setCompositionMode(QPainter::CompositionMode_Source);
    p.fillRect(rect(), g.whiteboard ? whiteboardBgColor() : Qt::transparent);

    // 始终画 Pixmap（光标模式下也可见）
    if (g.canvas) {
      p.setCompositionMode(QPainter::CompositionMode_SourceOver);
      p.drawPixmap(0, 0, *g.canvas);
    }
    // 手掌/大触点橡皮：圆形半透明预览（直径=当前擦除宽度）
    if (!g.palmErasePreview.isEmpty()) {
      int ew = (g.currentMode == 2) ? g.eraserWidth() : g.palmEraseW;
      p.setRenderHint(QPainter::Antialiasing, true);
      for (const QPoint& c : g.palmErasePreview) {
        QRectF cr(c.x() - ew / 2.0, c.y() - ew / 2.0, ew, ew);
        p.setBrush(QColor(255, 150, 190, 55));
        p.setPen(QPen(QColor(255, 255, 255, 190), 2));
        p.drawEllipse(cr);
      }
    }
    // 白板模式：左下角页码 + 页数上限提示（蒂芙尼蓝）
    if (g.whiteboard) {
      p.setRenderHint(QPainter::Antialiasing, true);
      QFont f = p.font();
      f.setBold(true);
      f.setPointSize(16);
      p.setFont(f);
      p.setPen(QColor("#0ABAB5"));
      p.drawText(QRect(16, height() - 46, 320, 32),
                 Qt::AlignLeft | Qt::AlignVCenter,
                 QString::fromUtf8("第 %1 页 / %2").arg(g.currentSlide).arg(kCacheBoard));
      if (monotonicMs() < g.wbLimitMsgUntil) {
        QFont mf = p.font();
        mf.setBold(true);
        mf.setPointSize(28);
        p.setFont(mf);
        p.setPen(QColor("#0ABAB5"));
        p.drawText(rect(), Qt::AlignHCenter | Qt::AlignVCenter,
                   QString::fromUtf8("已翻回第 1 页"));
      }
    }
    p.end();
  }

  void mousePressEvent(QMouseEvent* ev) override {
    if (g.currentMode == 0 || !g.canvas) return;
    // 右键（实验）：仅画笔模式下 → 指针处虚拟右键 + 归位光标模式（橡皮模式不处理）
    if (ev->button() == Qt::RightButton) {
      if (g.rightClickCursorOn && g.currentMode == 1) { doVirtualRightClick(ev->globalPos()); switchToCursorMode(); }
      return;
    }
    if (ev->button() == Qt::LeftButton) {
      closeAllPopups();          // 开始绘画/擦除时自动关闭画笔/橡皮子菜单
      pushUndo();                // 撤回：落笔前存快照
      g.isDrawing = true;
      g.lastPt   = ev->pos();
      g.strokeStartMs = monotonicMs();   // 起笔时间（时间→粗细）
      g.lastPenW = timedPenWidth();                            // 起笔最细
      // 落笔即画一个点（起笔最细）
      if (g.currentMode == 2) strokeSegment(g.lastPt, g.lastPt, true, g.eraserWidth());
      else                    strokeTapered(g.lastPt, g.lastPt, g.lastPenW, g.lastPenW);
      int w = g.currentMode == 2 ? g.eraserWidth() : g.lastPenW;
      update(QRect(g.lastPt, g.lastPt).adjusted(-w, -w, w, w));
    }
  }

  void mouseMoveEvent(QMouseEvent* ev) override {
    if (!g.isDrawing || !g.canvas || g.currentMode == 0) return;
    QPoint cur = ev->pos();
    QPoint prev = g.lastPt;
    bool erase = (g.currentMode == 2);
    int w = erase ? g.eraserWidth() : timedPenWidth();
    if (erase) strokeSegment(prev, cur, true, w);
    else       strokeTapered(prev, cur, g.lastPenW, w);   // 沿笔画由细到粗
    g.lastPenW = w;
    g.lastPt = cur;
    // 局部重绘：只刷新本段包围盒，大幅降低 VM 重绘开销
    QRect dirty(QPoint(qMin(prev.x(), cur.x()), qMin(prev.y(), cur.y())),
                QPoint(qMax(prev.x(), cur.x()), qMax(prev.y(), cur.y())));
    update(dirty.adjusted(-w, -w, w, w));
  }

  void mouseReleaseEvent(QMouseEvent* ev) override {
    Q_UNUSED(ev);
    if (g.isDrawing) { g.isDrawing = false; update(); }
  }

  // 手指触摸画线：不经过 grabMouse（XGrabPointer 会让红外触摸的 move 事件丢失）
  // QWidget 没有 touchEvent 虚函数，触摸事件走 event()
  bool event(QEvent* ev) override {
    if (ev->type() == QEvent::TouchBegin || ev->type() == QEvent::TouchUpdate ||
        ev->type() == QEvent::TouchEnd || ev->type() == QEvent::TouchCancel) {
      QTouchEvent* te = static_cast<QTouchEvent*>(ev);
      const QList<QTouchEvent::TouchPoint>& pts = te->touchPoints();
      QPoint cur = pts.isEmpty() ? QPoint() : pts.first().pos().toPoint();

      // 触摸取消：整段手势作废并复位（防幽灵线/卡橡皮）
      if (ev->type() == QEvent::TouchCancel) { resetPalmGesture(); g.isDrawing = false; update(); return true; }

      // 触摸点在侧边栏/弹窗上 → 不画线，返回 false 让 Qt 合成鼠标给正确的子控件（按钮）
      // 同时复位手势状态，防止状态泄漏（残留导致幽灵线 / 卡橡皮 / 幽灵预览）
      auto inWidget = [&](QWidget* w) {
        return w && w->isVisible() && w->geometry().contains(cur);
      };
      if (inWidget(g.sidebarArea) || inWidget(g.sidebarAreaRight) ||
          inWidget(g.penPopup) || inWidget(g.eraserPopup)) {
        resetPalmGesture();
        g.isDrawing = false;
        update();
        return false;
      }

      // 光标模式或画布无效 → 不画线，返回 false（Qt 合成鼠标或穿透桌面）
      if (g.currentMode == 0 || !g.canvas) { resetPalmGesture(); return false; }

      // 多点触控手掌橡皮（正式功能）
      handlePalmTouch(te);
      return true;  // 接受触摸，避免再合成鼠标事件导致重复处理
    }
    return QWidget::event(ev);
  }

  void keyPressEvent(QKeyEvent* ev) override {
    QWidget::keyPressEvent(ev);
  }

  void resizeEvent(QResizeEvent* ev) override {
    QWidget::resizeEvent(ev);
    // 画布尺寸始终跟随窗口（分辨率变化时任意模式都不错位）
    if (g.canvas && g.canvas->size() != size()) {
      QPixmap* old = g.canvas;
      g.canvas = new QPixmap(size());
      g.canvas->fill(Qt::transparent);
      QPainter p(g.canvas); p.drawPixmap(0,0,*old); p.end();
      delete old;
      clearUndo();
    }
    repositionPopups();
  }

  // ---- 手掌/大触点橡皮（正式）：单点=笔；多点或大触点→橡皮（锁存到全部抬手） ----
  void handlePalmTouch(QTouchEvent* te) {
    g.isDrawing = false;
    if (te->type() == QEvent::TouchBegin) closeAllPopups();  // 开始触摸即关闭画笔/橡皮子菜单
    const QList<QTouchEvent::TouchPoint>& pts = te->touchPoints();
    QMap<int, QPoint> now;
    for (const QTouchEvent::TouchPoint& tp : pts)
      if (tp.state() != Qt::TouchPointReleased) now.insert(tp.id(), tp.pos().toPoint());

    const int mode = g.currentMode;
    const bool modeEraseOnly = (mode == 2);
    const int n = now.size();

    // 大触点（手背等）：取最大触点直径
    qreal largeD = 0;
    for (const QTouchEvent::TouchPoint& tp : pts) {
      if (tp.state() == Qt::TouchPointReleased) continue;
      qreal d = qMax(tp.ellipseDiameters().width(), tp.ellipseDiameters().height());
      if (d < 1.0) d = qMax(tp.rect().width(), tp.rect().height());
      if (d > largeD) largeD = d;
    }
    const bool anyLarge = (largeD >= g.largeTouchThreshold);
    // 橡皮触发二选一：默认手背大触点；选“多指”时按 ≥2 指
    const bool largeTrigger = (!g.eraseByFinger) && anyLarge;
    const bool multiTrigger = (g.eraseByFinger) && (n >= 2);
    if (largeTrigger || multiTrigger) g.tPalmLatched = true;
    // 动态橡皮直径：大触点按尺寸×倍率；读不到尺寸/多指则回退固定大号
    if (largeTrigger)
      g.palmEraseW = qBound(kPalmEraseWidth, int(qRound(largeD * g.largeEraseScale10 / 10.0)), 1024);
    else
      g.palmEraseW = kPalmEraseWidth;

    // 笔：未锁存且非橡皮模式。多余触点忽略，仅第一点当笔
    const bool anyPen = (!modeEraseOnly && !g.tPalmLatched && n >= 1);
    const int  penId  = anyPen ? now.firstKey() : -1;

    // 新手势开始
    if (g.tPrevPos.isEmpty() && !now.isEmpty()) {
      pushUndo();                            // 撤回：落笔前存快照
      g.tMoved = true;                       // 视为已产生内容（落笔已画点）
      g.tBeginPos = now.first();
      g.tBeginRole = (modeEraseOnly || g.tPalmLatched) ? 2 : 1;
      g.strokeStartMs = monotonicMs();   // 起笔时间（时间→粗细）
      g.lastPenW = timedPenWidth();                            // 起笔最细
      if (g.tBeginRole == 1) strokeTapered(g.tBeginPos, g.tBeginPos, g.lastPenW, g.lastPenW);
      else strokeSegment(g.tBeginPos, g.tBeginPos, true, (mode == 2 ? g.eraserWidth() : g.palmEraseW));
      if (g.mainWidget) g.mainWidget->update();
    }

    QMap<int,int> curRoleMap;
    for (auto it = now.begin(); it != now.end(); ++it) {
      int id = it.key();
      QPoint pos = it.value();
      int curRole;
      if (modeEraseOnly)                    curRole = 2;   // 橡皮模式：全部触点擦
      else if (g.tPalmLatched)              curRole = 2;   // 大触点/多指锁存：全部擦
      else if (anyPen && id == penId)       curRole = 1;   // 笔（第一点）
      else                                  curRole = 0;   // 忽略（多指且未开多指橡皮）
      curRoleMap[id] = curRole;

      int oldRole = g.tPrevRole.value(id, -1);
      QPoint oldPos = g.tPrevPos.value(id);
      bool haveOld = g.tPrevPos.contains(id);

      if (curRole != oldRole || !haveOld) {   // 新触点或角色变化：记基准，不连笔/连擦
        g.tPrevPos[id] = pos;
        g.tPrevRole[id] = curRole;
        continue;
      }
      if (oldPos == pos || curRole == 0) { g.tPrevPos[id] = pos; continue; }

      int w = (curRole == 1) ? timedPenWidth() : (mode == 2 ? g.eraserWidth() : g.palmEraseW);
      if (curRole == 1) strokeTapered(oldPos, pos, g.lastPenW, w);   // 沿笔画由细到粗
      else              strokeSegment(oldPos, pos, true, w);
      g.lastPenW = w;
      g.tMoved = true;
      update(QRect(QPoint(qMin(oldPos.x(),pos.x()), qMin(oldPos.y(),pos.y())),
                   QPoint(qMax(oldPos.x(),pos.x()), qMax(oldPos.y(),pos.y()))).adjusted(-w,-w,w,w));
      g.tPrevPos[id] = pos;
      g.tPrevRole[id] = curRole;
    }

    // 清理抬起的触点
    for (auto it = g.tPrevPos.begin(); it != g.tPrevPos.end();) {
      if (!now.contains(it.key())) it = g.tPrevPos.erase(it); else ++it;
    }
    for (auto it = g.tPrevRole.begin(); it != g.tPrevRole.end();) {
      if (!now.contains(it.key())) it = g.tPrevRole.erase(it); else ++it;
    }

    // 圆形橡皮预览（直径=当前擦除宽度）
    QVector<QPoint> prev = g.tPrevPreview;
    g.palmErasePreview.clear();
    for (auto it = now.begin(); it != now.end(); ++it)
      if (curRoleMap.value(it.key(), 2) == 2) g.palmErasePreview << it.value();
    QVector<QPoint> all = prev; for (const QPoint& q : g.palmErasePreview) all << q;
    if (!all.isEmpty()) {
      int pad = (mode == 2 ? g.eraserWidth() : g.palmEraseW) / 2 + 4;
      QRect u(all.first(), all.first());
      for (const QPoint& q : all) u = u.united(QRect(q, q));
      update(u.adjusted(-pad, -pad, pad, pad));
    }
    g.tPrevPreview = g.palmErasePreview;

    if (now.isEmpty()) {
      if (!g.tMoved && !g.tPalmLatched && mode != 0) {   // 轻点补点
        if (g.tBeginRole == 1) strokeSegment(g.tBeginPos, g.tBeginPos, false, timedPenWidth());
        else strokeSegment(g.tBeginPos, g.tBeginPos, true, (mode == 2 ? g.eraserWidth() : g.palmEraseW));
      }
      resetPalmGesture();
      update();
    }
  }
};

