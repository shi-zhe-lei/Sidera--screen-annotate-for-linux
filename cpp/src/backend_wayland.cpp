// ============================================================
// Sidera - Wayland 后端实现（niri / hyprland / kwin 通用 wlr-layer-shell）
// 侧边栏/弹窗用 QPainter 重画，外观对齐 X11 版；左右两侧。
// 翻页/退出经 zwp_virtual_keyboard 注入按键；白板为本地功能。
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
// ============================================================
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "backend_wayland.h"
#include "wps_bridge.h"

#include <QPainter>
#include <QFont>
#include <QDateTime>
#include <QDebug>
#include <QSocketNotifier>
#include <QTimer>
#include <QStandardPaths>
#include <QDir>
#include <QClipboard>
#include <QGuiApplication>
#include <QCoreApplication>
#include <QFile>
#include <QFileInfo>
#include <QTextStream>

#include <xkbcommon/xkbcommon.h>
#include <xkbcommon/xkbcommon-keysyms.h>

#include <cstring>
#include <cstdlib>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <limits>
#include <cmath>

// ---- 调色板 / 尺寸（与 X11 路径一致） ----
static const QVector<QColor> kColors = {
  QColor(255,40,40), QColor(50,120,255), QColor(40,200,60), QColor(255,210,30), QColor(240,240,240),
  QColor("#0ABAB5"), QColor("#AB47BC"), QColor("#C9DD22"), QColor("#FF6D00"), QColor("#000000")
};
static const QVector<int> kPenSizes    = { 3, 5, 10 };
static const QVector<int> kEraserSizes = { 12, 24, 48 };
static const QColor kWhiteboardColors[] = { QColor("#0F3D2E"), QColor(255,255,255) };
static const int kMaxUndo = 12;
static const int kCacheBoard = 10;

static const int kSbBaseW = 56, kSbBaseBtn = 34, kSbBaseIcon = 24, kSbBaseDot = 19;

// ============================================================
// 静态回调
// ============================================================
static void seatCapabilities(void*, wl_seat*, uint32_t);
static void seatName(void*, wl_seat*, const char*) {}
static const wl_seat_listener seatListener = { seatCapabilities, seatName };
static void registryGlobal(void* data, wl_registry* r, uint32_t name, const char* interface, uint32_t version) {
  WlBackend* self = static_cast<WlBackend*>(data);
  if (!std::strcmp(interface, wl_compositor_interface.name))
    self->compositor_ = (wl_compositor*)wl_registry_bind(r, name, &wl_compositor_interface, qMin(version, 4u));
  else if (!std::strcmp(interface, wl_shm_interface.name))
    self->shm_ = (wl_shm*)wl_registry_bind(r, name, &wl_shm_interface, 1);
  else if (!std::strcmp(interface, wl_seat_interface.name) && !self->seat_) {
    self->seatName_ = name;
    self->seat_ = (wl_seat*)wl_registry_bind(r, name, &wl_seat_interface, qMin(version, 7u));
    wl_seat_add_listener(self->seat_, &seatListener, self);
  }
  else if (!std::strcmp(interface, wl_output_interface.name) && !self->output_) {
    self->outputName_ = name;
    self->output_ = (wl_output*)wl_registry_bind(r, name, &wl_output_interface, qMin(version, 2u));
  }
  else if (!std::strcmp(interface, zwlr_layer_shell_v1_interface.name))
    self->layerShell_ = (zwlr_layer_shell_v1*)wl_registry_bind(r, name, &zwlr_layer_shell_v1_interface, qMin(version, 4u));
  else if (!std::strcmp(interface, wp_fractional_scale_manager_v1_interface.name))
    self->fsManager_ = (wp_fractional_scale_manager_v1*)wl_registry_bind(r, name, &wp_fractional_scale_manager_v1_interface, 1);
  else if (!std::strcmp(interface, wp_viewporter_interface.name))
    self->viewporter_ = (wp_viewporter*)wl_registry_bind(r, name, &wp_viewporter_interface, 1);
  else if (!std::strcmp(interface, zwp_virtual_keyboard_manager_v1_interface.name))
    self->vkManager_ = (zwp_virtual_keyboard_manager_v1*)wl_registry_bind(r, name, &zwp_virtual_keyboard_manager_v1_interface, 1);
  else if (!std::strcmp(interface, zwlr_screencopy_manager_v1_interface.name))
    self->screencopy_ = (zwlr_screencopy_manager_v1*)wl_registry_bind(r, name, &zwlr_screencopy_manager_v1_interface, qMin(version, 3u));
}

static void pointerMotion(void* d, wl_pointer*, uint32_t, wl_fixed_t sx, wl_fixed_t sy) {
  if (static_cast<WlBackend*>(d)->touchId_ >= 0) return;
  static_cast<WlBackend*>(d)->onPointerMove(QPoint(wl_fixed_to_int(sx), wl_fixed_to_int(sy)));
}
static void pointerButton(void* d, wl_pointer*, uint32_t, uint32_t, uint32_t button, uint32_t state) {
  if (static_cast<WlBackend*>(d)->touchId_ >= 0) return;
  static_cast<WlBackend*>(d)->onPointerButton(button, state == WL_POINTER_BUTTON_STATE_PRESSED);
}
static void touchDown(void* d, wl_touch*, uint32_t, uint32_t, wl_surface*, int32_t id, wl_fixed_t x, wl_fixed_t y) {
  static_cast<WlBackend*>(d)->onTouchDown(id, QPoint(wl_fixed_to_int(x), wl_fixed_to_int(y)));
}
static void touchUp(void* d, wl_touch*, uint32_t, uint32_t, int32_t id) {
  static_cast<WlBackend*>(d)->onTouchUp(id);
}
static void touchMotion(void* d, wl_touch*, uint32_t, int32_t id, wl_fixed_t x, wl_fixed_t y) {
  static_cast<WlBackend*>(d)->onTouchMove(id, QPoint(wl_fixed_to_int(x), wl_fixed_to_int(y)));
}
static void outputScale(void* d, wl_output*, int32_t factor) {
  WlBackend* s = static_cast<WlBackend*>(d);
  if (factor > 0 && factor <= 4) s->scale_ = factor;
  if (s->initialized_) s->renderAndCommit();
}
static void fractionalPreferred(void* d, wp_fractional_scale_v1*, uint32_t scale) {
  WlBackend* s = static_cast<WlBackend*>(d);
  if (scale >= 120 && scale <= 480) s->fracScale_ = scale / 120.0;
  if (s->initialized_) s->renderAndCommit();
}
static void layerConfigure(void* d, zwlr_layer_surface_v1* ls, uint32_t serial, uint32_t w, uint32_t h) {
  WlBackend* s = static_cast<WlBackend*>(d);
  zwlr_layer_surface_v1_ack_configure(ls, serial);
  // Reject overflow before any allocation or QImage construction.
  if (!w || !h || w > 16384 || h > 16384 || uint64_t(w) * h > 67108864) return;
  s->logicalW_ = int(w); s->logicalH_ = int(h);
  if (s->initialized_) {
    s->cancelInput();
    s->layoutUi();
    s->renderAndCommit();
  }
}
static void bufferRelease(void* d, wl_buffer* buffer) {
  WlBackend* s = static_cast<WlBackend*>(d);
  for (int i = 0; i < 2; i++) if (s->bufs_[i].buffer == buffer) { s->bufs_[i].busy = false; break; }
  for (int i = 0; i < s->retiredPools_.size(); ++i) {
    if (s->retiredPools_[i].buffer != buffer) continue;
    const auto retired = s->retiredPools_.takeAt(i);
    wl_buffer_destroy(retired.buffer);
    wl_shm_pool_destroy(retired.pool);
    munmap(retired.data, retired.size);
    break;
  }
  if (s->pendingRender_) s->renderAndCommit();
}

