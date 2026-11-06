// ============================================================
// Sidera - Wayland 后端（wlr-layer-shell，面向 niri/sway/hyprland 等独立合成器）
// 软件渲染 wl_shm；输入用 wl_pointer / wl_touch；点击穿透用 wl_surface 输入区域
// ============================================================
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use crate::instance::Instance as UnixListener;
use std::time::Duration;

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{
    CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemedPointer, ThemeSpec,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::raw::RawPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
};
use tiny_skia::Pixmap;
use wayland_client::globals::{registry_queue_init, BindError, GlobalList};
use wayland_client::protocol::{wl_buffer, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{self, WpFractionalScaleV1};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_client::protocol::wl_keyboard::{KeyState, KeymapFormat};
use wayland_client::{Connection, EventQueue, Proxy, QueueHandle};

use crate::backend::{Backend, IRect, Key};
use crate::gesture::TouchKind;
use crate::text::Fonts;

#[cfg(test)]
#[path = "wayland_tests.rs"]
mod tests;

// ---- 虚拟键盘协议绑定（从仓库 XML 生成）----
mod vk {
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("../protocols/virtual-keyboard-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("../protocols/virtual-keyboard-unstable-v1.xml");
}
use vk::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use vk::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

mod vp {
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("../protocols/wlr-virtual-pointer-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("../protocols/wlr-virtual-pointer-unstable-v1.xml");
}
use vp::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use vp::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;
wayland_client::delegate_noop!(WlState: ignore ZwlrVirtualPointerManagerV1);
wayland_client::delegate_noop!(WlState: ignore ZwlrVirtualPointerV1);

/// 内嵌的 xkb keymap（构建期用 xkbcli 生成，避免运行时依赖 libxkbcommon）
const KEYMAP_XKB: &str = include_str!("keymap.xkb");

impl wayland_client::Dispatch<ZwpVirtualKeyboardManagerV1, ()> for WlState {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardManagerV1,
        _: <ZwpVirtualKeyboardManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<ZwpVirtualKeyboardV1, ()> for WlState {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardV1,
        _: <ZwpVirtualKeyboardV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<WpFractionalScaleManagerV1, ()> for WlState {
    fn event(
        _: &mut Self,
        _: &WpFractionalScaleManagerV1,
        _: <WpFractionalScaleManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<WpFractionalScaleV1, ()> for WlState {
    fn event(
        state: &mut Self,
        fractional: &WpFractionalScaleV1,
        event: <WpFractionalScaleV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            let sf = (scale as f64) / 120.0;
            if let Some(id) = state.surfaces.iter().find(|s| s.fractional.as_ref() == Some(fractional)).map(|s| s.id.clone()) {
                state.apply_scale(&id, sf);
            }
        }
    }
}

impl wayland_client::Dispatch<WpViewporter, ()> for WlState {
    fn event(
        _: &mut Self,
        _: &WpViewporter,
        _: <WpViewporter as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<WpViewport, ()> for WlState {
    fn event(
        _: &mut Self,
        _: &WpViewport,
        _: <WpViewport as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// ---------------- 平台对象（Backend 实现） ----------------
/// Each in-flight frame owns its storage, including across resize/scale changes.
/// A slot may only be written or replaced after wl_buffer.release.
struct ShmFrame {
    pool: RawPool,
    buffer: wl_buffer::WlBuffer,
    dims: (i32, i32),
    busy: bool,
}

impl Drop for ShmFrame {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}

fn shm_layout(w: i32, h: i32) -> Option<(i32, usize)> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let stride = w.checked_mul(4)?;
    // wl_shm_pool.size is signed 32-bit, even on 64-bit hosts.
    let len = stride.checked_mul(h)?;
    Some((stride, len as usize))
}

fn render_scale(requested: f64, viewport: bool, surface_version: u32) -> f64 {
    let scale = if requested.is_finite() { requested.clamp(1.0, 4.0) } else { 1.0 };
    if viewport {
        scale
    } else if surface_version >= 3 {
        scale.round()
    } else {
        1.0
    }
}

struct ShmFrames {
    shm: Shm,
    slots: [Option<ShmFrame>; 2],
}

impl ShmFrames {
    fn new(shm: &Shm) -> Self {
        Self { shm: Shm::from(shm.wl_shm().clone()), slots: [None, None] }
    }

    fn release(&mut self, buffer: &wl_buffer::WlBuffer) {
        for frame in self.slots.iter_mut().flatten() {
            if &frame.buffer == buffer {
                frame.busy = false;
            }
        }
    }

    fn write_frame(&mut self, back: &Pixmap, qh: &QueueHandle<WlState>)
        -> Result<Option<&wl_buffer::WlBuffer>, Box<dyn std::error::Error>>
    {
        let (w, h) = (back.width() as i32, back.height() as i32);
        let (stride, bytes) = shm_layout(w, h).ok_or("Wayland 画布尺寸过大")?;
        let Some(idx) = self.slots.iter().position(|b| b.as_ref().map_or(true, |b| !b.busy))
        else { return Ok(None) };
        if self.slots[idx].as_ref().map(|b| b.dims) != Some((w, h)) {
            let mut pool = RawPool::new(bytes, &self.shm)?;
            let buffer = pool.create_buffer(0, w, h, stride, wl_shm::Format::Argb8888, (), qh);
            self.slots[idx] = Some(ShmFrame { pool, buffer, dims: (w, h), busy: false });
        }
        let frame = self.slots[idx].as_mut().unwrap();
        // A free slot may be several frames old. Copy the authoritative canvas,
        // including changes made while both slots were busy.
        for (dst, src) in frame.pool.mmap().chunks_exact_mut(4).zip(back.data().chunks_exact(4)) {
            dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
        }
        frame.busy = true;
        Ok(Some(&frame.buffer))
    }
}

pub struct WlPlat {
    compositor: CompositorState,
    layer: LayerSurface,
    conn: Connection,
    qh: QueueHandle<WlState>,
    frames: RefCell<ShmFrames>,
    logical: Cell<(i32, i32)>,
    dims: Cell<(i32, i32)>, // 物理缓冲区尺寸
    scale: Cell<f64>,
    viewport: RefCell<Option<WpViewport>>,
    uinput: Rc<RefCell<Option<crate::uinput::KeyInjector>>>,
    back: RefCell<Pixmap>,
    vk: Rc<RefCell<Option<ZwpVirtualKeyboardV1>>>,
    pointer: RefCell<Option<ZwlrVirtualPointerV1>>,
    pending_click: Cell<Option<(i32, i32)>>,
    dirty: Cell<Option<(i32, i32, i32, i32)>>, // 脏区域包围盒（物理）x0,y0,x1,y1
    input_dirty: Cell<bool>,
}

impl WlPlat {
    /// 依据逻辑尺寸与 scale 重建物理缓冲区与 viewport 目标
    fn reconfigure(&self) {
        let (lw, lh) = self.logical.get();
        if lw <= 0 || lh <= 0 {
            return;
        }
        let sc = self.scale.get();
        let pw = ((lw as f64) * sc).round().max(1.0) as i32;
        let ph = ((lh as f64) * sc).round().max(1.0) as i32;
        if self.dims.get() == (pw, ph) {
            return;
        }
        if !crate::app::valid_geometry(lw, lh, sc) || shm_layout(pw, ph).is_none() {
            log::error!("[ERROR] Wayland 画布尺寸过大：{pw}×{ph}");
            return;
        }
        let Some(back) = Pixmap::new(pw as u32, ph as u32) else { return };
        *self.back.borrow_mut() = back;
        self.dims.set((pw, ph));
        // Do not clear, resize or destroy in-flight storage here. The next flush
        // replaces a released slot and submits the fully repainted new canvas.
        self.dirty.set(Some((0, 0, pw, ph)));
    }

    fn forward_click(&self) {
        let Some((x, y)) = self.pending_click.take() else { return };
        let Some(pointer) = self.pointer.borrow().clone() else { return };
        let (w,h) = self.logical.get();
        if w <= 0 || h <= 0 { return; }
        let time = crate::app::now_ms() as u32;
        pointer.motion_absolute(time, x.clamp(0,w-1) as u32, y.clamp(0,h-1) as u32, w as u32, h as u32);
        pointer.button(time, 0x111, wl_pointer::ButtonState::Pressed);
        pointer.frame();
        pointer.button(time.wrapping_add(1), 0x111, wl_pointer::ButtonState::Released);
        pointer.frame();
    }

    fn union_dirty(&self, x: i32, y: i32, w: i32, h: i32) {
        let (sw, sh) = self.dims.get();
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(sw);
        let y1 = (y + h).min(sh);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let r = match self.dirty.get() {
            None => (x0, y0, x1, y1),
            Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
        };
        self.dirty.set(Some(r));
    }
}

impl Backend for WlPlat {
    fn description(&self) -> &'static str { "原生 Wayland / layer-shell" }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn screen_size(&self) -> (i32, i32) {
        self.logical.get()
    }
    fn present(&self, pm: &Pixmap, x: i32, y: i32) {
        let (w, h) = self.dims.get();
        if w <= 0 || h <= 0 {
            return;
        }
        let sc = self.scale.get();
        let x = ((x as f64) * sc).round() as i32;
        let y = ((y as f64) * sc).round() as i32;
        {
            let mut back = self.back.borrow_mut();
            if back.width() != w as u32 || back.height() != h as u32 {
                match Pixmap::new(w as u32, h as u32) {
                    Some(p) => *back = p,
                    None => return,
                }
            }
            let bw = back.width() as i32;
            let data = back.data_mut();
            let src = pm.data();
            let pw = pm.width() as i32;
            let ph = pm.height() as i32;
            for row in 0..ph {
                let dy = y + row;
                if dy < 0 || dy >= h {
                    continue;
                }
                for col in 0..pw {
                    let dx = x + col;
                    if dx < 0 || dx >= w {
                        continue;
                    }
                    let si = ((row * pw + col) * 4) as usize;
                    let di = ((dy * bw + dx) * 4) as usize;
                    data[di..di + 4].copy_from_slice(&src[si..si + 4]);
                }
            }
        }
        self.union_dirty(x, y, pm.width() as i32, pm.height() as i32);
    }
    fn flush(&self) {
        let Some((x0, y0, x1, y1)) = self.dirty.get() else {
            if self.input_dirty.replace(false) {
                self.layer.commit();
            }
            self.forward_click();
            return;
        };
        let mut frames = self.frames.borrow_mut();
        let buffer = match frames.write_frame(&self.back.borrow(), &self.qh) {
            Ok(Some(buffer)) => buffer,
            Ok(None) => return, // Keep damage while waiting for release.
            Err(error) => {
                log::error!("[ERROR] 无法准备 Wayland 画面缓冲：{error}");
                return;
            }
        };
        let surface = self.layer.wl_surface();
        let (lw, lh) = self.logical.get();
        // Geometry and the matching new-size buffer must become visible in the
        // same commit, even if a resize arrived while both old frames were busy.
        if let Some(vp) = self.viewport.borrow().as_ref() {
            vp.set_destination(lw, lh);
            if surface.version() >= 3 {
                surface.set_buffer_scale(1);
            }
        } else if surface.version() >= 3 {
            surface.set_buffer_scale(self.scale.get() as i32);
        }
        if surface.version() >= 4 {
            surface.damage_buffer(x0, y0, x1 - x0, y1 - y0);
        } else {
            surface.damage(0, 0, lw, lh);
        }
        surface.attach(Some(buffer), 0, 0);
        self.layer.commit();
        self.dirty.set(None);
        self.input_dirty.set(false);
        self.forward_click();
    }
    fn set_text_input(&self, on: bool) {
        self.layer.set_keyboard_interactivity(if on {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::None
        });
        self.input_dirty.set(true);
    }

    fn set_input_region(&self, rects: &[IRect], full: bool) {
        if full {
            self.layer.set_input_region(None);
        } else {
            match Region::new(&self.compositor) {
                Ok(region) => {
                    for r in rects {
                        region.add(r.x, r.y, r.w.max(1), r.h.max(1));
                    }
                    self.layer.set_input_region(Some(region.wl_region()));
                }
                Err(_) => {
                    self.layer.set_input_region(None);
                }
            }
        }
        // Commit with the matching frame at the end of this event batch. In
        // particular, avoid committing old-size content after a new configure.
        self.input_dirty.set(true);
    }
    fn fake_key(&self, key: Key) {
        let code = match key {
            Key::Up => 103u16,
            Key::Down => 108u16,
            Key::Escape => 1u16,
        };
        // 优先合成器虚拟键盘（wlr 系）
        if let Some(kb) = self.vk.borrow().clone() {
            let t = (crate::app::now_ms() & 0xffff_ffff) as u32;
            kb.key(t, code as u32, KeyState::Pressed);
            kb.key(t.wrapping_add(1), code as u32, KeyState::Released);
            let _ = self.conn.flush();
            return;
        }
        // 否则用 uinput 内核虚拟键盘（KWin 等）
        if let Some(u) = self.uinput.borrow_mut().as_mut() {
            u.key(code);
            return;
        }
        static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            log::warn!("[WARN] 无可用的按键注入（虚拟键盘/uinput 均不可用）");
        }
    }
    fn start_presentation(&self) -> bool {
        if let Some(kb) = self.vk.borrow().clone() {
            let t = (crate::app::now_ms() & 0xffff_ffff) as u32;
            kb.key(t, 42, KeyState::Pressed);       // KEY_LEFTSHIFT
            kb.key(t + 1, 63, KeyState::Pressed);   // KEY_F5
            kb.key(t + 2, 63, KeyState::Released);
            kb.key(t + 3, 42, KeyState::Released);
            let _ = self.conn.flush();
            return true;
        }
        if let Some(u) = self.uinput.borrow_mut().as_mut() {
            u.start_presentation();
            return true;
        }
        false
    }
    fn can_inject_keys_without_prompt(&self) -> bool {
        self.vk.borrow().is_some() || self.uinput.borrow().is_some()
    }
    fn virtual_right_click(&self, x: i32, y: i32) { self.pending_click.set(Some((x,y))); }
    fn can_forward_right_click(&self) -> bool { self.pointer.borrow().is_some() }
    fn screenshot(&self) -> Option<(Vec<u8>, u32, u32)> {
        None
    }
    fn is_presentation_fullscreen(&self) -> bool {
        false
    }
    fn has_compositor(&self) -> bool {
        true
    }
    fn on_configure(&self, w: i32, h: i32) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.logical.set((w, h));
        self.reconfigure();
    }
    fn set_scale(&self, s: f64) {
        self.scale.set(render_scale(s, self.viewport.borrow().is_some(),
                                    self.layer.wl_surface().version()));
        self.reconfigure();
    }
}

impl wayland_client::Dispatch<wl_buffer::WlBuffer, ()> for WlState {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            for output in state.desktop.outputs.values() {
                let p = output.rt.backend.as_any().downcast_ref::<WlPlat>().unwrap();
                p.frames.borrow_mut().release(buffer);
            }
        }
    }
}

include!("wayland_session.rs");

/// Keep the negotiated connection for startup; fallback must happen before a surface exists.
pub struct PreparedWayland {
    conn: Connection,
    globals: GlobalList,
    event_queue: EventQueue<WlState>,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
}

#[derive(Debug)]
pub enum PrepareError {
    MissingLayerShell,
    Other(String),
}

impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingLayerShell => write!(f, "当前 Wayland 桌面缺少 Sidera 所需的覆盖层协议 zwlr_layer_shell_v1（layer-shell）。可使用 X11/XWayland 兼容模式，或支持该协议的桌面。"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for PrepareError {}

pub fn prepare() -> Result<PreparedWayland, PrepareError> {
    let conn = Connection::connect_to_env()
        .map_err(|e| PrepareError::Other(format!("无法连接 Wayland 桌面，请在图形会话中启动 Sidera：{e}")))?;
    let (globals, event_queue) = registry_queue_init::<WlState>(&conn)
        .map_err(|e| PrepareError::Other(format!("无法读取 Wayland 桌面提供的协议：{e}")))?;
    let qh = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|e| PrepareError::Other(format!("Wayland 桌面缺少所需的 wl_compositor 支持：{e}")))?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|e| match e {
        BindError::NotPresent => PrepareError::MissingLayerShell,
        _ => PrepareError::Other(format!("无法绑定 Wayland 覆盖层协议 zwlr_layer_shell_v1：{e}")),
    })?;
    let shm = Shm::bind(&globals, &qh)
        .map_err(|e| PrepareError::Other(format!("Wayland 桌面缺少所需的 wl_shm 支持：{e}")))?;
    Ok(PreparedWayland { conn, globals, event_queue, compositor, layer_shell, shm })
}
