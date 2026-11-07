#pragma once
// ============================================================
// Sidera - 公共声明（AppState / 常量 / 全局 / 共享函数）
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
// ============================================================
#include <QApplication>
#include <QWidget>
#include <QVBoxLayout>
#include <QHBoxLayout>
#include <QPushButton>
#include <QLabel>
#include <QPixmap>
#include <QPainter>
#include <QPen>
#include <QBrush>
#include <QIcon>
#include <QPainterPath>
#include <QMouseEvent>
#include <QKeyEvent>
#include <QPaintEvent>
#include <QTouchEvent>
#include <QMoveEvent>
#include <QResizeEvent>
#include <QScreen>
#include <QSocketNotifier>
#include <QDebug>
#include <QColor>
#include <QPalette>
#include <QFrame>
#include <QTimer>
#include <QMap>
#include <QSlider>
#include <QFile>
#include <QTextStream>
#include <QStandardPaths>
#include <QDir>
#include <QDialog>
#include <QTextEdit>
#include <QSysInfo>
#include <QWindow>
#include <QDateTime>
#include <QTcpServer>
#include <QTcpSocket>
#include <QHostAddress>
#include <QAbstractSocket>
#include <QRegExp>
#include <QQueue>
#include <QUrl>
#include <QUrlQuery>
#include <QFileInfo>
#include <QCoreApplication>
#include <QLocalServer>
#include <QLocalSocket>
#include <QProgressBar>
#include <QElapsedTimer>
#include <QThread>
#include <QMessageBox>
#include <QMenu>
#include <QClipboard>
#include "page_cache.h"
#include "clock.h"
#include "wps_bridge.h"

#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/keysym.h>
#include <X11/XKBlib.h>

#include <X11/Xatom.h>
#include <X11/extensions/shape.h>
#include <X11/extensions/XTest.h>

#include <cstdlib>
#include <functional>
#include <QSet>

// ---------------- 全局状态与常量 ----------------
struct AppState {
  QWidget* mainWidget = nullptr;   // 单窗口
  QPixmap* canvas     = nullptr;   // 绘图缓存

  // 左侧边栏 + 按钮（原名不变）
  QWidget* sidebarArea     = nullptr;
  QPushButton* cursorBtn = nullptr;
  QPushButton* penBtn    = nullptr;
  QPushButton* eraserBtn = nullptr;

  // 右侧镜像
  QWidget* sidebarAreaRight = nullptr;
  QPushButton *cursorBtnR = nullptr, *penBtnR = nullptr, *eraserBtnR = nullptr;

  QWidget* penPopup      = nullptr;
  QWidget* eraserPopup   = nullptr;
  bool popupOnRight      = false;   // 弹窗从哪个侧边栏触发的

  // 模式
  int  currentMode = 0;           // 0=光标 1=画笔 2=橡皮擦
  bool penPopupVisible   = false;
  bool eraserPopupVisible = false;

  // 白板模式：透明可穿透画布 ↔ 不透明白板（全屏可输入）
  bool whiteboard = false;
  QPushButton* wbBtnL = nullptr;
  QPushButton* wbBtnR = nullptr;
  // 白板背景色索引（会话内记忆，重启回默认；0=墨绿 1=白，默认白）
  int whiteboardBgIndex = 1;
  QPushButton* exitBtnL = nullptr;   // 退出放映键（白板模式下变为“背景颜色”键）
  QPushButton* exitBtnR = nullptr;

  // 侧边栏收缩
  bool collapsed = false;
  QPushButton* collapseBtnL = nullptr;
  QPushButton* collapseBtnR = nullptr;
  QTimer* sidebarIdleTimer = nullptr;   // 3 分钟无操作自动收缩