// ---- no-op stubs ----
static void noopOutputGeometry(void*, wl_output*, int32_t,int32_t,int32_t,int32_t,int32_t,const char*,const char*,int32_t) {}
static void noopOutputMode(void*, wl_output*, uint32_t,int32_t,int32_t,int32_t) {}
static void noopOutputDone(void*, wl_output*) {}
[[maybe_unused]] static void noopOutputName(void*, wl_output*, const char*) {}
[[maybe_unused]] static void noopOutputDesc(void*, wl_output*, const char*) {}
static void noopPointerEnter(void*, wl_pointer*, uint32_t, wl_surface*, wl_fixed_t, wl_fixed_t) {}
static void noopPointerLeave(void*, wl_pointer*, uint32_t, wl_surface*) {}
static void noopPointerAxis(void*, wl_pointer*, uint32_t, uint32_t, wl_fixed_t) {}
static void noopPointerFrame(void*, wl_pointer*) {}
static void noopPointerAxisSource(void*, wl_pointer*, uint32_t) {}
static void noopPointerAxisStop(void*, wl_pointer*, uint32_t, uint32_t) {}
static void noopPointerAxisDiscrete(void*, wl_pointer*, uint32_t, int32_t) {}
[[maybe_unused]] static void noopPointerAxisValue120(void*, wl_pointer*, uint32_t, int32_t) {}
[[maybe_unused]] static void noopPointerAxisRelDir(void*, wl_pointer*, uint32_t, uint32_t) {}
[[maybe_unused]] static void noopPointerWarp(void*, wl_pointer*, wl_fixed_t, wl_fixed_t) {}
static void noopTouchFrame(void*, wl_touch*) {}
static void noopTouchCancel(void* d, wl_touch*) { static_cast<WlBackend*>(d)->cancelInput(); }
static void noopTouchShape(void*, wl_touch*, int32_t, wl_fixed_t, wl_fixed_t) {}
static void noopTouchOrientation(void*, wl_touch*, int32_t, wl_fixed_t) {}
static void noopRegistryRemove(void* d, wl_registry*, uint32_t name) {
  auto* s = static_cast<WlBackend*>(d);
  // The legacy backend owns one output/seat. Exit safely instead of retaining
  // dead input objects; the Rust backend provides live multi-output recovery.
  if (name == s->seatName_ || name == s->outputName_) {
    s->cancelInput();
    QCoreApplication::quit();
  }
}
static void noopLayerClosed(void*, zwlr_layer_surface_v1*) { QCoreApplication::quit(); }

// ---- screencopy 回调 ----
static void shotBuffer(void* d, zwlr_screencopy_frame_v1* frame, uint32_t format, uint32_t w, uint32_t h, uint32_t stride);
static void shotReady(void* d, zwlr_screencopy_frame_v1* frame, uint32_t, uint32_t, uint32_t);
static void shotFailed(void* d, zwlr_screencopy_frame_v1* frame);
static void noopShotFlags(void* d, zwlr_screencopy_frame_v1*, uint32_t flags) {
  static_cast<WlBackend*>(d)->shotInverted_ = flags & ZWLR_SCREENCOPY_FRAME_V1_FLAGS_Y_INVERT;
}
static void noopShotDamage(void*, zwlr_screencopy_frame_v1*, uint32_t, uint32_t, uint32_t, uint32_t) {}
static void noopShotDmabuf(void*, zwlr_screencopy_frame_v1*, uint32_t, uint32_t, uint32_t) {}
static void noopShotBufferDone(void*, zwlr_screencopy_frame_v1*) {}
static const zwlr_screencopy_frame_v1_listener shotListener = {
  shotBuffer, noopShotFlags, shotReady, shotFailed, noopShotDamage, noopShotDmabuf, noopShotBufferDone
};

static const wl_registry_listener registryListener = { registryGlobal, noopRegistryRemove };
static const wl_pointer_listener pointerListener = {
  noopPointerEnter, noopPointerLeave, pointerMotion, pointerButton, noopPointerAxis,
  noopPointerFrame, noopPointerAxisSource, noopPointerAxisStop, noopPointerAxisDiscrete,
#ifdef WL_POINTER_AXIS_VALUE120_SINCE_VERSION
  noopPointerAxisValue120,
#endif
#ifdef WL_POINTER_AXIS_RELATIVE_DIRECTION_SINCE_VERSION
  noopPointerAxisRelDir,
#endif
#ifdef WL_POINTER_WARP_SINCE_VERSION
  noopPointerWarp,
#endif
};
static const wl_touch_listener touchListener = {
  touchDown, touchUp, touchMotion, noopTouchFrame, noopTouchCancel, noopTouchShape, noopTouchOrientation
};
static void seatCapabilities(void* data, wl_seat* seat, uint32_t caps) {
  auto* s = static_cast<WlBackend*>(data);
  if ((caps & WL_SEAT_CAPABILITY_POINTER) && !s->pointer_) {
    s->pointer_ = wl_seat_get_pointer(seat);
    wl_pointer_add_listener(s->pointer_, &pointerListener, s);
  } else if (!(caps & WL_SEAT_CAPABILITY_POINTER) && s->pointer_) {
    s->cancelInput(); wl_pointer_destroy(s->pointer_); s->pointer_ = nullptr;
  }
  if ((caps & WL_SEAT_CAPABILITY_TOUCH) && !s->touch_) {
    s->touch_ = wl_seat_get_touch(seat);
    wl_touch_add_listener(s->touch_, &touchListener, s);
  } else if (!(caps & WL_SEAT_CAPABILITY_TOUCH) && s->touch_) {
    s->cancelInput(); wl_touch_destroy(s->touch_); s->touch_ = nullptr;
  }
}
static const wl_output_listener outputListener = {
  noopOutputGeometry, noopOutputMode, noopOutputDone, outputScale,
#ifdef WL_OUTPUT_NAME_SINCE_VERSION
  noopOutputName,
#endif
#ifdef WL_OUTPUT_DESCRIPTION_SINCE_VERSION
  noopOutputDesc,
#endif
};
static const zwlr_layer_surface_v1_listener layerSurfaceListener = { layerConfigure, noopLayerClosed };
static const wl_buffer_listener bufferListener = { bufferRelease };
static const wp_fractional_scale_v1_listener fractionalScaleListener = { fractionalPreferred };

// ============================================================
// WlBackend
// ============================================================
WlBackend::WlBackend(QObject* parent) : QObject(parent) {}

WlBackend::~WlBackend() {
  if (shotFrame_) shotFailed(this, shotFrame_);
  if (vk_) zwp_virtual_keyboard_v1_destroy(vk_);
  if (xkbMap_) xkb_keymap_unref(xkbMap_);
  if (xkbCtx_) xkb_context_unref(xkbCtx_);
  if (fractionalScale_) wp_fractional_scale_v1_destroy(fractionalScale_);
  if (viewport_) wp_viewport_destroy(viewport_);
  if (layerSurface_) zwlr_layer_surface_v1_destroy(layerSurface_);
  if (surface_) wl_surface_destroy(surface_);
  destroyBuffers();
  for (const auto& retired : retiredPools_) {
    wl_buffer_destroy(retired.buffer);
    wl_shm_pool_destroy(retired.pool);
    munmap(retired.data, retired.size);
  }
  if (pointer_) wl_pointer_destroy(pointer_);
  if (touch_) wl_touch_destroy(touch_);
  if (compositor_) wl_compositor_destroy(compositor_);
  if (shm_) wl_shm_destroy(shm_);
  if (seat_) wl_seat_destroy(seat_);
  if (output_) wl_output_destroy(output_);
  if (layerShell_) zwlr_layer_shell_v1_destroy(layerShell_);
  if (fsManager_) wp_fractional_scale_manager_v1_destroy(fsManager_);
  if (viewporter_) wp_viewporter_destroy(viewporter_);
  if (vkManager_) zwp_virtual_keyboard_manager_v1_destroy(vkManager_);
  if (screencopy_) zwlr_screencopy_manager_v1_destroy(screencopy_);
  if (registry_) wl_registry_destroy(registry_);
  if (display_) wl_display_disconnect(display_);
}

bool WlBackend::init() {
  display_ = wl_display_connect(nullptr);
  if (!display_) { qWarning() << "[WlBackend] 无法连接 Wayland display"; return false; }

  registry_ = wl_display_get_registry(display_);
  wl_registry_add_listener(registry_, &registryListener, this);
  wl_display_roundtrip(display_);

  if (!compositor_ || !shm_ || !layerShell_) {
    qWarning() << "[WlBackend] 合成器缺少 wl_compositor/wl_shm/wlr-layer-shell";
    return false;
  }
  if (output_)  wl_output_add_listener(output_, &outputListener, this);

  createSurface();
  if (fsManager_ && viewporter_) {
    fractionalScale_ = wp_fractional_scale_manager_v1_get_fractional_scale(fsManager_, surface_);
    wp_fractional_scale_v1_add_listener(fractionalScale_, &fractionalScaleListener, this);
  }
  if (viewporter_) viewport_ = wp_viewporter_get_viewport(viewporter_, surface_);

  wl_display_roundtrip(display_);
  wl_display_roundtrip(display_);

  if (logicalW_ <= 0 || logicalH_ <= 0) {
    qWarning() << "[WlBackend] 未收到有效 configure";
    return false;
  }
  maybeCreateBuffers();
  if (!pool_ || !shmData_) {
    qWarning() << "[WlBackend] shm 缓冲创建失败";
    return false;
  }

  setupVirtualKeyboard();
  loadSettings();
  layoutUi();
  initialized_ = true;
  setMode(0);

  // WPS 联动桥（HTTP 16666 + 加载项自动注册）——平台无关
  wps_ = new WpsBridge(this);
  wps_->onRealPos = [this](int pos) { wpsOnRealPos(pos); };
  wps_->onSlideshowBegin = [this](int pos) { wpsOnBegin(pos); };
  wps_->onSlideshowEnd = [this]() {
    pageCache_.clear(); savedNormalPage_ = 1;
    if (!whiteboard_) { currentPage_ = 1; loadPage(1); renderAndCommit(); }
  };
  wps_->start();

  notifier_ = new QSocketNotifier(wl_display_get_fd(display_), QSocketNotifier::Read, this);
  connect(notifier_, &QSocketNotifier::activated, this, [this]() {
    if (wl_display_dispatch(display_) < 0) {
      notifier_->setEnabled(false);
      qWarning() << "[WlBackend] display 断开";
      QCoreApplication::quit();
    }
  });
  notifier_->setEnabled(true);

  qDebug() << "[WlBackend] 初始化完成:" << logicalW_ << "x" << logicalH_ << "fracScale" << fracScale_;
  return true;
}

