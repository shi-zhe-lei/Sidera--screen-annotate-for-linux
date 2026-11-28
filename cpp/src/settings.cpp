// Sidera - 设置面板/系统诊断
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
#include "app.h"

static QString autoStartPath() {
  QString dir = QStandardPaths::writableLocation(QStandardPaths::ConfigLocation) + "/autostart";
  QDir().mkpath(dir);
  return dir + "/sidera.desktop";
}
static bool isAutoStart() {
  return QFile::exists(autoStartPath());
}
static void setAutoStart(bool on) {
  QString path = autoStartPath();
  if (on) {
    QFile f(path);
    if (f.open(QIODevice::WriteOnly | QIODevice::Text)) {
      QTextStream ts(&f);
      ts << "[Desktop Entry]\n"
         << "Type=Application\n"
         << "Name=Sidera\n"
         << "Comment=屏幕批注与白板\n"
         << "Exec=sidera\n"
         << "Icon=sidera\n"
         << "Terminal=false\n";
      f.close();
    }
  } else {
    QFile::remove(path);
  }
}

// 恢复被设置窗口隐藏的主画布并重建输入区域
// 设置窗口可能以多种方式关闭：点标题栏 X、点“完成”。统一走这里收尾，
// 否则主画布会一直隐藏、应用看起来“消失”。
static void restoreCanvasAfterSettings() {
  if (!g.mainWidget) return;
  g.mainWidget->show();
  if (g.currentMode == 0) setInputShapeToSidebar();
  else resetInputShape();
}

// 设置窗口
// 滑块行：滑块 + 数值标签
static QHBoxLayout* makeSliderRow(QSlider* slider, QLabel* val) {
  QHBoxLayout* row = new QHBoxLayout();
  row->addWidget(slider, 1);
  val->setFixedWidth(36);
  val->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
  val->setStyleSheet("color:#aaddff;font-weight:bold;");
  row->addWidget(val);
  return row;
}

