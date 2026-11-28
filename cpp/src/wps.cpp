// Sidera - WPS 联动/缓存/HTTP 服务
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
#include "app.h"

// 当前生效的笔迹缓存：白板与放映独立，页数不作为丢弃条件。
PageCache& activeCache() { return g.whiteboard ? g.whiteboardCache : g.slideCache; }

void clearAllPages() {
  g.slideCache.clear();
  g.savedSlide = 1;
  if (g.whiteboard) return;
  g.currentSlide = 1;
  g.pageHasInk = false;
  g.isDrawing = false;
  g.lastPt = QPoint();
  if (g.canvas) g.canvas->fill(Qt::transparent);
  clearUndo();
  if (g.mainWidget) g.mainWidget->update();
}

void saveCurrentPage() {
  if (!g.canvas) return;
  if (!g.pageHasInk) { activeCache().remove(g.currentSlide); return; }
  activeCache().insert(g.currentSlide, *g.canvas);
}

void loadPage(int page) {
  if (!g.canvas) return;
  g.canvas->fill(Qt::transparent);
  const QPixmap cached = activeCache().value(page);
  if (!cached.isNull()) {
    QPainter painter(g.canvas);
    painter.drawPixmap(g.canvas->rect(), cached, cached.rect());
  }
  g.pageHasInk = !cached.isNull();
  clearUndo();
  if (g.mainWidget) g.mainWidget->update();
}

// ============================================================
// WPS 接口调试模式（实验）：本地 HTTP 16666 + 日志
// 核心思路：触发“下一步”的机制无所谓（假键/官方键都是同一件事），
//   真正的区别在缓存时机。
//   默认模式（调试关）：每点一次按钮 = 存一页并刷新（近似行为，多动画会错位）；
//   调试模式（开 + 加载项已连）：按钮只发 Up/Down 推进放映，不做缓存；
//     缓存改由加载项回传的真实页号事件驱动——真实换页才存旧页/载新页，
//     页内动画步不动缓存 → 批注一直跟着真页走，动画不错位。
//   没客户端连接时自动退回默认行为（点击即缓存），保证按钮始终可用。
// ============================================================
QString wpsLogFile() {
  return QDir::homePath() + "/wps-api-debug.log";
}

void wpsLog(const QString& msg) {
  QString line = QDateTime::currentDateTime().toString("yyyy-MM-dd HH:mm:ss.zzz ") + msg;
  qDebug().noquote() << "[WPSAPI]" << msg;
  QString path = wpsLogFile();
  if (QFileInfo(path).isSymLink()) return;
  // 超过 1MB 自动重开，防日志无限增长
  if (QFileInfo(path).size() > 1024 * 1024) QFile::remove(path);
  QFile f(path);
  if (f.open(QIODevice::Append | QIODevice::Text)) {
    QTextStream ts(&f);
    ts << line << "\n";
    f.close();
  }
}

// ---------------- 设置持久化（wpsDebug / 侧边栏透明度 / 大小） ----------------
static QString wpsConfigFile() {
  return QStandardPaths::writableLocation(QStandardPaths::ConfigLocation) + "/sidera/config";
}
static bool wpsConfigExists() { return QFile::exists(wpsConfigFile()); }
void wpsSaveSettings() {
  QDir().mkpath(QFileInfo(wpsConfigFile()).absolutePath());
  if (QFileInfo(wpsConfigFile()).isSymLink()) { qWarning() << "Refusing symlink settings"; return; }
  QSaveFile f(wpsConfigFile());
  if (!f.open(QIODevice::WriteOnly | QIODevice::Text)) return;
  QTextStream ts(&f);
  ts << "wpsDebug=" << (g.wpsDebug ? 1 : 0) << "\n";
  ts << "sbScale=" << QString::number(g.sbScale, 'f', 2) << "\n";
  ts << "sidebarAlpha=" << g.sidebarAlpha << "\n";
  ts << "rightClickCursor=" << (g.rightClickCursorOn ? 1 : 0) << "\n";
  ts << "eraseByFinger=" << (g.eraseByFinger ? 1 : 0) << "\n";
  ts << "largeTouchThreshold=" << g.largeTouchThreshold << "\n";
  ts << "largeEraseScale10=" << g.largeEraseScale10 << "\n";
  ts.flush();
  if (!f.commit()) qWarning() << "Cannot save settings:" << f.errorString();
}
void wpsLoadSettings() {
  QFile f(wpsConfigFile());
  if (!f.open(QIODevice::ReadOnly | QIODevice::Text)) return;
  while (!f.atEnd()) {
    QString line = QString::fromUtf8(f.readLine()).trimmed();
    int eq = line.indexOf('=');
    if (eq < 0) continue;
    QString k = line.left(eq);
    QString v = line.mid(eq + 1);
    if (k == "wpsDebug") g.wpsDebug = (v == "1");
    else if (k == "sbScale") { bool ok = false; double scale = v.toDouble(&ok); if (ok && qIsFinite(scale)) g.sbScale = qBound(0.6, scale, 1.4); }
    else if (k == "sidebarAlpha") g.sidebarAlpha = qBound(30, v.toInt(), 255);
    else if (k == "rightClickCursor") g.rightClickCursorOn = (v == "1");
    else if (k == "eraseByFinger") g.eraseByFinger = (v == "1");
    else if (k == "largeTouchThreshold") g.largeTouchThreshold = qBound(20, v.toInt(), 260);
    else if (k == "largeEraseScale10") g.largeEraseScale10 = qBound(8, v.toInt(), 25);
  }
  f.close();
}

