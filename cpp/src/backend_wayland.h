#pragma once
// ============================================================
// Sidera - Wayland 后端（裸 libwayland-client + Qt5 软件绘图）
// 覆盖层用 zwlr_layer_shell_v1，渲染用 wl_shm + QImage/QPainter。
// 不使用 Qt Widgets / Qt Wayland 插件，因此可在 Qt 5.12（Ubuntu 20.04）编译。
// 侧边栏/弹窗用 QPainter 重画，外观对齐 X11 版（左右两侧）。
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
// ============================================================
#include <QObject>
#include <QImage>
#include <QPixmap>
#include <QPoint>
#include <QRect>
#include <QColor>
#include <QVector>
#include <QMap>
#include "page_cache.h"
#include "clock.h"

#include <wayland-client-protocol.h>
#include "wlr-layer-shell-unstable-v1-client-protocol.h"
#include "fractional-scale-v1-client-protocol.h"
#include "viewporter-client-protocol.h"
#include "virtual-keyboard-unstable-v1-client-protocol.h"
#include "wlr-screencopy-unstable-v1-client-protocol.h"

#include <xkbcommon/xkbcommon.h>

class QSocketNotifier;
class QPaintDevice;
class WpsBridge;

// 共享绘图/图标函数（定义于 drawing.cpp，与 X11 路径共用）
QPixmap makeCursorIcon(int s);
QPixmap makePenIcon(int s);
QPixmap makeEraserIcon(int s);
void drawPenSegment(QPaintDevice* dev, const QColor& color, QPoint a, QPoint b, int width);
void drawEraseSegment(QPaintDevice* dev, QPoint a, QPoint b, int width);
void drawPenTapered(QPaintDevice* dev, const QColor& color, QPoint a, QPoint b, int wStart, int wEnd);

// 侧边栏按钮顺序（与 X11 buildSidebar 一致）
enum WlBtn {
  BtnCollapse = 0, BtnCursor, BtnPen, BtnEraser, BtnClear, BtnUndo,
  BtnPrev, BtnNext, BtnExit, BtnWhiteboard, BtnMore, BtnCount
};

class WlBackend : public QObject {
public:
  explicit WlBackend(QObject* parent = nullptr);
  ~WlBackend() override;

  bool init();

  // 以下成员/方法设为 public，供 file-scope 的 C 回调函数直接访问。
  // ---- Wayland 全局对象 ----
  wl_display*    display_      = nullptr;
  wl_registry*   registry_     = nullptr;
  wl_compositor* compositor_   = nullptr;
  wl_shm*        shm_          = nullptr;
  wl_seat*       seat_         = nullptr;
  wl_output*     output_       = nullptr;
  zwlr_layer_shell_v1* layerShell_ = nullptr;
  wp_fractional_scale_manager_v1* fsManager_ = nullptr;
  wp_viewporter* viewporter_   = nullptr;
  zwp_virtual_keyboard_manager_v1* vkManager_ = nullptr;
  zwlr_screencopy_manager_v1* screencopy_ = nullptr;

  wl_surface*            surface_      = nullptr;
  zwlr_layer_surface_v1* layerSurface_ = nullptr;
  wl_pointer*            pointer_      = nullptr;
  wl_touch*              touch_        = nullptr;
  wp_fractional_scale_v1* fractionalScale_ = nullptr;
  wp_viewport*           viewport_     = nullptr;
  zwp_virtual_keyboard_v1* vk_         = nullptr;
  xkb_context*           xkbCtx_       = nullptr;
  xkb_keymap*            xkbMap_       = nullptr;
  double                 fracScale_    = 1.0;

  // ---- shm 双缓冲 ----
  struct Buf { wl_buffer* buffer = nullptr; size_t size = 0; bool busy = false; };
  wl_shm_pool* pool_    = nullptr;
  void*        shmData_ = nullptr;
  size_t       shmSize_ = 0;
  Buf          bufs_[2];
  struct RetiredPool { wl_shm_pool* pool; void* data; size_t size; wl_buffer* buffer; };
  QVector<RetiredPool> retiredPools_;
  int          curBuf_  = 0;
  int          width_   = 0;
  int          height_  = 0;
  int          logicalW_ = 0;
  int          logicalH_ = 0;
  int          scale_   = 1;
  int          bufferW_ = 0, bufferH_ = 0;
  bool         pendingRender_ = false;
  bool         initialized_ = false;
  uint32_t     seatName_ = 0, outputName_ = 0;

  // ---- 状态 ----
  QPixmap ink_;
  int   mode_      = 0;         // 0=光标 1=画笔 2=橡皮
  int   curColor_  = 0;
  int   curPenSize_ = 1;
  int   curEraserSize_ = 1;
  bool  collapsed_ = false;
  bool  drawing_   = false;
  QPoint lastPt_;
  QPoint curPos_;
  int   touchId_   = -1;
  qint64 strokeStartMs_ = 0;
  int   lastPenW_  = 0;
  QVector<QPixmap> undoStack_;