void WlBackend::createSurface() {
  surface_ = wl_compositor_create_surface(compositor_);
  layerSurface_ = zwlr_layer_shell_v1_get_layer_surface(
      layerShell_, surface_, output_, ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY, "sidera");
  zwlr_layer_surface_v1_add_listener(layerSurface_, &layerSurfaceListener, this);
  zwlr_layer_surface_v1_set_anchor(layerSurface_,
      ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP | ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM |
      ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT | ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
  zwlr_layer_surface_v1_set_keyboard_interactivity(layerSurface_, ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_NONE);
  zwlr_layer_surface_v1_set_exclusive_zone(layerSurface_, 0);
  wl_surface_commit(surface_);
}

void WlBackend::createPool(int size) {
  int fd = memfd_create("sidera-shm", MFD_CLOEXEC);
  if (fd < 0) return;
  if (ftruncate(fd, size) < 0) { ::close(fd); return; }
  shmData_ = mmap(nullptr, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  if (shmData_ == MAP_FAILED) { shmData_ = nullptr; ::close(fd); return; }
  shmSize_ = size;
  pool_ = wl_shm_create_pool(shm_, fd, size);
  ::close(fd);
}

void WlBackend::createBuffers() {
  int stride = width_ * 4;
  int bsize  = stride * height_;
  createPool(bsize * 2);
  if (!pool_ || !shmData_) return;
  for (int i = 0; i < 2; i++) {
    bufs_[i].buffer = wl_shm_pool_create_buffer(pool_, i * bsize, width_, height_, stride, WL_SHM_FORMAT_ARGB8888);
    bufs_[i].size = bsize;
    wl_buffer_add_listener(bufs_[i].buffer, &bufferListener, this);
  }
  bufferW_ = width_; bufferH_ = height_;
}

void WlBackend::destroyBuffers() {
  for (auto& b : bufs_) { if (b.buffer) wl_buffer_destroy(b.buffer); b = Buf{}; }
  if (pool_) wl_shm_pool_destroy(pool_);
  if (shmData_) munmap(shmData_, shmSize_);
  pool_ = nullptr; shmData_ = nullptr; shmSize_ = 0;
  bufferW_ = bufferH_ = 0;
}

void WlBackend::maybeCreateBuffers() {
  if (logicalW_ <= 0 || logicalH_ <= 0) return;
  const double scale = viewport_ && fractionalScale_ ? fracScale_ :
      (wl_surface_get_version(surface_) >= 3 ? scale_ : 1);
  const qint64 width = qRound64(logicalW_ * scale), height = qRound64(logicalH_ * scale);
  if (width <= 0 || height <= 0 || width > 32768 || height > 32768 || width * height > 67108864) {
    width_ = height_ = 0;
    return;
  }
  width_ = int(width); height_ = int(height);
  if (pool_ && bufferW_ == width_ && bufferH_ == height_) return;
  // The displayed buffer may only be released after a replacement is committed.
  // Keep its entire old pool alive while creating the replacement at the new size.
  if ((bufs_[0].busy && bufs_[1].busy) || retiredPools_.size() >= 2) return;
  if (pool_ && (bufs_[0].busy || bufs_[1].busy)) {
    const int busy = bufs_[0].busy ? 0 : 1;
    retiredPools_.append({pool_, shmData_, shmSize_, bufs_[busy].buffer});
    if (bufs_[1 - busy].buffer) wl_buffer_destroy(bufs_[1 - busy].buffer);
    bufs_[0] = Buf{}; bufs_[1] = Buf{};
    pool_ = nullptr; shmData_ = nullptr; shmSize_ = 0;
  }
  destroyBuffers();
  createBuffers();
}

// ============================================================
// 虚拟键盘（翻页 / 退出放映）
// ============================================================
void WlBackend::setupVirtualKeyboard() {
  if (!vkManager_ || !seat_) return;
  vk_ = zwp_virtual_keyboard_manager_v1_create_virtual_keyboard(vkManager_, seat_);
  xkbCtx_ = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
  if (!xkbCtx_) return;
  xkbMap_ = xkb_keymap_new_from_names(xkbCtx_, nullptr, XKB_KEYMAP_COMPILE_NO_FLAGS);
  if (!xkbMap_) return;
  char* str = xkb_keymap_get_as_string(xkbMap_, XKB_KEYMAP_FORMAT_TEXT_V1);
  if (!str) return;
  size_t len = std::strlen(str) + 1;
  int fd = memfd_create("sidera-keymap", MFD_CLOEXEC);
  if (fd >= 0) {
    if (ftruncate(fd, len) < 0) { ::close(fd); free(str); return; }
    void* m = mmap(nullptr, len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (m != MAP_FAILED) { std::memcpy(m, str, len); munmap(m, len); }
    zwp_virtual_keyboard_v1_keymap(vk_, WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1, fd, len);
    wl_display_flush(display_);   // 先 flush，确保 fd 随消息发出后再关闭
    ::close(fd);
  }
  free(str);
}

void WlBackend::sendKeysym(xkb_keysym_t sym) {
  if (!vk_ || !xkbMap_) return;
  xkb_keycode_t kc = 0;
  xkb_keycode_t lo = xkb_keymap_min_keycode(xkbMap_), hi = xkb_keymap_max_keycode(xkbMap_);
  for (xkb_keycode_t k = lo; k <= hi && !kc; k++) {
    const xkb_keysym_t* syms = nullptr;
    int n = xkb_keymap_key_get_syms_by_level(xkbMap_, k, 0, 0, &syms);
    for (int i = 0; i < n; i++) if (syms[i] == sym) { kc = k; break; }
  }
  if (!kc) return;
  uint32_t evdev = (uint32_t)(kc - 8);   // 协议使用 evdev scancode
  uint32_t t = (uint32_t)monotonicMs();
  zwp_virtual_keyboard_v1_key(vk_, t, evdev, 1);
  zwp_virtual_keyboard_v1_key(vk_, t + 1, evdev, 0);
  wl_display_flush(display_);
}

// ============================================================
// 渲染
// ============================================================
void WlBackend::render(QImage& img) {
  QPainter p(&img);
  p.setRenderHint(QPainter::Antialiasing, true);
  p.scale(double(width_) / logicalW_, double(height_) / logicalH_);

  p.setCompositionMode(QPainter::CompositionMode_Source);
  if (whiteboard_) p.fillRect(0, 0, logicalW_, logicalH_, kWhiteboardColors[wbBgIndex_]);
  else             p.fillRect(0, 0, logicalW_, logicalH_, Qt::transparent);

  p.setCompositionMode(QPainter::CompositionMode_SourceOver);
  if (!ink_.isNull()) p.drawPixmap(0, 0, ink_);
  drawSidebar(p, false);
  drawSidebar(p, true);
  drawPopups(p);
  drawMoreMenu(p);
  drawSettings(p);

  // 白板模式：左下角页码
  if (whiteboard_) {
    QFont f = p.font(); f.setBold(true); f.setPixelSize(16); p.setFont(f);
    p.setPen(QColor("#0ABAB5"));
    p.drawText(QRect(16, logicalH_ - 46, 320, 32), Qt::AlignLeft | Qt::AlignVCenter,
               QString::fromUtf8("第 %1 页 / %2").arg(currentPage_).arg(kCacheBoard));
  }
  p.end();
}

void WlBackend::startAnim() {
  animT_ = 0.0;
  if (!animTimer_) {
    animTimer_ = new QTimer(this);
    animTimer_->setInterval(16);
    connect(animTimer_, &QTimer::timeout, this, [this]() {
      animT_ += 1.0 / 8.0;
      if (animT_ >= 1.0) { animT_ = 1.0; animTimer_->stop(); }
      renderAndCommit();
    });
  }
  animTimer_->start();
}

void WlBackend::renderAndCommit() {
  if (!surface_) return;
  pendingRender_ = true;
  maybeCreateBuffers();
  if (!pool_ || bufferW_ != width_ || bufferH_ != height_ || width_ <= 0 || height_ <= 0) return;
  int idx = (curBuf_ == 0) ? 1 : 0;
  if (bufs_[idx].busy) idx = curBuf_;
  if (bufs_[idx].busy) return;
  if (ink_.size() != QSize(logicalW_, logicalH_)) {
    if (ink_.isNull()) { ink_ = QPixmap(logicalW_, logicalH_); ink_.fill(Qt::transparent); }
    else ink_ = ink_.scaled(logicalW_, logicalH_, Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
    undoStack_.clear();
  }
  curBuf_ = idx;
  bufs_[idx].busy = true;
  uchar* base = (uchar*)shmData_ + (size_t)idx * bufs_[idx].size;
  QImage img(base, width_, height_, width_ * 4, QImage::Format_ARGB32_Premultiplied);
  render(img);
  if (qgetenv("SIDERA_WL_DUMP") == "1") img.save("/tmp/sidera_frame.png");
  wl_surface_attach(surface_, bufs_[idx].buffer, 0, 0);
  if (viewport_) wp_viewport_set_destination(viewport_, logicalW_, logicalH_);
  else if (wl_surface_get_version(surface_) >= 3) wl_surface_set_buffer_scale(surface_, scale_);
  if (wl_surface_get_version(surface_) >= 4) wl_surface_damage_buffer(surface_, 0, 0, width_, height_);
  else wl_surface_damage(surface_, 0, 0, logicalW_, logicalH_);
  pendingRender_ = false;
  if (mode_ == 0 && !whiteboard_) setInputRegionSidebar();
  else setInputRegionFull();
  wl_surface_commit(surface_);
  wl_display_flush(display_);
}

// ============================================================
// UI 绘制
// ============================================================
void WlBackend::drawText(QPainter& p, const QRect& r, const QString& t, const QColor& c, int px, bool bold) {
  QFont f = p.font();
  f.setPixelSize(qMax(8, px));
  f.setBold(bold);
  p.setFont(f);
  p.setPen(c);
  p.drawText(r, Qt::AlignCenter, t);
}

void WlBackend::drawBtnStyle(QPainter& p, const QRect& r, const QColor& fill, const QColor& border, double bw, double radius) {
  p.setBrush(fill.alpha() > 0 ? QBrush(fill) : QBrush(Qt::NoBrush));
  p.setPen(border.alpha() > 0 ? QPen(border, bw) : QPen(Qt::NoPen));
  if (radius > 0) p.drawRoundedRect(r, radius, radius);
  else            p.drawRect(r);
}

void WlBackend::drawIcon(QPainter& p, const QRect& r, int which) {
  int s = sbIcon();
  QPixmap pm = (which == 0) ? makeCursorIcon(s) : (which == 1) ? makePenIcon(s) : makeEraserIcon(s);
  p.drawPixmap(r.center().x() - s / 2, r.center().y() - s / 2, pm);
}

void WlBackend::drawSidebar(QPainter& p, bool right) {
  QRect sr = right ? sidebarRectR_ : sidebarRect_;
  const QVector<QRect>& btns = right ? buttonsR_ : buttons_;

  QRectF r = sr.adjusted(1, 1, -1, -1);
  qreal rad = r.width() / 2.0;
  p.setPen(Qt::NoPen);
  p.setBrush(QColor(42, 42, 50, sidebarAlpha_));
  p.drawRoundedRect(r, rad, rad);
  QLinearGradient g(r.topLeft(), QPointF(r.center().x(), r.top() + r.height() * 0.45));
  g.setColorAt(0.0, QColor(255, 255, 255, 20));
  g.setColorAt(0.3, QColor(255, 255, 255, 8));
  g.setColorAt(1.0, QColor(255, 255, 255, 0));
  p.setBrush(g); p.setPen(Qt::NoPen);
  p.drawRoundedRect(r, rad, rad);
  p.setBrush(Qt::NoBrush);
  p.setPen(QPen(QColor(90, 90, 100), 2.0));
  p.drawRoundedRect(r, rad, rad);

  if (collapsed_) {
    if (!btns.isEmpty()) {
      QRect b = btns.first();
      drawBtnStyle(p, b, QColor("#222436"), QColor(255, 213, 74, 153), 2, sbBtn() / 2.0);
      QFont f = p.font(); f.setPixelSize(12); f.setBold(true); p.setFont(f);
      p.setPen(QColor("#ffd54a"));
      p.drawText(b, Qt::AlignCenter | Qt::TextWordWrap, QString::fromUtf8("展开侧边栏"));
    }
    return;
  }

  { int d = sbDot();
    QPointF c(sr.center().x(), sr.top() - d / 4);
    p.setBrush(QColor(80, 85, 100, 240));
    p.setPen(QPen(QColor(136, 136, 136), 1.5));
    p.drawEllipse(c, d / 2.0, d / 2.0); }

  const double radPill = sbBtn() / 2.0;
  const double radRect = sbBtn() * 0.35;
  for (int i = 0; i < btns.size(); i++) {
    QRect br = btns[i];
    const bool hov = (i == hoverBtn_ && right == hoverRight_);
    switch (i) {
      case BtnCollapse:
        drawBtnStyle(p, br, hov ? QColor("#2f3250") : QColor("#222436"), QColor(255, 213, 74, 153), 2, radPill);
        drawText(p, br, QString::fromUtf8("收缩"), QColor("#ffd54a"), sbBtn() * 13 / 38, true);
        break;
      case BtnCursor: case BtnPen: case BtnEraser: {
        int m = i - BtnCursor;
        if (mode_ == m) drawBtnStyle(p, br, hov ? QColor(10, 186, 181, 140) : QColor(10, 186, 181, 102), QColor("#7fe9e4"), 2, radPill);
        else if (hov)   drawBtnStyle(p, br, QColor(255, 255, 255, 26), QColor(Qt::transparent), 0, radPill);
        drawIcon(p, br, m);
        break;
      }
      case BtnClear:
        drawBtnStyle(p, br, hov ? QColor("#26c6da") : QColor("#00bcd4"), QColor("#4dd0e1"), 2, radRect);
        drawText(p, br, QString::fromUtf8("清除"), QColor("#06343a"), sbBtn() * 16 / 38, true);
        break;
      case BtnUndo:
        drawBtnStyle(p, br, hov ? QColor(255, 154, 60, 46) : QColor(Qt::transparent), QColor(255, 154, 60, 179), 2, radRect);
        drawText(p, br, QString::fromUtf8("撤回"), QColor("#ff9a3c"), sbBtn() * 16 / 38, true);
        break;
      case BtnPrev:
        drawBtnStyle(p, br, hov ? QColor("#5555aa") : QColor("#3a3a4a"), QColor("#6688cc"), 1.5, radPill);
        drawText(p, br, QString::fromUtf8("\342\226\262"), QColor("#ccccff"), sbBtn() * 12 / 19, true);
        break;
      case BtnNext:
        drawBtnStyle(p, br, hov ? QColor("#5555aa") : QColor("#3a3a4a"), QColor("#6688cc"), 1.5, radPill);
        drawText(p, br, QString::fromUtf8("\342\226\274"), QColor("#ccccff"), sbBtn() * 12 / 19, true);
        break;
      case BtnExit:
        if (whiteboard_) {
          QColor c = kWhiteboardColors[wbBgIndex_];
          QString fg = (c.lightness() > 128) ? "#111111" : "#ffffff";
          drawBtnStyle(p, br, c, QColor("#ffffff"), 2, 4);
          drawText(p, br, QString::fromUtf8("背景\n颜色"), QColor(fg), sbBtn() * 11 / 34, true);
        } else {
          drawBtnStyle(p, br, hov ? QColor("#ff4d4d") : QColor("#d33a3a"), QColor("#ff8080"), 2, 4);
          drawText(p, br, QString::fromUtf8("退出\n放映"), QColor("#ffffff"), sbBtn() * 12 / 34, true);
        }
        break;
      case BtnWhiteboard:
        if (whiteboard_) {
          drawBtnStyle(p, br, QColor("#f4f4f4"), QColor("#ff9800"), 2, radRect);
          drawText(p, br, QString::fromUtf8("白板"), QColor("#111111"), sbBtn() * 16 / 38, true);
        } else {
          drawBtnStyle(p, br, hov ? QColor("#777777") : QColor("#555555"), QColor("#999999"), 2, radRect);
          drawText(p, br, QString::fromUtf8("白板"), QColor("#eeeeee"), sbBtn() * 16 / 38, false);
        }
        break;
      case BtnMore:
        drawBtnStyle(p, br, hov ? QColor("#222222") : QColor("#000000"), QColor("#333333"), 2, radPill);
        drawText(p, br, QString::fromUtf8("\342\213\257"), QColor("#ffffff"), sbBtn() * 16 / 38, true);
        break;
    }
  }
}

void WlBackend::drawPopups(QPainter& p) {
  if (popupType_ == 0) return;
  p.save();
  if (animT_ < 1.0) {
    p.setOpacity(animT_);
    QPointF c = popupRect_.center();
    double s = 0.92 + 0.08 * animT_;
    p.translate(c); p.scale(s, s); p.translate(-c);
  }
  QRect pr = popupRect_;
  p.setPen(Qt::NoPen); p.setBrush(QColor(42, 42, 50, 245));
  p.drawRoundedRect(pr, 14, 14);
  p.setPen(QPen(QColor(102, 102, 102), 2)); p.setBrush(Qt::NoBrush);
  p.drawRoundedRect(pr.adjusted(1, 1, -1, -1), 14, 14);

  if (popupType_ == 1) {
    drawText(p, QRect(pr.x() + 12, pr.y() + 8, pr.width() - 24, 18),
             QString::fromUtf8("画笔颜色"), QColor("#cccccc"), 12, true);
    for (int i = 0; i < colorRects_.size(); i++) {
      QRect r = colorRects_[i];
      p.setPen(Qt::NoPen); p.setBrush(kColors[i]);
      p.drawEllipse(r.center(), r.width() / 2, r.height() / 2);
      if (i == curColor_) { p.setPen(QPen(Qt::white, 3)); p.setBrush(Qt::NoBrush);
        p.drawEllipse(r.center(), r.width() / 2 + 1, r.height() / 2 + 1); }
    }
    drawText(p, QRect(pr.x() + 12, pr.y() + 150, pr.width() - 24, 18),
             QString::fromUtf8("画笔粗细"), QColor("#cccccc"), 12, true);
    static const char* labels[] = { "\347\273\206 3", "\344\270\255 6", "\347\262\227 10" };
    for (int i = 0; i < penSizeRects_.size(); i++) {
      QRect r = penSizeRects_[i];
      bool sel = (i == curPenSize_);
      drawBtnStyle(p, r, sel ? QColor("#3377cc") : QColor("#444444"),
                   sel ? QColor("#5599ff") : QColor("#666666"), 2, 6);
      drawText(p, r, QString::fromUtf8(labels[i]), QColor("#ffffff"), 13, true);
    }
  } else if (popupType_ == 2) {
    drawText(p, QRect(pr.x() + 12, pr.y() + 8, pr.width() - 24, 18),
             QString::fromUtf8("橡皮擦大小"), QColor("#cccccc"), 12, true);
    static const char* labels[] = { "\345\260\217 12", "\344\270\255 24", "\345\244\247 48" };
    for (int i = 0; i < eraserSizeRects_.size(); i++) {
      QRect r = eraserSizeRects_[i];
      bool sel = (i == curEraserSize_);
      drawBtnStyle(p, r, sel ? QColor("#ff8800") : QColor("#444444"),
                   sel ? QColor("#ffaa00") : QColor("#666666"), 2, 6);
      drawText(p, r, QString::fromUtf8(labels[i]), sel ? QColor("#000000") : QColor("#ffffff"), 13, true);
    }
    drawBtnStyle(p, eraserClearRect_, QColor("#555555"), QColor("#666666"), 2, 6);
    drawText(p, eraserClearRect_, QString::fromUtf8("清除全部"), QColor("#ffffff"), 13, true);
  }
  p.restore();
}

void WlBackend::drawMoreMenu(QPainter& p) {
  if (menu_ == 0) return;
  p.save();
  if (animT_ < 1.0) {
    p.setOpacity(animT_);
    QPointF c = menuRect_.center();
    double s = 0.92 + 0.08 * animT_;
    p.translate(c); p.scale(s, s); p.translate(-c);
  }
  QRect mr = menuRect_;
  p.setPen(Qt::NoPen); p.setBrush(QColor(43, 43, 51, 250));
  p.drawRoundedRect(mr, 8, 8);
  p.setPen(QPen(QColor(80, 80, 90), 1.5)); p.setBrush(Qt::NoBrush);
  p.drawRoundedRect(mr.adjusted(1, 1, -1, -1), 8, 8);
  static const char* items[] = { "\346\210\252\345\233\276", "\350\256\276\347\275\256" };  // 截图 / 设置
  for (int i = 0; i < moreRects_.size(); i++)
    drawText(p, moreRects_[i], QString::fromUtf8(items[i]), QColor("#eeeeee"), 14, false);
  p.restore();
}

// ============================================================
// 输入区域
// ============================================================
void WlBackend::setInputRegionSidebar() {
  if (!compositor_ || !surface_) return;
  wl_region* region = wl_compositor_create_region(compositor_);
  wl_region_add(region, sidebarRect_.x(), sidebarRect_.y(), sidebarRect_.width(), sidebarRect_.height());
  wl_region_add(region, sidebarRectR_.x(), sidebarRectR_.y(), sidebarRectR_.width(), sidebarRectR_.height());
  if (popupType_ != 0 && !popupRect_.isEmpty())
    wl_region_add(region, popupRect_.x(), popupRect_.y(), popupRect_.width(), popupRect_.height());
  if (menu_ != 0 && !menuRect_.isEmpty())
    wl_region_add(region, menuRect_.x(), menuRect_.y(), menuRect_.width(), menuRect_.height());
  if (settingsOpen_ && !settingsRect_.isEmpty())
    wl_region_add(region, settingsRect_.x(), settingsRect_.y(), settingsRect_.width(), settingsRect_.height());
  wl_surface_set_input_region(surface_, region);
  wl_region_destroy(region);
}

void WlBackend::setInputRegionFull() {
  if (surface_) wl_surface_set_input_region(surface_, nullptr);
}

// ============================================================
// 输入处理
// ============================================================
void WlBackend::onPointerMove(const QPoint& pos) {
  curPos_ = pos;
  if (draggingSlider_) { updateSlider(draggingSlider_, pos); renderAndCommit(); return; }
  if (!drawing_) {
    // 悬停高亮
    bool hr = false;
    int hb = settingsOpen_ ? -1 : buttonAt(pos, &hr);
    if (hb != hoverBtn_ || hr != hoverRight_) {
      hoverBtn_ = hb; hoverRight_ = hr;
      renderAndCommit();
    }
    return;
  }
  QPoint prev = lastPt_;
  lastPt_ = pos;
  if (mode_ == 2) drawEraseSegment(&ink_, prev, pos, eraserWidth());
  else { int w = timedWidth(); drawPenTapered(&ink_, kColors[curColor_], prev, pos, lastPenW_, w); lastPenW_ = w; }
  renderAndCommit();
}

void WlBackend::onPointerButton(uint32_t button, bool pressed) {
  if (button != 0x110) return;
  if (pressed) {
    // 设置面板优先
    if (settingsOpen_) {
      int sl = settingsSliderAt(curPos_);
      if (sl) { draggingSlider_ = sl; updateSlider(sl, curPos_); renderAndCommit(); return; }
      if (autostartRect_.contains(curPos_)) { setAutoStart(!isAutoStart()); saveSettings(); renderAndCommit(); return; }
      if (quitRect_.contains(curPos_)) { QCoreApplication::quit(); return; }
      if (closeRect_.contains(curPos_)) { closeSettings(); renderAndCommit(); return; }
      if (!settingsRect_.contains(curPos_)) { closeSettings(); renderAndCommit(); return; }
      return;
    }
    // 更多菜单
    if (menu_ != 0) {
      int mi = moreAt(curPos_);
      if (mi == 0) { menu_ = 0; if (mode_ == 0) setInputRegionSidebar(); doScreenshot(); return; }
      if (mi == 1) { menu_ = 0; openSettings(); return; }
      menu_ = 0;
      if (mode_ == 0) setInputRegionSidebar();
      // 继续向下判断（可能点在别处）
    }
    // 弹窗
    if (popupType_ != 0 && inPopup(curPos_)) {
      if (popupType_ == 1) {
        int c = colorAt(curPos_);   if (c >= 0) { curColor_ = c; renderAndCommit(); return; }
        int s = penSizeAt(curPos_); if (s >= 0) { curPenSize_ = s; renderAndCommit(); return; }
      } else {
        int s = eraserSizeAt(curPos_); if (s >= 0) { curEraserSize_ = s; renderAndCommit(); return; }
        if (eraserClearRect_.contains(curPos_)) { clearInk(); renderAndCommit(); return; }
      }
      return;
    }
    bool right = false;
    int bi = buttonAt(curPos_, &right);
    if (bi >= 0) { clickButton(bi, right); return; }
    if (mode_ != 0) {
      closePopup();
      pushUndo();
      drawing_ = true;
      lastPt_ = curPos_;
      strokeStartMs_ = monotonicMs();
      lastPenW_ = timedWidth();
      if (mode_ == 2) drawEraseSegment(&ink_, lastPt_, lastPt_, eraserWidth());
      else            drawPenTapered(&ink_, kColors[curColor_], lastPt_, lastPt_, lastPenW_, lastPenW_);
      renderAndCommit();
    }
  } else {
    if (draggingSlider_) { draggingSlider_ = 0; saveSettings(); }
    if (drawing_) drawing_ = false;
  }
}

void WlBackend::onTouchDown(int id, const QPoint& pos) {
  if (touchId_ >= 0) return;
  touchId_ = id;
  curPos_ = pos;
  onPointerButton(0x110, true);
}
void WlBackend::onTouchMove(int id, const QPoint& pos) {
  if (id != touchId_) return;
  onPointerMove(pos);
}
void WlBackend::onTouchUp(int id) {
  if (id != touchId_) return;
  touchId_ = -1;
  onPointerButton(0x110, false);
}
void WlBackend::cancelInput() {
  drawing_ = false;
  touchId_ = -1;
  draggingSlider_ = 0;
  strokeStartMs_ = 0;
}

// ============================================================
// 布局 / 命中
// ============================================================
int WlBackend::sbW() const { return int(kSbBaseW * sbScale_); }
int WlBackend::sbBtn() const { return int(kSbBaseBtn * sbScale_); }
int WlBackend::sbIcon() const { return int(kSbBaseIcon * sbScale_); }
int WlBackend::sbDot() const { return int(kSbBaseDot * sbScale_); }

void WlBackend::layoutUi() {
  const int W = sbW(), B = sbBtn();
  const int top = 12, bottom = 10, spacing = 6;
  const int x = 4;
  const int btnX = x + 8 + (W - 16 - B) / 2;
  const int rightX = logicalW_ - W - 4;
  const int btnXR = rightX + 8 + (W - 16 - B) / 2;

  buttons_.clear(); buttonsR_.clear();
  colorRects_.clear(); penSizeRects_.clear(); eraserSizeRects_.clear(); moreRects_.clear();
  eraserClearRect_ = QRect(); popupRect_ = QRect(); menuRect_ = QRect(); settingsRect_ = QRect();

  if (collapsed_) {
    int h = B * 3 + 12, y = (logicalH_ - h) / 2;
    sidebarRect_  = QRect(x, y, W, h);
    sidebarRectR_ = QRect(rightX, y, W, h);
    buttons_.append(QRect(x + 4, y + 6, W - 8, h - 12));
    buttonsR_.append(QRect(rightX + 4, y + 6, W - 8, h - 12));
    return;
  }

  int n = BtnCount;
  int sidebarH = top + bottom + n * B + (n - 1) * spacing;
  int y = (logicalH_ - sidebarH) / 2;
  sidebarRect_  = QRect(x, y, W, sidebarH);
  sidebarRectR_ = QRect(rightX, y, W, sidebarH);
  for (int i = 0; i < n; i++) {
    int by = y + top + i * (B + spacing);
    buttons_.append(QRect(btnX, by, B, B));
    buttonsR_.append(QRect(btnXR, by, B, B));
  }

  if (popupType_ != 0) {
    int pw = (popupType_ == 1) ? 200 : 220;
    int ph = (popupType_ == 1) ? 268 : 175;
    int px = popupOnRight_ ? (rightX - pw - 8) : (x + W + 8);
    int py = y + (sidebarH - ph) / 2;
    popupRect_ = QRect(px, py, pw, ph);
    if (popupType_ == 1) {
      const int perRow = 4, sw = 30, gap = 8;
      int x0 = px + 12, y0 = py + 34;
      for (int i = 0; i < kColors.size(); i++)
        colorRects_.append(QRect(x0 + (i % perRow) * (sw + gap), y0 + (i / perRow) * (sw + gap), sw, sw));
      int bw = (200 - 24 - 2 * 8) / 3, bx = px + 12, byy = py + 174;
      for (int i = 0; i < kPenSizes.size(); i++) penSizeRects_.append(QRect(bx + i * (bw + 8), byy, bw, 36));
    } else {
      int bw = (220 - 24 - 2 * 8) / 3, bx = px + 12, byy = py + 34;
      for (int i = 0; i < kEraserSizes.size(); i++) eraserSizeRects_.append(QRect(bx + i * (bw + 8), byy, bw, 36));
      eraserClearRect_ = QRect(px + 12, py + 90, 220 - 24, 38);
    }
  }

  if (menu_ != 0) {
    QRect mb = popupOnRight_ ? buttonsR_[BtnMore] : buttons_[BtnMore];
    int mw = 120, mh = 2 * 36 + 8;
    int mx = popupOnRight_ ? (mb.x() - mw - 6) : (mb.x() + mb.width() + 6);
    int my = mb.y();
    if (my + mh > logicalH_) my = logicalH_ - mh - 4;
    menuRect_ = QRect(mx, my, mw, mh);
    moreRects_.append(QRect(mx + 4, my + 4, mw - 8, 32));
    moreRects_.append(QRect(mx + 4, my + 40, mw - 8, 32));
  }

  if (settingsOpen_) {
    int pw = 340, ph = 360;
    int px = (logicalW_ - pw) / 2, py = (logicalH_ - ph) / 2;
    settingsRect_ = QRect(px, py, pw, ph);
    int lx = px + 28, lw = pw - 56;
    alphaTrack_    = QRect(lx, py + 96,  lw, 8);
    sizeTrack_     = QRect(lx, py + 166, lw, 8);
    autostartRect_ = QRect(lx, py + 206, lw, 40);
    quitRect_      = QRect(lx, py + 258, lw, 40);
    closeRect_     = QRect(lx, py + 310, lw, 38);
  }
}

int WlBackend::buttonAt(const QPoint& p, bool* right) const {
  for (int i = 0; i < buttons_.size(); i++) if (buttons_[i].contains(p)) { if (right) *right = false; return i; }
  for (int i = 0; i < buttonsR_.size(); i++) if (buttonsR_[i].contains(p)) { if (right) *right = true; return i; }
  return -1;
}
int WlBackend::colorAt(const QPoint& p) const {
  for (int i = 0; i < colorRects_.size(); i++) if (colorRects_[i].contains(p)) return i;
  return -1;
}
int WlBackend::penSizeAt(const QPoint& p) const {
  for (int i = 0; i < penSizeRects_.size(); i++) if (penSizeRects_[i].contains(p)) return i;
  return -1;
}
int WlBackend::eraserSizeAt(const QPoint& p) const {
  for (int i = 0; i < eraserSizeRects_.size(); i++) if (eraserSizeRects_[i].contains(p)) return i;
  return -1;
}
int WlBackend::moreAt(const QPoint& p) const {
  for (int i = 0; i < moreRects_.size(); i++) if (moreRects_[i].contains(p)) return i;
  return -1;
}
bool WlBackend::inSidebar(const QPoint& p) const {
  return sidebarRect_.adjusted(-4, -4, 4, 4).contains(p) || sidebarRectR_.adjusted(-4, -4, 4, 4).contains(p);
}
bool WlBackend::inPopup(const QPoint& p) const { return popupType_ != 0 && popupRect_.contains(p); }
bool WlBackend::inMenu(const QPoint& p) const { return menu_ != 0 && menuRect_.contains(p); }

// ============================================================
// 动作
// ============================================================
void WlBackend::setMode(int m) {
  mode_ = m;
  closePopup();
  if (mode_ == 0) setInputRegionSidebar();
  else            setInputRegionFull();
  renderAndCommit();
}

void WlBackend::togglePopup(int type) {
  if (popupType_ == type) { closePopup(); renderAndCommit(); return; }
  popupType_ = type;
  menu_ = 0;
  layoutUi();
  if (mode_ == 0) setInputRegionSidebar();
  startAnim();
  renderAndCommit();
}

void WlBackend::closePopup() {
  if (popupType_ == 0) return;
  popupType_ = 0;
  popupRect_ = QRect();
  colorRects_.clear(); penSizeRects_.clear(); eraserSizeRects_.clear(); eraserClearRect_ = QRect();
  if (mode_ == 0) setInputRegionSidebar();
}

void WlBackend::toggleCollapse() {
  collapsed_ = !collapsed_;
  closePopup();
  layoutUi();
  if (mode_ == 0) setInputRegionSidebar();
  renderAndCommit();
}

void WlBackend::clickButton(int idx, bool right) {
  popupOnRight_ = right;
  switch (idx) {
    case BtnCollapse:   toggleCollapse(); break;
    case BtnCursor:     setMode(0); break;
    case BtnPen:        if (mode_ == 1) togglePopup(1); else setMode(1); break;
    case BtnEraser:     if (mode_ == 2) togglePopup(2); else setMode(2); break;
    case BtnClear:      clearInk(); break;
    case BtnUndo:       undo(); break;
    case BtnPrev:       goPrevPage(); break;
    case BtnNext:       goNextPage(); break;
    case BtnExit:       exitPresentation(); break;
    case BtnWhiteboard: toggleWhiteboard(); break;
    case BtnMore:
      menu_ = (menu_ == 1) ? 0 : 1;
      layoutUi();
      if (mode_ == 0) setInputRegionSidebar();
      if (menu_) startAnim();
      renderAndCommit();
      break;
  }
}

void WlBackend::pushUndo() {
  if (undoStack_.size() >= kMaxUndo) undoStack_.removeFirst();
  undoStack_.append(ink_);
}
void WlBackend::undo() {
  if (undoStack_.isEmpty()) return;
  ink_ = undoStack_.takeLast();
  renderAndCommit();
}
void WlBackend::clearInk() {
  ink_.fill(Qt::transparent);
  undoStack_.clear();
  renderAndCommit();
}

void WlBackend::savePage() {
  PageCache& cache = whiteboard_ ? wbCache_ : pageCache_;
  cache.insert(currentPage_, ink_);
}
void WlBackend::loadPage(int page) {
  ink_.fill(Qt::transparent);
  const QPixmap cached = (whiteboard_ ? wbCache_ : pageCache_).value(page);
  if (!cached.isNull()) { QPainter painter(&ink_); painter.drawPixmap(ink_.rect(), cached, cached.rect()); }
  undoStack_.clear();
}

void WlBackend::goNextPage() {
  // 白板：纯本地；非白板且加载项在线：交给加载项翻页（按真实页号回传驱动缓存）
  if (!whiteboard_ && wps_ && wps_->connected() && wps_->position() > 0) { wps_->enqueue("NEXT"); return; }
  savePage();
  if (whiteboard_) {
    currentPage_++;
    if (currentPage_ > kCacheBoard) currentPage_ = 1;
    loadPage(currentPage_);
  } else {
    currentPage_++;
    sendKeysym(XKB_KEY_Down);
    loadPage(currentPage_);
  }
  renderAndCommit();
}

void WlBackend::goPrevPage() {
  if (!whiteboard_ && wps_ && wps_->connected() && wps_->position() > 0) { wps_->enqueue("PREV"); return; }
  savePage();
  if (whiteboard_) {
    if (currentPage_ > 1) currentPage_--;
    loadPage(currentPage_);
  } else {
    if (currentPage_ > 1) currentPage_--;
    sendKeysym(XKB_KEY_Up);
    loadPage(currentPage_);
  }
  renderAndCommit();
}

// 加载项回传真实页号：存旧页、载新页（页内动画步不动缓存）
void WlBackend::wpsOnRealPos(int pos) {
  if (pos <= 0) return;
  if (whiteboard_) { savedNormalPage_ = pos; return; }
  if (pos == currentPage_) return;
  savePage();
  currentPage_ = pos;
  loadPage(pos);
  renderAndCommit();
}

// 放映开始：清空并落到起始页
void WlBackend::wpsOnBegin(int pos) {
  pageCache_.clear();
  if (whiteboard_) { savedNormalPage_ = pos > 0 ? pos : 1; return; }
  currentPage_ = pos > 0 ? pos : 1;
  loadPage(currentPage_);
  renderAndCommit();
}

void WlBackend::exitPresentation() {
  if (whiteboard_) {
    wbBgIndex_ = (wbBgIndex_ + 1) % 2;
    renderAndCommit();
    return;
  }
  if (mode_ != 0) { mode_ = 0; setInputRegionSidebar(); }
  if (!(wps_ && wps_->connected() && wps_->position() > 0 && wps_->enqueue("EXIT")))
    sendKeysym(XKB_KEY_Escape);
  renderAndCommit();
}

void WlBackend::toggleWhiteboard() {
  savePage();
  if (!whiteboard_) {
    savedNormalPage_ = currentPage_;
    whiteboard_ = true;
    currentPage_ = whiteboardPage_;
    loadPage(currentPage_);
  } else {
    whiteboardPage_ = currentPage_;
    whiteboard_ = false;
    currentPage_ = savedNormalPage_;
    loadPage(currentPage_);
  }
  if (wps_) wps_->setWhiteboard(whiteboard_);
  renderAndCommit();
}

int WlBackend::timedWidth() {
  int base = kPenSizes[curPenSize_];
  if (strokeStartMs_ <= 0) return base;
  int minw = qMax(1, int(qRound(base * 0.85)));
  qint64 dt = monotonicMs() - strokeStartMs_;
  double t = qBound(0.0, double(dt) / 600.0, 1.0);
  return minw + int(qRound((base - minw) * t));
}

int WlBackend::eraserWidth() { return kEraserSizes[curEraserSize_]; }

// ============================================================
// 截图（zwlr_screencopy）
// ============================================================
void WlBackend::doScreenshot() {
  if (!screencopy_ || !output_ || shotFrame_) return;
  shotFrame_ = zwlr_screencopy_manager_v1_capture_output(screencopy_, 0, output_);
  shotInverted_ = false;
  zwlr_screencopy_frame_v1_add_listener(shotFrame_, &shotListener, this);
  auto* frame = shotFrame_;
  QTimer::singleShot(5000, this, [this, frame]() { if (shotFrame_ == frame) shotFailed(this, frame); });
}

static void shotBuffer(void* d, zwlr_screencopy_frame_v1* frame, uint32_t format, uint32_t w, uint32_t h, uint32_t stride) {
  WlBackend* s = static_cast<WlBackend*>(d);
  if ((format != WL_SHM_FORMAT_ARGB8888 && format != WL_SHM_FORMAT_XRGB8888) ||
      !w || !h || w > 32768 || h > 32768 || stride < uint64_t(w) * 4 ||
      uint64_t(stride) * h > 256u * 1024 * 1024) { shotFailed(d, frame); return; }
  s->shotFmt_ = (int)format; s->shotW_ = (int)w; s->shotH_ = (int)h; s->shotStride_ = (int)stride;
  size_t size = (size_t)stride * h;
  int fd = memfd_create("sidera-shot", MFD_CLOEXEC);
  if (fd < 0) { shotFailed(d, frame); return; }
  if (ftruncate(fd, size) < 0) { ::close(fd); shotFailed(d, frame); return; }
  void* m = mmap(nullptr, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  if (m == MAP_FAILED) { ::close(fd); shotFailed(d, frame); return; }
  s->shotFd_ = fd; s->shotData_ = m; s->shotSize_ = size;
  wl_shm_pool* pool = wl_shm_create_pool(s->shm_, fd, (int)size);
  s->shotBuffer_ = wl_shm_pool_create_buffer(pool, 0, w, h, stride, format);
  wl_shm_pool_destroy(pool);
  zwlr_screencopy_frame_v1_copy(frame, s->shotBuffer_);
}

static void shotReady(void* d, zwlr_screencopy_frame_v1* frame, uint32_t, uint32_t, uint32_t) {
  WlBackend* s = static_cast<WlBackend*>(d);
  QImage::Format fmt = (s->shotFmt_ == WL_SHM_FORMAT_XRGB8888) ? QImage::Format_RGB32
                                                               : QImage::Format_ARGB32;
  QImage img((const uchar*)s->shotData_, s->shotW_, s->shotH_, s->shotStride_, fmt);
  img = s->shotInverted_ ? img.mirrored(false, true) : img.copy();
  QString dir = QStandardPaths::writableLocation(QStandardPaths::PicturesLocation);
  if (dir.isEmpty()) dir = QDir::homePath();
  QDir().mkpath(dir);
  QString fn = dir + "/sidera_" + QDateTime::currentDateTime().toString("yyyyMMdd_hhmmss") + ".png";
  if (img.save(fn, "PNG")) {
    qDebug() << "[WlBackend] 截图已保存:" << fn;
    if (QGuiApplication::platformName() != "offscreen") QGuiApplication::clipboard()->setImage(img);
    else qWarning() << "截图已保存；当前会话无法直接复制到剪贴板。";
  }
  // 清理
  if (s->shotBuffer_) { wl_buffer_destroy(s->shotBuffer_); s->shotBuffer_ = nullptr; }
  if (s->shotData_) { munmap(s->shotData_, s->shotSize_); s->shotData_ = nullptr; }
  if (s->shotFd_ >= 0) { ::close(s->shotFd_); s->shotFd_ = -1; }
  zwlr_screencopy_frame_v1_destroy(frame);
  s->shotFrame_ = nullptr;
}

static void shotFailed(void* d, zwlr_screencopy_frame_v1* frame) {
  WlBackend* s = static_cast<WlBackend*>(d);
  qWarning() << "[WlBackend] 截图失败";
  if (s->shotBuffer_) { wl_buffer_destroy(s->shotBuffer_); s->shotBuffer_ = nullptr; }
  if (s->shotData_) { munmap(s->shotData_, s->shotSize_); s->shotData_ = nullptr; }
  if (s->shotFd_ >= 0) { ::close(s->shotFd_); s->shotFd_ = -1; }
  zwlr_screencopy_frame_v1_destroy(frame);
  s->shotFrame_ = nullptr;
}

// ============================================================
// 设置面板（在覆盖层内绘制）
// ============================================================
static QString wlConfigPath() { return QStandardPaths::writableLocation(QStandardPaths::ConfigLocation) + "/sidera/config"; }
static QString wlAutoStartPath() {
  QString dir = QStandardPaths::writableLocation(QStandardPaths::ConfigLocation) + "/autostart";
  QDir().mkpath(dir);
  return dir + "/sidera.desktop";
}

bool WlBackend::isAutoStart() const { return QFile::exists(wlAutoStartPath()); }

void WlBackend::setAutoStart(bool on) {
  QString path = wlAutoStartPath();
  if (on) {
    QFile f(path);
    if (f.open(QIODevice::WriteOnly | QIODevice::Text)) {
      QTextStream ts(&f);
      ts << "[Desktop Entry]\nType=Application\nName=Sidera\nComment=Sidera\nExec=sidera\nIcon=sidera\nTerminal=false\n";
    }
  } else {
    QFile::remove(path);
  }
}

void WlBackend::loadSettings() {
  QFile f(wlConfigPath());
  if (!f.open(QIODevice::ReadOnly | QIODevice::Text)) return;
  while (!f.atEnd()) {
    QString line = QString::fromUtf8(f.readLine()).trimmed();
    int eq = line.indexOf('='); if (eq < 0) continue;
    QString k = line.left(eq), v = line.mid(eq + 1);
    if (k == "sbScale") { bool ok = false; double scale = v.toDouble(&ok); if (ok && qIsFinite(scale)) sbScale_ = qBound(0.6, scale, 1.4); }
    else if (k == "sidebarAlpha") sidebarAlpha_ = qBound(30, v.toInt(), 255);
  }
}

void WlBackend::saveSettings() {
  QString path = wlConfigPath();
  QMap<QString, QString> kv;
  QFile f(path);
  if (f.open(QIODevice::ReadOnly | QIODevice::Text)) {
    while (!f.atEnd()) {
      QString line = QString::fromUtf8(f.readLine()).trimmed();
      int eq = line.indexOf('='); if (eq < 0) continue;
      kv[line.left(eq)] = line.mid(eq + 1);
    }
    f.close();
  }
  kv["sbScale"] = QString::number(sbScale_, 'f', 2);
  kv["sidebarAlpha"] = QString::number(sidebarAlpha_);
  QDir().mkpath(QFileInfo(path).absolutePath());
  if (QFileInfo(path).isSymLink()) { qWarning() << "Refusing symlink settings"; return; }
  QSaveFile pending(path);
  if (pending.open(QIODevice::WriteOnly | QIODevice::Text)) {
    QTextStream ts(&pending);
    for (auto it = kv.constBegin(); it != kv.constEnd(); ++it) ts << it.key() << "=" << it.value() << "\n";
    ts.flush();
    if (!pending.commit()) qWarning() << "Cannot save settings:" << pending.errorString();
  }
}

void WlBackend::openSettings() {
  closePopup();
  menu_ = 0;
  settingsOpen_ = true;
  draggingSlider_ = 0;
  layoutUi();
  if (mode_ == 0) setInputRegionSidebar();
  startAnim();
  renderAndCommit();
}

void WlBackend::closeSettings() {
  settingsOpen_ = false;
  draggingSlider_ = 0;
  settingsRect_ = QRect();
  if (mode_ == 0) setInputRegionSidebar();
}

int WlBackend::settingsSliderAt(const QPoint& p) const {
  if (alphaTrack_.adjusted(-10, -14, 10, 14).contains(p)) return 1;
  if (sizeTrack_.adjusted(-10, -14, 10, 14).contains(p)) return 2;
  return 0;
}

void WlBackend::updateSlider(int which, const QPoint& p) {
  if (which == 1) {
    const QRect& t = alphaTrack_;
    double fr = qBound(0.0, double(p.x() - t.x()) / qMax(1, t.width()), 1.0);
    sidebarAlpha_ = 30 + int(qRound(fr * (255 - 30)));
  } else if (which == 2) {
    const QRect& t = sizeTrack_;
    double fr = qBound(0.0, double(p.x() - t.x()) / qMax(1, t.width()), 1.0);
    sbScale_ = 0.6 + fr * (1.4 - 0.6);
    layoutUi();
  }
  if (mode_ == 0) setInputRegionSidebar();
}

void WlBackend::drawSlider(QPainter& p, const QRect& track, double frac) {
  frac = qBound(0.0, frac, 1.0);
  int h = track.height();
  p.setPen(Qt::NoPen);
  p.setBrush(QColor("#444444"));
  p.drawRoundedRect(track, h / 2.0, h / 2.0);
  QRect fill(track.x(), track.y(), int(track.width() * frac), h);
  p.setBrush(QColor("#3377cc"));
  p.drawRoundedRect(fill, h / 2.0, h / 2.0);
  int hx = track.x() + int(track.width() * frac);
  p.setBrush(QColor("#ffffff"));
  p.drawEllipse(QPointF(hx, track.center().y()), 9, 9);
}

void WlBackend::drawSettings(QPainter& p) {
  if (!settingsOpen_) return;
  p.save();
  if (animT_ < 1.0) {
    p.setOpacity(animT_);
    QPointF c = settingsRect_.center();
    double s = 0.92 + 0.08 * animT_;
    p.translate(c); p.scale(s, s); p.translate(-c);
  }
  QRect r = settingsRect_;
  p.setPen(Qt::NoPen); p.setBrush(QColor(43, 43, 51, 250));
  p.drawRoundedRect(r, 14, 14);
  p.setPen(QPen(QColor(90, 90, 100), 2)); p.setBrush(Qt::NoBrush);
  p.drawRoundedRect(r.adjusted(1, 1, -1, -1), 14, 14);

  drawText(p, QRect(r.x(), r.y() + 14, r.width(), 28), QString::fromUtf8("Sidera - 设置"), QColor("#eeeeee"), 18, true);

  drawText(p, QRect(r.x() + 28, r.y() + 60, 200, 20), QString::fromUtf8("侧边栏透明度"), QColor("#cccccc"), 14, false);
  drawText(p, QRect(r.right() - 90, r.y() + 60, 62, 20), QString::number(sidebarAlpha_), QColor("#aaddff"), 14, true);
  drawSlider(p, alphaTrack_, (sidebarAlpha_ - 30) / double(255 - 30));

  drawText(p, QRect(r.x() + 28, r.y() + 130, 200, 20), QString::fromUtf8("侧边栏大小"), QColor("#cccccc"), 14, false);
  drawText(p, QRect(r.right() - 90, r.y() + 130, 62, 20), QString::number(sbScale_, 'f', 1), QColor("#aaddff"), 14, true);
  drawSlider(p, sizeTrack_, (sbScale_ * 100 - 60) / double(140 - 60));

  bool on = isAutoStart();
  drawBtnStyle(p, autostartRect_, on ? QColor("#2a6e3f") : QColor("#444444"),
               on ? QColor("#3a8f55") : QColor("#666666"), 1, 6);
  drawText(p, autostartRect_, on ? QString::fromUtf8("开机自启动: 开") : QString::fromUtf8("开机自启动: 关"),
           QColor("#ffffff"), 14, false);

  drawBtnStyle(p, quitRect_, QColor("#8a2f2f"), QColor("#a83a3a"), 1, 6);
  drawText(p, quitRect_, QString::fromUtf8("退出软件"), QColor("#ffffff"), 14, false);

  drawBtnStyle(p, closeRect_, QColor("#3377cc"), QColor("#4488dd"), 1, 6);
  drawText(p, closeRect_, QString::fromUtf8("关闭"), QColor("#ffffff"), 15, true);
  p.restore();
}