static void finishSlideshow() {
  clearAllPages();
  g.wpsRealPos = -1;
  if (g.whiteboard) g.cursorAfterWhiteboard = true;
  else if (g.currentMode != 0) switchToCursorMode();
}

// Shared bridge owns registration, HTTP bounds, command lifetime and heartbeat.
static void startWpsApiServer() {
  if (g.wpsBridge) return;
  auto* bridge = new WpsBridge(qApp);
  bridge->onConnectionChanged = [](bool online) { g.wpsConnected = online; };
  bridge->onSlideshowBegin = [](int page) {
    clearAllPages(); g.wpsRealPos = page;
    if (g.whiteboard) g.savedSlide = page;
    else { g.currentSlide = page; loadPage(page); }
  };
  bridge->onSlideshowEnd = finishSlideshow;
  bridge->onRealPos = [](int page) {
    if (g.whiteboard) { g.savedSlide = page; g.wpsRealPos = page; return; }
    if (page == g.currentSlide && page == g.wpsRealPos) return;
    saveCurrentPage(); g.wpsRealPos = page; g.currentSlide = page; loadPage(page);
  };
  bridge->setWhiteboard(g.whiteboard);
  if (!bridge->start()) { delete bridge; return; }
  g.wpsBridge = bridge;
}
void stopWpsApiServer() {
  delete g.wpsBridge; g.wpsBridge = nullptr;
  g.wpsConnected = false;
}
void setWpsDebug(bool enabled, bool persist) {
  g.wpsDebug = enabled;
  if (enabled) startWpsApiServer();
  else { stopWpsApiServer(); g.wpsRealPos = -1; }
  if (persist) wpsSaveSettings();
}

void goToPrevPage() {
  // 白板模式：完全本地翻页，绝不发送任何虚拟按键，与外界隔离
  if (g.whiteboard) {
    saveCurrentPage();
    if (g.currentSlide > 1) g.currentSlide--;
    loadPage(g.currentSlide);
    return;
  }
  if (g.wpsDebug && g.wpsConnected && g.wpsRealPos > 0 && g.wpsBridge) {
    wpsLog("调试：入队 PREV 指令，等待加载项轮询执行");
    g.wpsBridge->enqueue("PREV");
    return;
  }
  saveCurrentPage();
  if (g.currentSlide > 1) g.currentSlide--;
  // 始终发送 Up 键（与假键语义一致：有动画时它就是“下一步”）
  {
    Display* dpy = g.xDisplay;
    bool nc = false; if (!dpy) { dpy = XOpenDisplay(nullptr); nc = true; }
    if (dpy) { sendXTestKey(dpy, XK_Up); if (nc) XCloseDisplay(dpy); }
  }
  loadPage(g.currentSlide);
}

void goToNextPage() {
  // 白板模式：完全本地翻页，绝不发送任何虚拟按键，与外界隔离
  if (g.whiteboard) {
    saveCurrentPage();
    int p = g.currentSlide + 1;
    if (p > kCacheBoard) {
      // 翻过第 10 页 → 回到第 1 页，并提示 3 秒（不清除第 1 页内容）
      p = 1;
      g.wbLimitMsgUntil = monotonicMs() + 3000;
      QTimer::singleShot(3000, []() { if (g.mainWidget) g.mainWidget->update(); });
      qDebug() << "[INFO] 白板页数已达上限，回到第 1 页";
    }
    g.currentSlide = p;
    loadPage(g.currentSlide);
    return;
  }
  // 调试模式且已连加载项：只入队 NEXT 指令，由加载项执行；不再发假键（避免双重翻页）
  if (g.wpsDebug && g.wpsConnected && g.wpsRealPos > 0 && g.wpsBridge) {
    wpsLog("调试：入队 NEXT 指令，等待加载项轮询执行");
    g.wpsBridge->enqueue("NEXT");
    return;
  }
  saveCurrentPage();
  g.currentSlide++;
  // 始终发送 Down 键（与假键语义一致：有动画时它就是“下一步”）
  {
    Display* dpy = g.xDisplay;
    bool nc = false; if (!dpy) { dpy = XOpenDisplay(nullptr); nc = true; }
    if (dpy) { sendXTestKey(dpy, XK_Down); if (nc) XCloseDisplay(dpy); }
  }
  loadPage(g.currentSlide);
}

/*
 * 判断窗口是否属于 WPS / OnlyOffice / LibreOffice
 * （用 WM_CLASS 的 res_name / res_class 匹配，小写 contains）
 */