  // 颜色 + 粗细
  QVector<QColor> colors     = { QColor(255,40,40), QColor(50,120,255), QColor(40,200,60), QColor(255,210,30), QColor(240,240,240),
                                 QColor("#0ABAB5"), QColor("#AB47BC"), QColor("#C9DD22"),
                                 QColor("#FF6D00"), QColor("#000000") };
  QVector<int> penSizes      = { 3, 5, 10 };
  QVector<int> eraserSizes   = { 12, 24, 48 };
  int curColor   = 0;
  int curPen     = 1;
  int curEraser  = 1;      // 默认中等橡皮

  QColor penColor()      const { return colors[curColor]; }
  int    penWidth()      const { return penSizes[curPen]; }
  int    eraserWidth()   const { return eraserSizes[curEraser]; }

  // 绘图状态
  bool   isDrawing = false;
  QPoint lastPt;

  // 侧边栏屏幕坐标（手动跟踪，避免 mapToGlobal 的累积误差）
  QPoint sidebarScreenPos;
  QPoint sidebarScreenPosR;

  // 图标
  QPixmap* iconCursor = nullptr;
  QPixmap* iconPen    = nullptr;
  QPixmap* iconEraser = nullptr;

  // 侧边栏缩放系数（0.6 ~ 1.4，1.0 为默认）
  double sbScale = 1.0;
  // 侧边栏背景透明度（30 ~ 255，255=不透明）
  int sidebarAlpha = 80;
  // 设置窗口指针
  QWidget* settingsWin = nullptr;

  // 平台
  QString platform  = "unknown";
  bool    hotkeyOk  = false;
  Display* xDisplay = nullptr;
  Window   xRootWin = 0;

  // WPS 联动
  bool wpsFullscreen = false;
  QPushButton* prevBtn = nullptr;
  QPushButton* nextBtn = nullptr;

  // 多页缓存
  PageCache slideCache;
  PageCache whiteboardCache;
  int whiteboardSlide = 1;
  int currentSlide  = 1;           // 当前页码（普通/放映；白板模式下即白板页码）
  int savedSlide    = 1;           // 进入白板前记住的普通模式页码
  bool pageHasInk   = false;       // 当前页是否有过笔迹（无笔迹页不入缓存）
  qint64 wbLimitMsgUntil = 0;      // 白板页数上限提示的截止时间(ms)

  // WPS 接口调试模式（教室默认开）：翻页走本地 HTTP 16666，WPS 加载项回传真实页号/事件
  bool         wpsDebug         = true;
  bool         wpsConnected     = false;
  WpsBridge*   wpsBridge        = nullptr;
  int          wpsRealPos       = -1;   // 加载项上报的真实页号（1 起）

  // 手掌/大触点橡皮（正式功能）：多点或大触点→橡皮；擦除直径按大触点尺寸缩放
  QVector<QPoint> palmErasePreview;   // 当前作为“橡皮”的触点位置（画圆形预览用）
  int palmEraseW = 64;                // 当前大触点橡皮直径（动态；回退固定 64）

  // 右键 → 虚拟右键 + 归位光标模式（试验，默认开）
  bool rightClickCursorOn = true;

  // 橡皮触发方式：false=手背大触点（默认），true=多指
  bool eraseByFinger = false;
  // 大触点判定阈值（触点直径 px，可调，默认 64）
  int largeTouchThreshold = 64;
  // 大触点橡皮尺寸倍率 ×10（默认 12 = 1.2 倍，可调）
  int largeEraseScale10 = 12;

  // 笔画起笔时间（时间→粗细）
  qint64 strokeStartMs = 0;
  int    lastPenW = 0;                // 当前笔画上一段的宽度（锥形过渡用）

  // 手掌/大触点手势引擎状态
  QMap<int, QPoint> tPrevPos;
  QMap<int, int>    tPrevRole;        // 1=笔, 2=橡皮
  QVector<QPoint>   tPrevPreview;
  bool   tPalmLatched = false;        // 本次触摸出现多点/大触点后，直到全部抬手当橡皮
  bool   tMoved = false;              // 本次手势是否产生过笔迹/擦除
  QPoint tBeginPos;
  int    tBeginRole = 0;

  // 撤回栈（每笔落笔前存快照）
  QList<QPixmap*> undoStack;
};