void openSettings() {
  if (g.settingsWin) { g.settingsWin->raise(); g.settingsWin->activateWindow(); return; }
  if (g.mainWidget) g.mainWidget->hide();  // 隐藏画布，让普通设置窗口不被 override-redirect 盖住

  QWidget* win = new QWidget();
  g.settingsWin = win;
  // 关闭即销毁：标题栏 X 也会触发 destroyed，从而执行恢复画布的回调
  win->setAttribute(Qt::WA_DeleteOnClose, true);
  win->setWindowFlags(Qt::Window | Qt::WindowStaysOnTopHint);
  win->setWindowTitle(QString::fromUtf8("Sidera 设置"));
  win->setFixedWidth(300);
  win->setStyleSheet(
    "QWidget{background:#2b2b33;color:#eee;font-size:14px;}"
    "QLabel{color:#ccc;}"
    "QPushButton{background:#444;color:#fff;border:1px solid #666;border-radius:6px;padding:8px;}"
    "QPushButton:hover{background:#555;}"
    "QSlider::groove:horizontal{height:6px;background:#444;border-radius:3px;}"
    "QSlider::sub-page:horizontal{background:#3377cc;border-radius:3px;}"
    "QSlider::handle:horizontal{width:16px;margin:-5px 0;background:#fff;border-radius:8px;}"
  );

  QVBoxLayout* lay = new QVBoxLayout(win);
  lay->setContentsMargins(16, 16, 16, 12);
  lay->setSpacing(10);

  // 顶部居中：Sidera 图标
  {
    QLabel* iconLbl = new QLabel();
    iconLbl->setPixmap(sideraIconPixmap(56));
    iconLbl->setAlignment(Qt::AlignCenter);
    lay->addWidget(iconLbl);
  }

  // 透明度
  lay->addWidget(new QLabel(QString::fromUtf8("工具栏不透明度")));
  QSlider* alphaSlider = new QSlider(Qt::Horizontal);
  alphaSlider->setRange(30, 255);
  alphaSlider->setValue(g.sidebarAlpha);
  QLabel* alphaVal = new QLabel(QString::number(g.sidebarAlpha));
  QObject::connect(alphaSlider, &QSlider::valueChanged, [alphaVal](int v) {
    g.sidebarAlpha = v;
    alphaVal->setText(QString::number(v));
    // 触发两侧边栏重绘（背景由 SidebarPainter 用 g.sidebarAlpha 画）
    if (g.sidebarArea) g.sidebarArea->update();
    if (g.sidebarAreaRight) g.sidebarAreaRight->update();
    wpsSaveSettings();
  });
  lay->addLayout(makeSliderRow(alphaSlider, alphaVal));

  // 大小
  lay->addWidget(new QLabel(QString::fromUtf8("工具栏大小")));
  QSlider* sizeSlider = new QSlider(Qt::Horizontal);
  sizeSlider->setRange(60, 140);
  sizeSlider->setValue(int(g.sbScale * 100));
  QLabel* sizeVal = new QLabel(QString::number(g.sbScale, 'f', 1));
  QObject::connect(sizeSlider, &QSlider::valueChanged, [sizeVal](int v) {
    g.sbScale = v / 100.0;
    sizeVal->setText(QString::number(g.sbScale, 'f', 1));
    rebuildSidebars();
    wpsSaveSettings();
  });
  lay->addLayout(makeSliderRow(sizeSlider, sizeVal));

  lay->addSpacing(4);

  // 开机自启动切换按键
  QPushButton* autoBtn = new QPushButton();
  auto updateAutoBtn = [autoBtn]() {
    bool on = isAutoStart();
    autoBtn->setText(on ? QString::fromUtf8("登录时自动启动：开") : QString::fromUtf8("登录时自动启动：关"));
    autoBtn->setStyleSheet(on ? "QPushButton{background:#2a6e3f;color:#fff;border:1px solid #3a8f55;border-radius:6px;padding:8px;}"
                              : "QPushButton{background:#444;color:#fff;border:1px solid #666;border-radius:6px;padding:8px;}");
  };
  updateAutoBtn();
  QObject::connect(autoBtn, &QPushButton::clicked, [autoBtn, updateAutoBtn]() {
    setAutoStart(!isAutoStart());
    updateAutoBtn();
  });
  lay->addWidget(autoBtn);

  lay->addSpacing(6);

  // 系统诊断按钮（用于定位旧驱动下的侧边栏/XShape 问题）
  QPushButton* infoBtn = new QPushButton(QString::fromUtf8("系统诊断"));
  infoBtn->setStyleSheet("QPushButton{background:#444;color:#ccc;padding:8px;border-radius:6px;}"
                         "QPushButton:hover{background:#555;}");
  QObject::connect(infoBtn, &QPushButton::clicked, []() { showSystemInfo(); });
  lay->addWidget(infoBtn);

  lay->addSpacing(6);

  // WPS 接口调试模式（实验）：翻页走本地 TCP 16666 + 加载项回传真实页号
  QPushButton* wpsBtn = new QPushButton();
  QLabel* wpsHint = new QLabel();
  wpsHint->setWordWrap(true);
  wpsHint->setStyleSheet("color:#667;font-size:11px;");
  auto updateWpsBtn = [wpsBtn, wpsHint]() {
    bool on = g.wpsDebug;
    wpsBtn->setText(on ? QString::fromUtf8("同步 WPS 页码：开") : QString::fromUtf8("同步 WPS 页码：关"));
    wpsBtn->setStyleSheet(on ? "QPushButton{background:#6a3f2a;color:#fff;border:1px solid #aa7a4a;border-radius:6px;padding:8px;}"
                             : "QPushButton{background:#444;color:#fff;border:1px solid #666;border-radius:6px;padding:8px;}");
    wpsHint->setText(on ? QString::fromUtf8("按幻灯片页码保存批注，页内动画共用同一页。")
                        : QString::fromUtf8("每次翻页操作都切换批注页。"));
  };
  updateWpsBtn();
  QObject::connect(wpsBtn, &QPushButton::clicked, [wpsBtn, updateWpsBtn]() {
    setWpsDebug(!g.wpsDebug);
    updateWpsBtn();
  });
  lay->addWidget(wpsBtn);
  lay->addWidget(wpsHint);

  // 右键 → 虚拟右键 + 归位光标模式（实验，默认开）
  QPushButton* rcBtn = new QPushButton();
  auto updateRcBtn = [rcBtn]() {
    bool on = g.rightClickCursorOn;
    rcBtn->setText(on ? QString::fromUtf8("右键切回光标（试验）：开") : QString::fromUtf8("右键切回光标（试验）：关"));
    rcBtn->setStyleSheet(on ? "QPushButton{background:#2a5a6a;color:#fff;border:1px solid #4dd0e1;border-radius:6px;padding:8px;}"
                            : "QPushButton{background:#444;color:#fff;border:1px solid #666;border-radius:6px;padding:8px;}");
  };
  updateRcBtn();
  QObject::connect(rcBtn, &QPushButton::clicked, [updateRcBtn]() {
    g.rightClickCursorOn = !g.rightClickCursorOn;
    wpsSaveSettings();
    updateRcBtn();
  });
  lay->addWidget(rcBtn);
  QLabel* rcHint = new QLabel(QString::fromUtf8("批注时右击也会打开演示窗口的右键菜单。"));
  rcHint->setWordWrap(true);
  rcHint->setStyleSheet("color:#667;font-size:11px;");
  lay->addWidget(rcHint);

  // 橡皮触发方式：手背大触点 / 多指（二选一，默认手背）
  QPushButton* trigBtn = new QPushButton();

  // 大触点判定阈值（px）—— 仅“手背大触点”触发时显示
  QLabel* thrLabel = new QLabel(QString::fromUtf8("大触点阈值 (px)"));
  QSlider* thrSlider = new QSlider(Qt::Horizontal);
  thrSlider->setRange(20, 260);
  thrSlider->setValue(g.largeTouchThreshold);
  QLabel* thrVal = new QLabel(QString::number(g.largeTouchThreshold));
  QObject::connect(thrSlider, &QSlider::valueChanged, [thrVal](int v) {
    g.largeTouchThreshold = v;
    thrVal->setText(QString::number(v));
    wpsSaveSettings();
  });

  // 大触点橡皮倍率（×0.1）—— 仅“手背大触点”触发时显示
  QLabel* scLabel = new QLabel(QString::fromUtf8("大触点橡皮倍率"));
  QSlider* scSlider = new QSlider(Qt::Horizontal);
  scSlider->setRange(8, 25);      // 0.8 ~ 2.5
  scSlider->setValue(g.largeEraseScale10);
  QLabel* scVal = new QLabel(QString::number(g.largeEraseScale10 / 10.0, 'f', 1));
  QObject::connect(scSlider, &QSlider::valueChanged, [scVal](int v) {
    g.largeEraseScale10 = v;
    scVal->setText(QString::number(v / 10.0, 'f', 1));
    wpsSaveSettings();
  });

  auto updateTrigBtn = [trigBtn, thrLabel, thrSlider, thrVal, scLabel, scSlider, scVal]() {
    trigBtn->setText(g.eraseByFinger ? QString::fromUtf8("橡皮触发方式: 多指")
                                     : QString::fromUtf8("橡皮触发方式: 手背"));
    trigBtn->setStyleSheet("QPushButton{background:#2a5a6a;color:#fff;border:1px solid #4dd0e1;border-radius:6px;padding:8px;}"
                           "QPushButton:hover{background:#356b7d;}");
    // 仅“手背大触点”时显示阈值/倍率调节；隐藏不改变其值
    const bool show = !g.eraseByFinger;
    thrLabel->setVisible(show); thrSlider->setVisible(show); thrVal->setVisible(show);
    scLabel->setVisible(show);  scSlider->setVisible(show);  scVal->setVisible(show);
  };
  updateTrigBtn();
  QObject::connect(trigBtn, &QPushButton::clicked, [updateTrigBtn]() {
    g.eraseByFinger = !g.eraseByFinger;
    wpsSaveSettings();
    updateTrigBtn();
  });
  lay->addWidget(trigBtn);
  QLabel* trigHint = new QLabel(QString::fromUtf8("手背模式按触点大小识别，多指模式用两指或更多手指擦除。"));
  trigHint->setWordWrap(true);
  trigHint->setStyleSheet("color:#667;font-size:11px;");
  lay->addWidget(trigHint);

  lay->addWidget(thrLabel);
  lay->addLayout(makeSliderRow(thrSlider, thrVal));
  lay->addWidget(scLabel);
  lay->addLayout(makeSliderRow(scSlider, scVal));

  // 清空调试日志（防止不熟悉的人让日志越积越多）
  QPushButton* clearLogBtn = new QPushButton(QString::fromUtf8("清空调试日志"));
  clearLogBtn->setStyleSheet("QPushButton{background:#444;color:#ccc;padding:8px;border-radius:6px;font-size:13px;}"
                             "QPushButton:hover{background:#555;}");
  QObject::connect(clearLogBtn, &QPushButton::clicked, [clearLogBtn]() {
    QFile::remove(wpsLogFile());
    clearLogBtn->setText(QString::fromUtf8("已清空 ✓"));
    QTimer::singleShot(1200, [clearLogBtn]() { clearLogBtn->setText(QString::fromUtf8("清空调试日志")); });
  });
  lay->addWidget(clearLogBtn);

  // 版权信息
  QLabel* creditLbl = new QLabel(QString::fromUtf8("Sidera 3.2-Electro-stable   © 2026 Carl_Jin\nGNU GPL v3"));
  creditLbl->setAlignment(Qt::AlignCenter);
  creditLbl->setStyleSheet("color:#556;font-size:11px;");
  lay->addWidget(creditLbl);

  lay->addSpacing(6);

  QPushButton* doneBtn = new QPushButton(QString::fromUtf8("完成"));
  doneBtn->setStyleSheet("QPushButton{background:#3377cc;color:#fff;font-weight:bold;padding:10px;border-radius:6px;}"
                         "QPushButton:hover{background:#4488dd;}");
  QObject::connect(doneBtn, &QPushButton::clicked, []() { closeSettings(); });
  lay->addWidget(doneBtn);

  // 退出软件（独立按键 + 确认弹窗；窗口右上角 X 只关闭设置、不会退出）
  QPushButton* quitBtn = new QPushButton(QString::fromUtf8("退出 Sidera"));
  quitBtn->setStyleSheet("QPushButton{background:#8a2f2f;color:#fff;padding:10px;border-radius:6px;}"
                         "QPushButton:hover{background:#a83a3a;}");
  QObject::connect(quitBtn, &QPushButton::clicked, [win]() {
    QMessageBox box(QMessageBox::Question, QString::fromUtf8("退出 Sidera？"),
                    QString::fromUtf8("退出会清除当前批注。需要保留的内容请先截图。"),
                    QMessageBox::Yes | QMessageBox::No, win);
    box.setDefaultButton(QMessageBox::No);
    if (box.exec() == QMessageBox::Yes) QApplication::quit();
  });
  lay->addWidget(quitBtn);

  // 窗口关闭（点 X / 完成）恢复画布并复位指针（不退出软件）
  QObject::connect(win, &QWidget::destroyed, []() {
    g.settingsWin = nullptr;
    restoreCanvasAfterSettings();
  });

  // 屏幕居中
  win->adjustSize();
  win->move(QGuiApplication::primaryScreen()->geometry().center() - win->rect().center());
  win->show();
}