static bool isOfficeWindow(Display* dpy, Window w) {
  XClassHint cls;
  if (!XGetClassHint(dpy, w, &cls)) return false;
  QString name  = QString::fromLocal8Bit(cls.res_name).toLower();
  QString klass = QString::fromLocal8Bit(cls.res_class).toLower();
  XFree(cls.res_name); XFree(cls.res_class);

  // WPS 各组件 + OnlyOffice + LibreOffice
  const char* keys[] = {
    "wps", "wpp", "et", "wpspdf",          // WPS Office
    "onlyoffice", "desktopeditors",        // OnlyOffice
    "soffice", "impress", "libreoffice",   // LibreOffice
    nullptr
  };
  for (int i = 0; keys[i]; i++)
    if (QString(keys[i]).size() <= 3 ? (name == keys[i] || klass == keys[i]) :
        (name.contains(keys[i]) || klass.contains(keys[i]))) return true;
  return false;
}

/*
 * 全屏放映检测（组合方案 + 递归遍历）：
 *   1. 递归遍历所有顶层/嵌套窗口（WPS 放映窗口是嵌套窗口，XQueryTree(root) 只查直接子节点会漏掉）
 *   2. 只检查 WPS/OnlyOffice/LibreOffice 窗口（WM_CLASS 白名单）——彻底隔离 ClassIsland 等悬浮窗
 *   3. 对这些窗口：_NET_WM_STATE_FULLSCREEN 标志 或 几何尺寸铺满屏幕，任一命中即放映
 *
 * 这样无论 WPS 在 kwin 下是设 FULLSCREEN 还是只铺满屏幕、窗口是否嵌套都能检测到。
 */
static bool isPresentationFullscreen(Display* dpy) {
  // 静态缓存 Atom，避免每 500ms 重复 Intern
  static Atom netWmState = 0, netWmFullscreen = 0;
  if (!netWmState) {
    netWmState      = XInternAtom(dpy, "_NET_WM_STATE", False);
    netWmFullscreen = XInternAtom(dpy, "_NET_WM_STATE_FULLSCREEN", False);
  }

  // 自己的窗口 XID
  Window selfWin = 0;
  if (g.mainWidget) {
    QWindow* wh = g.mainWidget->windowHandle();
    if (wh) selfWin = (Window)wh->winId();
    if (!selfWin) selfWin = (Window)g.mainWidget->winId();
  }

  QRect scr = QGuiApplication::primaryScreen()->geometry();
  bool found = false;

  std::function<void(Window)> walk = [&](Window w) {
    if (found) return;
    if (selfWin && w == selfWin) return;  // 排除自己

    // 对当前窗口：若是办公软件且全屏 → 判定放映
    if (isOfficeWindow(dpy, w)) {
      XWindowAttributes attrs;
      if (XGetWindowAttributes(dpy, w, &attrs) && attrs.map_state == IsViewable) {
        // FULLSCREEN 标志
        bool fullscreen = false;
        Atom at; int af; unsigned long ni, ba; unsigned char* pr = nullptr;
        if (XGetWindowProperty(dpy, w, netWmState, 0, 1024, False, XA_ATOM,
                               &at, &af, &ni, &ba, &pr) == Success && pr) {
          Atom* a = (Atom*)pr;
          for (unsigned long j = 0; j < ni; j++) if (a[j] == netWmFullscreen) { fullscreen = true; break; }
          XFree(pr);
        }
        // 几何尺寸兜底（kwin 下 WPS 可能不设 FULLSCREEN，只铺满屏幕）
        if (!fullscreen) {
          if (attrs.width >= scr.width() - 8 && attrs.height >= scr.height() - 8) {
            fullscreen = true;
          }
        }
        if (fullscreen) { found = true; return; }
      }
    }

    // 递归子窗口（放映窗口可能嵌套多层，必须深入遍历）
    Window r, p, *ch; unsigned int n;
    if (XQueryTree(dpy, w, &r, &p, &ch, &n) && ch) {
      for (unsigned int i = 0; i < n; i++) walk(ch[i]);
      XFree(ch);
    }
  };

  walk(DefaultRootWindow(dpy));
  return found;
}

/*
 * 定时检查 WPS 全屏状态，控制翻页按钮可见性。
 */
void checkWpsState() {
  Display* dpy = g.xDisplay;
  bool needClose = false;
  if (!dpy) { dpy = XOpenDisplay(nullptr); needClose = true; }
  if (!dpy) return;

  bool was = g.wpsFullscreen;
  g.wpsFullscreen = isPresentationFullscreen(dpy);

  if (needClose) XCloseDisplay(dpy);

  if (was && !g.wpsFullscreen && !g.wpsConnected) {
    finishSlideshow();
  } else if (!was && g.wpsFullscreen && !g.whiteboard && !g.wpsDebug) {
    clearAllPages();
  }

  // 翻页按钮始终显示
  if (g.prevBtn) g.prevBtn->setVisible(!g.collapsed);
  if (g.nextBtn) g.nextBtn->setVisible(!g.collapsed);

  // 光标模式下定期刷新输入区域，确保侧边栏可点击稳定（XShape 可能因时序/位置变化偶发失效）
  if (g.currentMode == 0) setInputShapeToSidebar();
}