// 大触点/多点触发时的回退橡皮直径（比可选最大橡皮 48 更大）
static const int kPalmEraseWidth = 64;
// 笔画“时间→粗细”参数：起笔为当前宽度的 kStrokeMinScale，随按住时间增粗到当前宽度
static const int    kStrokeRampMs   = 600;
static const double kStrokeMinScale = 0.85;
// 撤回栈最大步数
static const int kMaxUndo = 12;
// 多页缓存容量：WPS 放映 / 白板 / 其它
static const int kCacheWps   = 20;
static const int kCacheBoard = 10;
static const int kCacheOther = 2;

extern AppState g;

// 白板背景色：墨绿 / 白（默认索引见 AppState.whiteboardBgIndex）
static const QColor kWhiteboardColors[] = { QColor("#0F3D2E"), QColor(255, 255, 255) };
static const int    kWhiteboardColorCount = 2;
inline QColor whiteboardBgColor() {
  int i = qBound(0, g.whiteboardBgIndex, kWhiteboardColorCount - 1);
  return kWhiteboardColors[i];
}

// 当前生效的笔迹缓存：白板模式与普通/放映模式各自独立
PageCache& activeCache();

// ---------------- 图标 / 绘制 / 撤回 ----------------
QPixmap makeCursorIcon(int s);
QPixmap makePenIcon(int s);
QPixmap makeEraserIcon(int s);
QString sideraIconPath();
QPixmap sideraIconPixmap(int px);
void drawPenSegment(QPaintDevice* dev, const QColor& color, QPoint a, QPoint b, int width);
void drawEraseSegment(QPaintDevice* dev, QPoint a, QPoint b, int width);
void drawPenTapered(QPaintDevice* dev, const QColor& color, QPoint a, QPoint b, int wStart, int wEnd);
void strokeSegment(QPoint a, QPoint b, bool erase, int width);
void strokeToCanvas(QPoint a, QPoint b);
void strokeTapered(QPoint a, QPoint b, int wStart, int wEnd);   // 宽度沿笔画渐变的画笔笔迹
int  timedPenWidth();          // 按落笔时长计算的当前笔迹宽度
void clearUndo();
void pushUndo();
void undoLast();
void resetPalmGesture();

// ---------------- 侧边栏 ----------------
int  sbWidth();
int  sbBtn();
int  sbIcon();
int  sbDot();
int  sbHeight();
void updateSidebarStyles();
void updateWhiteboardButtonStyles();
void updateExitButtons();       // 退出键文字/样式（白板模式下变为背景颜色键）
void toggleWhiteboard();
void updateCollapseButtons();
void collapseSidebars();
void expandSidebars();
void toggleSidebarCollapse();
void rebuildSidebars();

// ---------------- 弹窗 / 子菜单 ----------------
void showPenPopup();
void showEraserPopup();
void closeAllPopups();
void repositionPopups();
void showMoreMenu(QPushButton* b);
void clearCurrentStrokes();

// ---------------- 模式 / 画布 / 输入区域 / X11 ----------------
void switchToCursorMode();
void switchToDrawMode(int mode);
void initCanvas();
void clearCanvas();
void setInputShapeToSidebar();
void resetInputShape();
void exitPresentation();
void sendXTestKey(Display* dpy, KeySym ks);
void doVirtualRightClick(const QPoint& globalPos);
bool hasCompositor();

// ---------------- 多页缓存 / WPS 联动 ----------------
void clearAllPages();
void saveCurrentPage();
void loadPage(int page);
void goToPrevPage();
void goToNextPage();
QString wpsLogFile();
void wpsLog(const QString& msg);
void wpsSaveSettings();
void wpsLoadSettings();
void setWpsDebug(bool on, bool persist = true);
void stopWpsApiServer();
void checkWpsState();

// ---------------- 设置 / 诊断 / 闪屏 ----------------
void openSettings();
void closeSettings();
void showSystemInfo();
void showSplashFor(QApplication& app);