void closeSettings() {
  if (g.settingsWin) g.settingsWin->close();
  g.settingsWin = nullptr;
  restoreCanvasAfterSettings();
}

// ============================================================
// 系统诊断信息（用于定位旧驱动下 XShape/侧边栏问题）
// ============================================================
static QString collectSystemInfo() {
  QString s;
  QFile osf("/etc/os-release");
  if (osf.open(QIODevice::ReadOnly)) {
    while (!osf.atEnd()) {
      QString line = QString::fromLocal8Bit(osf.readLine()).trimmed();
      if (line.startsWith("PRETTY_NAME=")) s += "系统: " + line.mid(13) + "\n";
    }
    osf.close();
  }
  s += "架构: " + QString(QSysInfo::currentCpuArchitecture()) + "\n";
  s += "内核: " + QString(QSysInfo::kernelType()) + " " + QString(QSysInfo::kernelVersion()) + "\n";
  s += "Qt: " + QString(qVersion()) + "\n";
  s += "XDG_SESSION_TYPE: " + QString::fromLocal8Bit(qgetenv("XDG_SESSION_TYPE")) + "\n";
  s += "DISPLAY: " + QString::fromLocal8Bit(qgetenv("DISPLAY")) + "\n";
  s += "合成器: " + QString(hasCompositor() ? "有" : "无") + "\n";

  // X11 / XShape 扩展检查
  Display* dpy = g.xDisplay;
  bool nc = false; if (!dpy) { dpy = XOpenDisplay(nullptr); nc = true; }
  if (dpy) {
    int evBase, errBase, major, minor;
    if (XShapeQueryExtension(dpy, &evBase, &errBase) && XShapeQueryVersion(dpy, &major, &minor)) {
      s += "XShape: 扩展可用 v" + QString::number(major) + "." + QString::number(minor) + "\n";
    } else {
      s += "XShape: 扩展不可用!\n";
    }
    if (nc) XCloseDisplay(dpy);
  }

  QScreen* sc = QGuiApplication::primaryScreen();
  if (sc) s += "屏幕: " + QString::number(sc->geometry().width()) + "x" + QString::number(sc->geometry().height()) + "\n";

  if (g.mainWidget) {
    s += "模式: " + QString(g.currentMode == 0 ? "光标" : (g.currentMode == 1 ? "画笔" : "橡皮")) + "\n";
    s += "画布: 可见=" + QString(g.mainWidget->isVisible() ? "是" : "否")
       + " pos=(" + QString::number(g.mainWidget->pos().x()) + "," + QString::number(g.mainWidget->pos().y()) + ")"
       + " size=(" + QString::number(g.mainWidget->width()) + "x" + QString::number(g.mainWidget->height()) + ")"
       + " override_redirect 窗口\n";
  }
  auto sbInfo = [&](const char* tag, QWidget* w) {
    if (w)
      s += QString(tag) + ": pos=(" + QString::number(w->pos().x()) + "," + QString::number(w->pos().y()) + ")"
         + " size=(" + QString::number(w->width()) + "x" + QString::number(w->height()) + ")"
         + " 可见=" + QString(w->isVisible() ? "是" : "否") + "\n";
    else
      s += QString(tag) + ": (空/未创建)\n";
  };
  sbInfo("左侧边栏", g.sidebarArea);
  sbInfo("右侧边栏", g.sidebarAreaRight);
  return s;
}

void showSystemInfo() {
  QDialog* dlg = new QDialog(g.settingsWin);
  dlg->setWindowTitle(QString::fromUtf8("系统诊断"));
  dlg->resize(500, 360);
  QVBoxLayout* lay = new QVBoxLayout(dlg);
  QTextEdit* te = new QTextEdit(dlg);
  te->setReadOnly(true);
  te->setPlainText(collectSystemInfo());
  lay->addWidget(te);
  QPushButton* close = new QPushButton(QString::fromUtf8("关闭"), dlg);
  QObject::connect(close, &QPushButton::clicked, dlg, &QDialog::close);
  lay->addWidget(close);
  dlg->exec();
  delete dlg;
}