  // 多页缓存 + 白板
  PageCache pageCache_;
  PageCache wbCache_;
  int whiteboardPage_ = 1;
  int   currentPage_ = 1;
  int   savedNormalPage_ = 1;
  bool  whiteboard_  = false;
  bool  cursorAfterWhiteboard_ = false;
  int   wbBgIndex_   = 1;       // 0=墨绿 1=白

  // 截图（zwlr_screencopy 异步）
  zwlr_screencopy_frame_v1* shotFrame_ = nullptr;
  int    shotFmt_ = 0, shotW_ = 0, shotH_ = 0, shotStride_ = 0;
  int    shotFd_ = -1;
  void*  shotData_ = nullptr;
  size_t shotSize_ = 0;
  wl_buffer* shotBuffer_ = nullptr;
  bool shotInverted_ = false;

  int   popupType_ = 0;         // 0=无 1=画笔弹窗 2=橡皮弹窗
  bool  popupOnRight_ = false;  // 弹窗由哪一侧触发
  int   menu_ = 0;              // 0=无 1=更多菜单

  // 设置面板
  bool   settingsOpen_ = false;
  bool   quitConfirm_  = false;
  int    sidebarAlpha_ = 80;    // 30~255
  double sbScale_      = 1.0;   // 0.6~1.4
  int    draggingSlider_ = 0;   // 0=无 1=透明度 2=大小
  QRect  settingsRect_, alphaTrack_, sizeTrack_, autostartRect_, quitRect_, closeRect_;
  QVector<QRect> buttons_;      // 左侧按钮
  QVector<QRect> buttonsR_;     // 右侧按钮
  QVector<QRect> colorRects_;
  QVector<QRect> penSizeRects_;
  QVector<QRect> eraserSizeRects_;
  QVector<QRect> moreRects_;    // 更多菜单项（截图/设置）
  QRect eraserClearRect_;
  QRect sidebarRect_;
  QRect sidebarRectR_;
  QRect popupRect_;
  QRect menuRect_;

  QSocketNotifier* notifier_ = nullptr;
  WpsBridge* wps_ = nullptr;    // WPS 联动桥（HTTP 16666）

  // 交互反馈
  int   hoverBtn_   = -1;       // 悬停的侧边栏按钮
  bool  hoverRight_ = false;
  qreal animT_      = 1.0;      // 弹窗淡入/缩放动画进度 0→1
  QTimer* animTimer_ = nullptr;

  // ---- 初始化 ----
  void createSurface();
  void createPool(int size);
  void createBuffers();
  void maybeCreateBuffers();
  void destroyBuffers();
  void cancelInput();
  void setupVirtualKeyboard();
  void sendKeysym(xkb_keysym_t sym);

  // ---- 渲染 ----
  void render(QImage& img);
  void drawSidebar(QPainter& p, bool right);
  void drawPopups(QPainter& p);
  void drawMoreMenu(QPainter& p);
  void drawIcon(QPainter& p, const QRect& r, int which);
  void drawText(QPainter& p, const QRect& r, const QString& t, const QColor& c, int px, bool bold);
  void drawBtnStyle(QPainter& p, const QRect& r, const QColor& fill, const QColor& border, double bw, double radius);
  void renderAndCommit();
  void startAnim();

  // ---- 输入区域 ----
  void setInputRegionSidebar();
  void setInputRegionFull();

  // ---- 输入处理 ----
  void onPointerMove(const QPoint& pos);
  void onPointerButton(uint32_t button, bool pressed);
  void onTouchDown(int id, const QPoint& pos);
  void onTouchMove(int id, const QPoint& pos);
  void onTouchUp(int id);

  // ---- 布局 / 命中 ----
  void layoutUi();
  int  sbW() const;
  int  sbBtn() const;
  int  sbIcon() const;
  int  sbDot() const;
  int  buttonAt(const QPoint& p, bool* right) const;
  int  colorAt(const QPoint& p) const;
  int  penSizeAt(const QPoint& p) const;
  int  eraserSizeAt(const QPoint& p) const;
  int  moreAt(const QPoint& p) const;
  bool inSidebar(const QPoint& p) const;
  bool inPopup(const QPoint& p) const;
  bool inMenu(const QPoint& p) const;

  // ---- 动作 ----
  void setMode(int m);
  void togglePopup(int type);
  void closePopup();
  void clickButton(int idx, bool right);
  void toggleCollapse();
  void pushUndo();
  void undo();
  void clearInk();
  void savePage();
  void loadPage(int page);
  void goNextPage();
  void goPrevPage();
  void exitPresentation();
  void toggleWhiteboard();
  void doScreenshot();
  void wpsOnRealPos(int pos);
  void wpsOnBegin(int pos);
  int  timedWidth();
  int  eraserWidth();

  // ---- 设置面板 ----
  void openSettings();
  void closeSettings();
  void drawSettings(QPainter& p);
  void drawSlider(QPainter& p, const QRect& track, double frac);
  int  settingsSliderAt(const QPoint& p) const;
  void updateSlider(int which, const QPoint& p);
  bool isAutoStart() const;
  void setAutoStart(bool on);
  void loadSettings();
  void saveSettings();
};
