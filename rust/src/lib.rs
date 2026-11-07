//! Sidera core: platform-neutral state, drawing, UI and event routing.
//!
//! Linux and Windows binaries share this crate. Platform modules that still
//! depend on X11/Wayland/Unix are compiled only on `cfg(unix)`.
// Copyright (C) 2026 Carl_Jin   GNU GPL v3

pub mod app;
pub mod settings_state;
pub mod settings_panel;
pub mod value_input;
pub mod backend;
pub mod bridge_http;
pub mod bridge_ws;
pub mod clear_history;
pub mod clipboard;
pub mod diag_log;
pub mod gesture;
pub mod launch;
pub mod page_cache;
pub mod page_picker;
pub mod paint;
pub mod persistence;
pub mod process_block;
pub mod splash_paint;
pub mod text;
pub mod toolbar;
pub mod ui;
pub mod ui_icons;
pub mod ui_theme;
pub mod wps;

#[cfg(unix)] pub mod desktop;
#[cfg(unix)] pub mod gnome;
#[cfg(unix)] pub mod instance;
#[cfg(unix)] pub mod portal;
#[cfg(unix)] mod settings_window;
#[cfg(unix)] mod settings_wayland;
#[cfg(unix)] pub mod startup;
#[cfg(unix)] pub mod system_theme;
#[cfg(unix)] pub mod touch_evdev;
#[cfg(unix)] pub mod tray;
#[cfg(unix)] pub mod uinput;
#[cfg(unix)] pub mod wayland;
#[cfg(unix)] pub mod x11;

#[cfg(test)] pub mod navigation_tests;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tiny_skia::Pixmap;

use crate::app::{now_ms, App};
use crate::backend::{Backend, IRect, Key};
use crate::gesture::TouchKind;
use crate::text::Fonts;
use crate::ui::{Btn, MoreHit, PopupHit, R, SetHit};
use crate::wps::{WpsBridge, WpsEvent};

static INPUT_GROUP_STATE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

#[cfg(unix)]
fn current_user() -> String {
    if let Ok(u) = std::env::var("USER") {
        if !u.is_empty() {
            return u;
        }
    }
    let uid = unsafe { libc::getuid() };
    if let Ok(p) = std::fs::read_to_string("/etc/passwd") {
        for line in p.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() >= 3 && f[2].parse::<u32>() == Ok(uid) {
                return f[0].to_string();
            }
        }
    }
    String::from("$USER")
}

/// 后台用 pkexec 把当前用户加入 input 组（弹系统授权框输入密码）
#[cfg(unix)]
fn start_input_group_auth(app: &mut App) {
    use std::sync::atomic::Ordering;
    app.input_group_state = 1;
    INPUT_GROUP_STATE.store(1, Ordering::Relaxed);
    let user = current_user();
    std::thread::spawn(move || {
        let res = std::process::Command::new("pkexec")
            .args(["usermod", "-aG", "input", &user])
            .status();
        let state = match res {
            Ok(s) if s.success() => {
                log::info!("[INFO] 已将 {} 加入 input 组，注销重登后生效", user);
                2
            }
            Ok(s) => {
                log::warn!("[WARN] pkexec usermod 失败，退出码 {:?}", s.code());
                3
            }
            Err(e) => {
                log::warn!(
                    "[WARN] 无法运行 pkexec: {}（可手动执行: sudo usermod -aG input {}）",
                    e,
                    user
                );
                3
            }
        };
        INPUT_GROUP_STATE.store(state, Ordering::Relaxed);
    });
}
#[cfg(windows)]
fn start_input_group_auth(app: &mut App) {
    // Windows touch input does not use the Linux `input` group.
    app.input_group_state = 3;
}

pub struct Ip {
    pub dragging: bool,
    pub drag_start_y: i32,
    pub drag_mouse_y0: i32,
    pub moved: bool,
    pub collapse_press: bool,
    // 触摸：正在按下的触点 id，以及每个触点是否起始于 UI（点按钮而非画线）
    pub touch_ids: HashSet<i32>,
    pub touch_over_ui: HashMap<i32, bool>,
    // 正在拖动的设置滑条
    pub slider: Option<ui::SliderId>,
    pub dot_slider: bool,
    dot_value_press_x: i32,
    pub picker_drag: Option<settings_state::PickerHit>,
    // Start y, initial row/page, and whether motion cancelled the pending tap.
    pub panel_scroll: Option<(i32, usize, bool)>,
    // 每个触点的接触尺寸（直径，逻辑像素）：X11 由 raw touch 填，Wayland 由 wl_touch.shape 填
    pub touch_shape: HashMap<i32, f64>,
    // 画笔中断：当前笔画是否刚经过 UI 区域（用于从侧栏出来时断开续画）
    pub stroke_skipped: bool,
    // 最近一次触摸事件时间（用于兜底清理漏掉的 TouchEnd）
    pub last_touch_ms: i64,
    // evdev 触摸是否启用；是否完全忽略 X 合成鼠标；最近触摸位置；被忽略(穿透)的触点
    pub evdev_touch: bool,
    pub ignore_core_pointer: bool,
    pub last_touch_pos: (i32, i32),
    pub ignored_touch: HashSet<i32>,
    pub pending_right_click: Option<(i32, i32)>,
    pub pending_ui: Option<ui::Control>,
    pub ui_touch: Option<i32>,
    pub rejected_touch: HashSet<i32>,
}

impl Default for Ip {
    fn default() -> Self {
        Ip {
            dragging: false,
            drag_start_y: 0,
            drag_mouse_y0: 0,
            moved: false,
            collapse_press: false,
            touch_ids: HashSet::new(),
            touch_over_ui: HashMap::new(),
            slider: None,
            dot_slider: false,
            dot_value_press_x: 0,
            picker_drag: None,
            panel_scroll: None,
            touch_shape: HashMap::new(),
            stroke_skipped: false,
            last_touch_ms: 0,
            evdev_touch: false,
            ignore_core_pointer: false,
            last_touch_pos: (0, 0),
            ignored_touch: HashSet::new(),
            pending_right_click: None,
            pending_ui: None,
            ui_touch: None,
            rejected_touch: HashSet::new(),
        }
    }
}

pub struct Rt {
    pub backend: Box<dyn Backend>,
    pub fonts: Fonts,
    pub icon: Option<Pixmap>,
    pub icon_big: Option<Pixmap>,
    // 重绘合并：事件处理期间只累积脏区，一帧结束时统一渲染一次，避免事件积压
    pub batching: std::cell::Cell<bool>,
    pub dirty: std::cell::RefCell<Vec<R>>,
}

impl Rt {
    /// 真正渲染并上屏一个区域
    fn render(&self, app: &App, r: R) {
        let x = r.x.max(0);
        let y = r.y.max(0);
        let x2 = (r.x + r.w).min(app.screen_w);
        let y2 = (r.y + r.h).min(app.screen_h);
        if x2 <= x || y2 <= y {
            return;
        }
        let icon = if app.launch.active() { self.icon_big.as_ref() } else { self.icon.as_ref() };
        if let Some(pm) = ui::render_region(app, &self.fonts, icon, x, y, x2 - x, y2 - y)
        {
            self.backend.present(&pm, x, y);
        }
    }
    /// 请求重绘一个区域：批处理模式下只登记脏区，否则立即渲染
    pub fn redraw(&self, app: &App, r: R) {
        #[cfg(unix)]
        if let Some(window) = &app.settings_ui.window {
            let full = r.x == 0 && r.y == 0 && r.w >= app.screen_w && r.h >= app.screen_h;
            if full {
                self.redraw_overlay(app, r);
            }
            self.redraw_settings_window(app, window, !full);
            return;
        }
        self.redraw_overlay(app, r);
    }
    #[cfg(unix)]
    fn redraw_settings_window(&self, app: &App, window: &settings_window::Window, sidebar_preview_dirty: bool) {
        window.invalidate();
        if sidebar_preview_dirty {
            for right in [false, true] {
                self.redraw_overlay(app, ui::sidebar_preview_strip(app, right));
            }
        }
        if !self.batching.get() {
            window.paint(self, app);
        }
    }
    fn redraw_preferences(&self, app: &App, sidebar_preview_dirty: bool) {
        #[cfg(unix)]
        if let Some(window) = &app.settings_ui.window {
            self.redraw_settings_window(app, window, sidebar_preview_dirty);
            return;
        }
        let _ = sidebar_preview_dirty;
        self.redraw_full(app);
    }
    fn redraw_overlay(&self, app: &App, r: R) {
        if self.batching.get() {
            let x = r.x.max(0);
            let y = r.y.max(0);
            let x2 = (r.x + r.w).min(app.screen_w);
            let y2 = (r.y + r.h).min(app.screen_h);
            if x2 > x && y2 > y {
                self.dirty.borrow_mut().push(R {
                    x,
                    y,
                    w: x2 - x,
                    h: y2 - y,
                });
            }
        } else {
            self.render(app, r);
        }
    }
    pub fn redraw_full(&self, app: &App) {
        self.redraw(
            app,
            R {
                x: 0,
                y: 0,
                w: app.screen_w,
                h: app.screen_h,
            },
        );
    }
    /// 开启批处理（清空上一帧残留）
    pub fn begin_batch(&self) {
        self.dirty.borrow_mut().clear();
        self.batching.set(true);
    }
    /// 合并相交脏区；左右工具栏分别刷新，避免动画重绘整屏宽度。
    pub fn flush_dirty(&self, app: &App) {
        #[cfg(unix)]
        if let Some(window) = &app.settings_ui.window {
            window.paint(self, app);
        }
        let rects: Vec<R> = std::mem::take(&mut *self.dirty.borrow_mut());
        for r in merge_redraw_regions(rects) { self.render(app, r); }
    }
    /// 结束批处理并渲染
    pub fn end_batch(&self, app: &App) {
        self.batching.set(false);
        self.flush_dirty(app);
    }
    pub fn screen_size(&self) -> (i32, i32) {
        self.backend.screen_size()
    }
    pub fn fake_key(&self, key: Key) {
        self.backend.fake_key(key);
    }
    pub fn virtual_right_click(&self, x: i32, y: i32) {
        self.backend.virtual_right_click(x, y);
    }
    pub fn has_compositor(&self) -> bool {
        self.backend.has_compositor()
    }
    pub fn shape_available(&self) -> bool {
        self.backend.shape_available()
    }
    pub fn is_fullscreen(&self) -> bool {
        self.backend.is_presentation_fullscreen()
    }
    pub fn wps_foreground(&self) -> Option<bool> {
        self.backend.presentation_foreground()
    }
    pub fn set_input_region(&self, rects: &[IRect], full: bool) {
        self.backend.set_input_region(rects, full);
    }

    fn set_text_input(&self, app: &App, active: bool) {
        // Managed windows already receive keys through normal desktop focus.
        if app.settings_ui.window_size().is_none() {
            self.backend.set_text_input(active);
        }
    }
}

fn merge_redraw_regions(rects: Vec<R>) -> Vec<R> {
    let mut regions: Vec<R> = Vec::new();
    for mut r in rects {
        let mut i = 0;
        while i < regions.len() {
            let other = regions[i];
            if r.x <= other.x + other.w && other.x <= r.x + r.w
                && r.y <= other.y + other.h && other.y <= r.y + r.h {
                r = gesture::union(Some(r), regions.swap_remove(i));
                i = 0;
            } else { i += 1; }
        }
        regions.push(r);
        // Bound fragmented drawing damage without penalizing the two sidebars.
        if regions.len() > 32 {
            let all = regions.drain(..).reduce(|a, b| gesture::union(Some(a), b)).unwrap();
            regions.push(all);
        }
    }
    regions
}

fn rect_of(app: &App, r: R, pad: i32) -> Option<IRect> {
    // Clip both ends instead of moving an offscreen rectangle onto the desktop.
    if r.w <= 0 || r.h <= 0 { return None; }
    let x = (r.x - pad).clamp(0, app.screen_w);
    let y = (r.y - pad).clamp(0, app.screen_h);
    let right = (r.x + r.w + pad).clamp(0, app.screen_w);
    let bottom = (r.y + r.h + pad).clamp(0, app.screen_h);
    (right > x && bottom > y).then(|| IRect::new(x, y, right - x, bottom - y))
}

pub fn apply_input_shape(rt: &Rt, app: &App) {
    if app.settings_ui.window_size().is_some() {
        rt.set_input_region(&[], false);
        return;
    }
    if app.launch.active() {
        let mut rects = Vec::new();
        if app.launch.panel_alpha() > 0.0 { rects.extend(rect_of(app, ui::launch_rect(app), 0)); }
        if app.launch.reveal() > 0.0 {
            // Reserve the two short-lived reveal strips, without remapping an
            // XWayland input window on every animation frame. Actions wait.
            for right in [false, true] { rects.extend(rect_of(app, ui::sidebar_rect(app, right), 4)); }
        }
        rt.set_input_region(&rects, false);
        return;
    }
    if app.settings_ui.modal() || app.confirm_quit || app.show_diag || app.show_input_group_prompt {
        rt.set_input_region(&[], true);
        return;
    }
    if app.process_block_open {
        rt.set_input_region(&[], true);
        return;
    }
    if app.settings_open {
        let rects: Vec<_> = rect_of(app, ui::settings_rect(app), 0).into_iter().collect();
        rt.set_input_region(&rects, false);
        return;
    }
    if app.whiteboard {
        rt.set_input_region(&[], true);
        return;
    }
    if dismiss_capture(app) {
        rt.set_input_region(&[], true);
        return;
    }
    if app.mode == 0 {
        let mut rects = Vec::new();
        for right in [false, true] {
            rects.extend(rect_of(app, ui::sidebar_input_rect(app, right), 4));
        }
        if app.pen_popup_visible {
            rects.extend(rect_of(app, ui::pen_popup_rect(app), 4));
        }
        if app.eraser_popup_visible {
            rects.extend(rect_of(app, ui::eraser_popup_rect(app), 4));
        }
        if app.more_menu_open {
            rects.extend(rect_of(app, ui::more_menu_rect(app), 4));
        }
        if app.page_picker.is_some() { rects.extend(rect_of(app, ui::page_picker_rect(app), 4)); }
        rt.set_input_region(&rects, false);
    } else {
        rt.set_input_region(&[], true);
    }
}

pub fn close_all_popups(rt: &Rt, app: &mut App) {
    // Capture the old bounds before hiding the panels. A stroke only redraws
    // its own small area; forgetting these bounds leaves a torn popup behind.
    let regions = [
        app.pen_popup_visible.then(|| ui::pen_popup_rect(app)),
        app.eraser_popup_visible.then(|| ui::eraser_popup_rect(app)),
        app.more_menu_open.then(|| ui::more_menu_rect(app)),
        app.page_picker.as_ref().map(|_| ui::page_picker_rect(app)),
    ];
    app.pen_popup_visible = false;
    app.eraser_popup_visible = false;
    app.more_menu_open = false;
    if app.page_picker.take().is_some() {
        if app.settings_ui.editor.as_ref().is_some_and(|e|e.target==value_input::Target::Page) {
            app.settings_ui.editor=None;
            rt.backend.set_text_input(app.process_input_active);
        }
        for right in [false, true] { rt.redraw(app, ui::sidebar_page_rect(app, right)); }
    }
    app.open_anim = 0;
    if regions.iter().any(Option::is_some) {
        apply_input_shape(rt, app);
        for r in regions.into_iter().flatten() { rt.redraw(app, r); }
    }
}

/// 当前点是否落在任何 UI 元素上（侧栏/弹窗/更多菜单/设置面板）。
/// 画笔/橡皮进入这些区域应中断，避免把墨迹画到侧栏底下（半透明侧栏会透出来）。
pub fn over_ui(app: &App, x: i32, y: i32) -> bool {
    if app.show_input_group_prompt { return true; }
    if app.settings_ui.modal() || app.process_block_open { return true; }
    if app.confirm_quit || app.show_diag { return true; }
    if app.page_picker.is_some() && ui::page_picker_rect(app).contains(x, y) { return true; }
    for right in [false, true] {
        if ui::sidebar_input_rect(app, right).contains(x, y) {
            return true;
        }
    }
    if app.pen_popup_visible && ui::pen_popup_rect(app).contains(x, y) {
        return true;
    }
    if app.eraser_popup_visible && ui::eraser_popup_rect(app).contains(x, y) {
        return true;
    }
    if app.more_menu_open && ui::more_menu_rect(app).contains(x, y) {
        return true;
    }
    if app.settings_open && ui::settings_rect(app).contains(x, y) {
        return true;
    }
    if app.process_block_open && ui::process_block_rect(app).contains(x, y) {
        return true;
    }
    if app.whiteboard && ui::whiteboard_bar_rect(app).contains(x, y) {
        return true;
    }
    false
}

/// 光标模式下有弹窗/菜单打开时，需要整屏接收一次点击来关闭它。
pub fn dismiss_capture(app: &App) -> bool {
    app.mode == 0
        && (app.pen_popup_visible
            || app.eraser_popup_visible
            || app.more_menu_open
            || app.page_picker.is_some())
}

/// 该逻辑坐标是否属于覆盖层“可交互”区域（与 apply_input_shape 一致）。
/// evdev 是全局读取，需要用它模拟输入区域，实现光标模式下的点击穿透。
pub fn point_input_ok(app: &App, x: i32, y: i32) -> bool {
    if app.settings_ui.window_size().is_some() {
        return false;
    }
    if app.settings_ui.modal() || app.confirm_quit || app.show_diag || app.show_input_group_prompt || app.whiteboard {
        return true;
    }
    if app.process_block_open {
        return true;
    }
    if app.settings_open {
        return ui::settings_rect(app).contains(x, y);
    }
    if app.mode != 0 {
        return true;
    }
    dismiss_capture(app) || over_ui(app, x, y)
}

pub fn switch_to_cursor(rt: &Rt, app: &mut App) {
    if app.launch.active() { return; }
    if app.whiteboard { return; }
    close_all_popups(rt, app);
    app.mode = 0;
    if app.is_drawing {
        paint::commit_stroke(app);
    }
    app.is_drawing = false;
    app.reset_palm_gesture();
    app.request_drawing_tools_collapse();
    apply_input_shape(rt, app);
    rt.redraw_full(app);
    log::info!("[INFO] 光标模式");
}

pub fn switch_to_draw(rt: &Rt, app: &mut App, mode: i32) {
    if app.launch.active() { return; }
    close_all_popups(rt, app);
    app.mode = mode;
    app.drawing_tools.set_expanded(true);
    if app.collapsed {
        expand_sidebars(rt, app);
    }
    if app.is_drawing {
        paint::commit_stroke(app);
    }
    app.is_drawing = false;
    app.reset_palm_gesture();
    apply_input_shape(rt, app);
    rt.redraw_full(app);
    log::info!(
        "[INFO] 绘画模式: {}",
        if mode == 1 { "画笔" } else { "橡皮擦" }
    );
}

pub fn collapse_sidebars(rt: &Rt, app: &mut App) {
    if app.collapsed {
        return;
    }
    close_all_popups(rt, app);
    app.drawing_tools.set_expanded(false);
    app.set_sidebar_collapsed(true);
    if app.mode != 0 {
        switch_to_cursor(rt, app);
    }
    apply_input_shape(rt, app);
    rt.redraw_full(app);
    log::info!("[INFO] 侧边栏已收缩");
}

pub fn expand_sidebars(rt: &Rt, app: &mut App) {
    if !app.collapsed {
        return;
    }
    app.set_sidebar_collapsed(false);
    apply_input_shape(rt, app);
    rt.redraw_full(app);
    log::info!("[INFO] 侧边栏已展开");
}

pub fn toggle_collapse(rt: &Rt, app: &mut App) {
    if app.collapsed {
        expand_sidebars(rt, app);
    } else {
        collapse_sidebars(rt, app);
    }
}

#[derive(Debug, PartialEq)]
enum NavigationRoute { Wps, DirectKeys, Unavailable }

fn navigation_route(linked_show: bool, bridge_ready: bool, direct_keys: bool) -> NavigationRoute {
    if linked_show {
        if bridge_ready { NavigationRoute::Wps } else { NavigationRoute::Unavailable }
    } else if direct_keys {
        NavigationRoute::DirectKeys
    } else {
        NavigationRoute::Unavailable
    }
}

// 未接入放映时恢复模拟键盘；已接入的放映只走 API，失败也不重复发键。
// 后端必须提供免弹窗输入：XWayland 使用 uinput，不能退回 XTEST/Portal。
fn external_navigation(rt: &Rt, app: &mut App, wps: Option<&WpsBridge>, cmd: &str, key: Key) -> bool {
    let connected = app.wps_debug && wps.is_some_and(|w| w.connected());
    if app.wps_connected != connected {
        close_all_popups(rt, app);
        app.wps_connected = connected;
        apply_input_shape(rt, app);
        rt.redraw_full(app);
    }
    if connected && rt.wps_foreground() == Some(false) {
        app.wps_navigation_ready = false;
        rt.redraw_full(app);
        return false;
    }
    match navigation_route(app.wps_debug && wps.is_some_and(|w| w.showing()),
                           wps.is_some_and(|w| w.ready()),
                           rt.backend.can_inject_keys_without_prompt()) {
        NavigationRoute::Wps => {
            app.wps_navigation_ready = wps.is_some_and(|w| w.enqueue(cmd));
        }
        NavigationRoute::DirectKeys => {
            app.wps_navigation_ready = true;
            rt.fake_key(key);
            return true;
        }
        NavigationRoute::Unavailable => {
            app.wps_navigation_ready = false;
            crate::wps::wps_log(&format!("[PAGE] {} 未执行：联动未就绪或当前后端需要输入授权", cmd));
        }
    }
    rt.redraw_full(app);
    false
}

pub fn go_prev(rt: &Rt, app: &mut App, wps: Option<&WpsBridge>) {
    if app.whiteboard {
        app.save_current_page();
        if app.current_slide > 1 {
            app.current_slide -= 1;
        }
        app.load_page(app.current_slide);
        rt.redraw_full(app);
        return;
    }
    if !external_navigation(rt, app, wps, "PREV", Key::Up) {
        return;
    }
    app.save_current_page();
    if app.current_slide > 1 {
        app.current_slide -= 1;
    }
    app.load_page(app.current_slide);
    rt.redraw_full(app);
}

fn open_page_picker(rt: &Rt, app: &mut App, right: bool) {
    let Some((page, total, source)) = app.page_picker_context() else { return };
    if matches!(source, page_picker::Source::Wps(_)) && rt.wps_foreground() == Some(false) {
        return;
    }
    close_all_popups(rt, app);
    app.popup_on_right = right;
    app.page_picker = Some(page_picker::PagePicker::new(page, total, source));
    app.open_anim = now_ms();
    apply_input_shape(rt, app);
    rt.redraw(app, ui::page_picker_rect(app));
    rt.redraw(app, ui::sidebar_page_rect(app, right));
}

fn commit_page_picker(rt: &Rt, app: &mut App, wps: Option<&WpsBridge>) {
    let Some(picker) = &app.page_picker else { return };
    let (page, total, source) = (picker.selected(), picker.total, picker.source);
    let context = app.page_picker_context();
    close_all_popups(rt, app);
    let Some((current, live_total, live_source)) = context else { return };
    if total != live_total || source != live_source || page == current { return; }
    match source {
        page_picker::Source::Whiteboard => {
            app.save_current_page();
            app.current_slide = page;
            app.load_page(page);
            rt.redraw_full(app);
        }
        page_picker::Source::Wps(epoch) => {
            // A single absolute command; real WPS state moves the ink cache.
            // Never approximate a jump with repeated next/previous keystrokes.
            if rt.wps_foreground() != Some(false) {
                if let Some(bridge) = wps { bridge.enqueue_jump(page, epoch); }
            }
        }
    }
}

pub fn on_scroll(rt: &Rt, app: &mut App, ip: &Ip, x: i32, y: i32, pages: f64) {
    if app.show_input_group_prompt || app.settings_ui.modal() || app.confirm_quit || ip.pending_ui.is_some()
        || ip.slider.is_some() || ip.dragging || ip.panel_scroll.is_some() || ip.picker_drag.is_some() { return; }
    if app.show_diag {
        if pages.is_finite() && pages != 0.0 && ui::diag_rect(app).contains(x,y) {
            handle_settings_hit(rt, app, SetHit::InspectorPage(if pages>0.0 {1} else {-1}));
        }
        return;
    }
    if app.process_block_open {
        if pages.is_finite() && pages != 0.0 && ui::process_block_rect(app).contains(x, y) {
            let step = if pages > 0.0 { 1 } else { -1 };
            let rows = ui::process_rows(app).len();
            let visible = ui::process_visible_rows(app);
            let max = rows.saturating_sub(visible);
            app.process_scroll = (app.process_scroll as i64 + step as i64).clamp(0, max as i64) as usize;
            rt.redraw(app, ui::process_block_rect(app));
        }
        return;
    }
    if app.settings_open {
        if pages.is_finite() && pages != 0.0 && ui::settings_rect(app).contains(x, y)
            && ui::settings_change_page(app, if pages > 0.0 { 1 } else { -1 }) {
            rt.redraw(app, ui::settings_rect(app));
        }
        return;
    }
    if app.page_picker.is_none() || !ui::page_picker_control_rect(app, ui::PagePickerHit::Wheel).contains(x, y) { return; }
    if let Some(picker) = app.page_picker.as_mut() { picker.scroll(pages, Instant::now()); }
    rt.redraw(app, ui::page_picker_rect(app));
}

pub fn go_next(rt: &Rt, app: &mut App, wps: Option<&WpsBridge>) {
    if app.whiteboard {
        app.save_current_page();
        let mut p = app.current_slide + 1;
        if p > app::CACHE_BOARD {
            p = 1;
            app.wb_limit_msg_until = now_ms() + 3000;
        }
        app.current_slide = p;
        app.load_page(app.current_slide);
        rt.redraw_full(app);
        return;
    }
    if !external_navigation(rt, app, wps, "NEXT", Key::Down) {
        return;
    }
    app.save_current_page();
    app.current_slide += 1;
    app.load_page(app.current_slide);
    rt.redraw_full(app);
}

fn presentation_action_for(
    rt: &Rt,
    app: &App,
    wps: Option<&WpsBridge>,
) -> ui::PresentationAction {
    let visible = rt.wps_foreground() != Some(false);
    let showing = app.wps_debug && wps.is_some_and(WpsBridge::showing);
    if showing && visible {
        return ui::PresentationAction::End;
    }
    let start_ready = app.wps_debug
        && visible
        && if app.wps_test_mode {
            wps.is_some_and(WpsBridge::start_available)
        } else {
            wps.is_some_and(WpsBridge::connected)
        };
    if start_ready {
        ui::PresentationAction::Start
    } else {
        ui::PresentationAction::Escape
    }
}

pub fn exit_presentation(rt: &Rt, app: &mut App, wps: Option<&WpsBridge>) {
    if app.whiteboard {
        app.whiteboard_bg_index = (app.whiteboard_bg_index + 1) % app::WHITEBOARD_COLORS.len() as i32;
        rt.redraw_full(app);
        return;
    }
    if app.mode != 0 {
        switch_to_cursor(rt, app);
    }
    // Recompute instead of trusting the last tick: the WPS window may have
    // dropped behind another window since the label was painted. Esc must never
    // turn into a presentation-start shortcut.
    match presentation_action_for(rt, app, wps) {
        ui::PresentationAction::Start => {
            if app.wps_debug && !app.wps_test_mode {
                // Default mode deliberately avoids the WPS command queue for entering
                // or leaving a show. Keyboard shortcuts still work when the add-in is
                // slow or its API is blocked.
                if now_ms() >= app.wps_start_sent_until {
                    if !rt.backend.start_presentation() {
                        crate::diag_log::warn("WPS 默认模式无法注入 Shift+F5，开始放映失败");
                    } else {
                        app.wps_start_sent_until = now_ms() + 2500;
                        crate::diag_log::info("WPS 默认模式已发送 Shift+F5 开始放映");
                    }
                }
            } else if app.wps_debug && wps.is_some_and(WpsBridge::start_available) {
                // START has its own guard against duplicate taps; never send Esc here.
                if let Some(w) = wps { w.enqueue("START"); }
            }
            rt.redraw_full(app);
        }
        ui::PresentationAction::End => {
            if app.wps_debug && !app.wps_test_mode {
                if wps.is_some_and(WpsBridge::showing) {
                    rt.fake_key(Key::Escape);
                }
            } else {
                external_navigation(rt, app, wps, "EXIT", Key::Escape);
            }
            rt.redraw_full(app);
        }
        ui::PresentationAction::Escape => {
            // No WPS show is active and the office window is not foreground.
            // Keep the direct-key fallback for truly disconnected desktops,
            // but never start a presentation from the Esc control.
            let linked = app.wps_debug && wps.is_some_and(WpsBridge::connected);
            if !linked {
                external_navigation(rt, app, wps, "EXIT", Key::Escape);
            }
            rt.redraw_full(app);
        }
    }
}

pub fn do_screenshot(rt: &Rt) {
    if rt.backend.request_screenshot() { return; }
    match rt.backend.screenshot() {
        Some((rgba, w, h)) => {
            save_and_clipboard(rgba, w, h);
        }
        None => {
            #[cfg(unix)]
            {
                // Wayland（无 screencopy）走 xdg-desktop-portal，异步进行
                log::info!("[INFO] 已提交门户截图任务");
                portal::screenshot_nonblocking();
            }
        }
    }
}

/// 保存 PNG 并复制到剪贴板
pub fn save_and_clipboard(rgba: Vec<u8>, w: u32, h: u32) {
    let Some(img) = image::RgbaImage::from_raw(w, h, rgba) else {
        return;
    };
    let dir = dirs::picture_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&dir);
    let fn_ = dir.join(format!("sidera_{}.png", now_ms()));
    if img.save(&fn_).is_ok() {
        log::info!("[INFO] 截图已保存: {}", fn_.display());
    } else {
        log::warn!("[WARN] 截图保存失败: {}", fn_.display());
    }
    clipboard::copy_image(img.into_raw(), w, h);
}

pub fn collect_system_info(rt: &Rt, app: &App) -> String {
    let mut s = String::new();
    s.push_str("=== 系统环境诊断 ===\n");
    if let Ok(os) = std::fs::read_to_string("/etc/os-release") {
        for line in os.lines() {
            if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
                s.push_str(&format!("系统: {}\n", v));
            }
        }
    }
    s.push_str(&format!("架构: {}\n", std::env::consts::ARCH));
    s.push_str(&format!(
        "XDG_SESSION_TYPE: {}\n",
        std::env::var("XDG_SESSION_TYPE").unwrap_or_default()
    ));
    s.push_str(&format!(
        "DISPLAY: {}\n",
        std::env::var("DISPLAY").unwrap_or_default()
    ));
    s.push_str(&format!(
        "合成器: {}\n",
        if rt.has_compositor() { "有" } else { "无" }
    ));
    s.push_str(&format!("显示后端: {}\n", rt.backend.description()));
    s.push_str(&format!("界面主题: {}（当前{}）\n", app.theme.preference.label(),
        if app.theme.dark() { "深色" } else { "浅色" }));
    s.push_str(&format!("直接翻页按键: {}\n右键转发: {}\nCtrl+Shift+D: {}\n",
        rt.backend.can_inject_keys_without_prompt(), rt.backend.can_forward_right_click(), app.hotkey_ok));
    s.push_str(&format!(
        "点击区域控制: {}\n",
        if rt.shape_available() {
            "扩展可用"
        } else {
            "扩展不可用!"
        }
    ));
    s.push_str(&format!(
        "屏幕: {}x{}\n",
        rt.screen_size().0, rt.screen_size().1
    ));
    s.push_str(&format!(
        "模式: {}\n",
        match app.mode {
            0 => "光标",
            1 => "画笔",
            _ => "橡皮",
        }
    ));
    s.push_str(&format!("WPS 全屏: {}\n", app.wps_fullscreen));
    s.push_str(&format!("WPS 已连接: {}\n", app.wps_connected));
    s.push_str(&format!("WPS 模式: {}\n", if app.wps_test_mode { "测试（页码/跳页/开始）" } else { "默认（API 翻页 + 快捷键）" }));
    if let Some(path) = app::config_path() {
        s.push_str(&format!("设置文件: {}\n", path.display()));
    }
    s.push_str(&format!(
        "主题: {} / {}（{}）\n启动延迟: {} 秒\n工具栏自动展开: {} 分钟\n",
        app.theme.preference.label(),
        app.theme.accent.label(),
        app.theme.accent.key(),
        app.startup_delay,
        app.sidebar_expand_minutes
    ));
    if let Some(path) = diag_log::path() {
        s.push_str(&format!("诊断日志: {}\n", path.display()));
    }

    s
}

pub fn set_process_keyboard(rt: &Rt, app: &mut App, on: bool) {
    app.process_input_active = on;
    rt.set_text_input(app, on);
}

fn save_process_rules(app: &App) {
    if let Err(error) = process_block::save_rules(&app.process_rules) {
        diag_log::warn(&format!("保存进程屏蔽规则失败：{error}"));
    }
}

pub fn add_process_input(app: &mut App) {
    let value = app.process_input.trim().to_string();
    if value.is_empty() || value.chars().count() > 256 {
        return;
    }
    if !app.process_rules.iter().any(|rule| rule == &value) {
        if app.process_rules.len()>=64 {return;}
        app.process_rules.push(value.clone());
        if app.process_rules.len() > 64 {
            app.process_rules.truncate(64);
        }
        save_process_rules(app);
    }
    app.process_input.clear();
    app.settings_ui.process_view=settings_state::ProcessView::Rules;
    app.process_scroll=app.process_rules.iter().position(|rule|rule==&value).unwrap_or(0)
        .saturating_sub(ui::process_visible_rows(app).saturating_sub(1));
}

fn handle_process_hit(rt: &Rt, app: &mut App, ip: &mut Ip, x: i32, y: i32) {
    let Some(hit) = ui::process_block_hit(app, x, y) else {
        set_process_keyboard(rt, app, false);
        return;
    };
    match hit {
        ui::ProcessHit::View(view)=>{
            set_process_keyboard(rt,app,false);
            app.settings_ui.process_view=view;
            app.process_scroll=0;
        }
        ui::ProcessHit::Enable => {
            app.settings_ui.begin_toggle(ui::Control::Process(ui::ProcessHit::Enable), app.process_block_enabled);
            app.process_block_enabled = !app.process_block_enabled;
            app::save_settings(app);
            process_block::set_daemon_enabled(app.process_block_enabled);
        }
        ui::ProcessHit::Input => set_process_keyboard(rt, app, true),
        ui::ProcessHit::Add => add_process_input(app),
        ui::ProcessHit::Paste => {
            if let Some(text) = clipboard::get_text() {
                app.process_input = text.chars().take(256).collect();
            }
        }
        ui::ProcessHit::Scan => {
            app.process_scan = process_block::scan();
            app.process_scroll = 0;
        }
        ui::ProcessHit::ScrollUp => app.process_scroll = app.process_scroll.saturating_sub(ui::process_visible_rows(app)),
        ui::ProcessHit::ScrollDown => {
            let max = ui::process_rows(app)
                .len()
                .saturating_sub(ui::process_visible_rows(app));
            app.process_scroll = (app.process_scroll + ui::process_visible_rows(app)).min(max);
        }
        ui::ProcessHit::Rule(index) => {
            if index < app.process_rules.len() {
                app.process_rules.remove(index);
                save_process_rules(app);
            }
        }
        ui::ProcessHit::Proc(index) => {
            if let Some(proc) = app.process_scan.get(index).cloned() {
                if let Some(pos) = app
                    .process_rules
                    .iter()
                    .position(|rule| process_block::matches(rule, &proc))
                {
                    app.process_rules.remove(pos);
                } else if app.process_rules.len() < 64 {
                    app.process_rules.push(proc.name);
                }
                save_process_rules(app);
            }
        }
        ui::ProcessHit::Done => {
            set_process_keyboard(rt, app, false);
            app.process_block_open = false;
            app.settings_open = true;
            app.settings_ui.opened_at = now_ms();
        }
    }
    ip.pending_ui = None;
    app.process_scroll = app.process_scroll.min(ui::process_rows(app).len().saturating_sub(ui::process_visible_rows(app)));
    apply_input_shape(rt, app);
    rt.redraw_full(app);
}

pub fn open_settings(rt: &Rt, app: &mut App) {
    #[cfg(unix)]
    if let Some(window) = &app.settings_ui.window {
        window.raise();
        return;
    }
    if std::env::var("SIDERA_TRACE").is_ok() { log::info!("[SETTINGS] 打开"); }
    app.settings_open = true;
    app.settings_page = 0;
    app.settings_ui.addins_unlocked = false;
    if app.settings_ui.tab == settings_state::Tab::Addins { app.settings_ui.select(settings_state::Tab::Appearance); }
    app.settings_ui.opened_at = now_ms();
    app.settings_ui.closing_at = 0;
    app.settings_ui.overlay_closing_at = 0;
    app.settings_ui.dialog = None;
    app.settings_ui.picker = None;
    app.settings_ui.editor = None;
    app.settings_ui.autostart = ui::is_autostart();
    rt.backend.set_text_input(false);
    app.process_block_open = false;
    app.process_input_active = false;
    app.open_anim = now_ms();
    close_all_popups(rt, app);
    #[cfg(unix)]
    if !app.launch.active() && !app.show_input_group_prompt {
        match settings_window::Window::open(rt, app) {
            Ok(window) => {
                app.settings_ui.window = Some(window);
                // The annotation canvas stays intact while the preferences
                // window is open. Clear only its presentation surface.
                rt.render(app, R { x: 0, y: 0, w: app.screen_w, h: app.screen_h });
            }
            Err(error) => log::info!("独立设置窗口不可用，使用原有设置面板：{error}"),
        }
    }
    apply_input_shape(rt, app);
    rt.redraw_full(app);
}

fn apply_slider(app: &mut App, id: ui::SliderId, v: f32) {
    match id {
        ui::SliderId::Position => {
            let height = app.sidebar_position_height();
            app.sb_y=((app.screen_h-height).max(0) as f32*v.clamp(0.0,100.0)/100.0).round() as i32;
        }
        ui::SliderId::AutoExpand => { app.sidebar_expand_minutes = v.round().clamp(1.0,20.0) as u32; }
        ui::SliderId::Alpha => {
            app.sidebar_alpha = v.round().clamp(30.0, 255.0) as u8;
        }
        ui::SliderId::Scale => {
            // Keep exact percent steps stable across pixel rounding and saved settings.
            app.sb_scale = (f64::from(v) * 100.0).round().clamp(60.0, 140.0) / 100.0;
        }
        ui::SliderId::StartupDelay => {
            app.startup_delay = (v.round() as i32).clamp(
                app::MIN_STARTUP_DELAY as i32,
                app::MAX_STARTUP_DELAY as i32,
            ) as u32;
        }
        ui::SliderId::Threshold => {
            app.large_touch_threshold = (v as f64).clamp(20.0, 260.0);
        }
        ui::SliderId::EraseScale => {
            app.large_erase_scale10 = (v * 10.0).round().clamp(8.0, 25.0) as i32;
        }
    }
}

pub fn open_value_input(rt:&Rt,app:&mut App,target:value_input::Target) {
    app.settings_ui.editor=Some(value_input::Editor::new(app,target));
    app.press=None;
    rt.set_text_input(app, true);
    apply_input_shape(rt,app);
    rt.redraw_full(app);
}

pub fn value_input_key(rt:&Rt,app:&mut App,key:value_input::Key)->bool {
    use value_input::{Key,Target};
    let Some(target)=app.settings_ui.editor.as_ref().map(|editor|editor.target) else {return false};
    if key==Key::Escape || (key==Key::Enter && app.settings_ui.editor.as_ref().is_some_and(|e|e.text==e.initial)) {
        app.settings_ui.editor=None;
        rt.set_text_input(app, app.process_input_active);
        apply_input_shape(rt,app);
    } else if key==Key::Enter {
        match value_input::parse(app) {
            Err(message)=>{if let Some(editor)=app.settings_ui.editor.as_mut(){editor.error=message;}},
            Ok(value)=>{
                match target {
                    Target::Setting(id)=>{
                        let raw=match id {ui::SliderId::Alpha=>value/100.0*255.0,ui::SliderId::Scale=>value/100.0,_=>value};
                        apply_slider(app,id,raw);app::save_settings(app);
                    }
                    Target::DotSize=>{app.whiteboard_dot_size=value as i32;app::save_settings(app);}
                    Target::Page=>{if let Some(picker)=app.page_picker.as_mut(){picker.set_selected(value as i32);}}
                    Target::Hex=>{app.settings_ui.picker=Some(settings_state::ColorDraft::new(value as u32));}
                    Target::Hue|Target::Saturation|Target::Brightness=>{
                        if let Some(picker)=app.settings_ui.picker.as_mut(){match target {
                            Target::Hue=>picker.hue=(value/360.0).rem_euclid(1.0),
                            Target::Saturation=>picker.saturation=value/100.0,
                            _=>picker.value=value/100.0,
                        }}
                    }
                }
                app.settings_ui.editor=None;
                rt.set_text_input(app, app.process_input_active);
                apply_input_shape(rt,app);
            }
        }
    } else if key==Key::Paste {
        if let Some(text)=clipboard::get_text(){if let Some(editor)=app.settings_ui.editor.as_mut(){editor.insert(text.trim());}}
    } else if let Some(editor)=app.settings_ui.editor.as_mut(){editor.edit(key);}
    rt.redraw_full(app);true
}

pub fn text_input_key(rt:&Rt,app:&mut App,key:value_input::Key)->bool {
    if value_input_key(rt,app,key) {return true;}
    if !app.process_block_open || !app.process_input_active {return false;}
    use value_input::Key;
    match key {
        Key::Escape=>set_process_keyboard(rt,app,false),
        Key::Enter=>add_process_input(app),
        Key::Backspace=>{app.process_input.pop();},
        Key::Paste=>{if let Some(text)=clipboard::get_text(){app.process_input=text.chars().take(256).collect();}},
        Key::Character(ch) if !ch.is_control() && app.process_input.chars().count()<256=>app.process_input.push(ch),
        _=>{},
    }
    rt.redraw_full(app);true
}

fn handle_value_hit(rt:&Rt,app:&mut App,hit:value_input::Hit) {
    use value_input::{Hit,Key};
    match hit {
        Hit::Field=>{value_input_key(rt,app,Key::SelectAll);},
    }
}

fn save_theme_selection(app: &mut App, next: ui_theme::Theme) -> bool {
    let previous = app.theme;
    let recent = app.settings_ui.recent.clone();
    let rgb = next.accent.rgb();
    app.settings_ui.recent.retain(|color| *color != rgb);
    app.settings_ui.recent.insert(0, rgb);
    app.settings_ui.recent.truncate(8);
    let changed = app.apply_theme(next);
    match settings_state::save_appearance(app) {
        Ok(()) => {
            app::save_settings(app);
            // The synchronous atomic writes must not consume the first frames.
            if changed {
                if let Some((_, start)) = app.settings_ui.theme_from.as_mut() {
                    *start = now_ms();
                }
            }
            app.settings_ui.notify("外观已保存");
            true
        }
        Err(error) => {
            app.apply_theme(previous);
            app.settings_ui.recent = recent;
            app.settings_ui.notify(format!("未保存：{error}"));
            diag_log::warn(&format!("保存外观失败：{error}"));
            false
        }
    }
}

fn show_inspector(app: &mut App, kind: settings_state::Inspector, text: String) {
    app.diag_text = text;
    app.show_diag = true;
    app.settings_ui.inspector = kind;
    app.settings_ui.inspector_page = 0;
    app.settings_ui.overlay_at = now_ms();
}

fn open_project_link(app: &mut App, url: &'static str) {
    #[cfg(unix)]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(windows)]
    let result = std::process::Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(not(any(unix, windows)))]
    let result: std::io::Result<std::process::Child> = Err(std::io::Error::other("当前系统无法打开链接"));
    match result {
        Ok(mut child) => { std::thread::spawn(move || { let _ = child.wait(); }); }
        Err(error) => app.settings_ui.notify(format!("打开链接失败：{error}")),
    }
}

pub fn handle_settings_hit(rt: &Rt, app: &mut App, hit: SetHit) {
    use settings_state::{Dialog, Inspector, Tab};
    match hit {
        SetHit::EditSlider(id) => open_value_input(rt,app,value_input::Target::Setting(id)),
        SetHit::Tab(tab) => {
            if tab == Tab::Addins && !app.settings_ui.addins_unlocked {
                app.settings_ui.show_dialog(Dialog::Addins);
            } else {
                app.settings_ui.select(tab);
                app.settings_page = 0;
            }
        }
        SetHit::Theme(preference) => { save_theme_selection(app, ui_theme::Theme { preference, ..app.theme }); }
        SetHit::ResetSlider(id) => {
            apply_slider(app, id, id.default_value());
            app::save_settings(app);
            app.settings_ui.notify("已恢复此项默认值");
        }
        SetHit::Autostart => {
            let old = ui::is_autostart();
            app.settings_ui.begin_toggle(ui::Control::Setting(hit), old);
            ui::set_autostart(!old);
            app.settings_ui.autostart = ui::is_autostart();
            if app.settings_ui.autostart == old { app.settings_ui.notify("未能修改自动启动设置"); }
        }
        // Kept for source compatibility; there is no off state in the settings UI.
        SetHit::WpsDebug => { app.wps_debug = true; app::save_settings(app); }
        SetHit::WpsRemove => app.settings_ui.show_dialog(Dialog::Remove),
        SetHit::WpsRegister => app.settings_ui.show_dialog(Dialog::Register),
        SetHit::ProcessBlock => {
            app.settings_open = false;
            app.process_block_open = true;
            app.process_rules = process_block::load_rules();
            app.process_scan = process_block::scan();
            app.process_input.clear();
            app.settings_ui.process_view=if app.process_rules.is_empty() {settings_state::ProcessView::Running}else{settings_state::ProcessView::Rules};
            app.process_input_active = false;
            app.process_scroll = 0;
            app.settings_ui.overlay_at = now_ms();
        }
        SetHit::ThemeAccent => {
            app.settings_ui.picker = Some(settings_state::ColorDraft::new(app.theme.accent.rgb()));
            app.settings_ui.overlay_at = now_ms();
            app.settings_ui.overlay_closing_at = 0;
        }
        SetHit::RightClick => {
            app.settings_ui.begin_toggle(ui::Control::Setting(hit), app.right_click_cursor_on);
            app.right_click_cursor_on = !app.right_click_cursor_on;
            app::save_settings(app);
        }
        SetHit::ClearLog => app.settings_ui.show_dialog(Dialog::ClearLogs),
        SetHit::Logs => {
            let lines = diag_log::tail(400);
            show_inspector(app, Inspector::Logs, if lines.is_empty() { "暂无运行日志。".into() } else { lines.join("\n") });
        }
        SetHit::SysInfo => { let info = collect_system_info(rt, app); show_inspector(app, Inspector::Diagnostics, info); }
        SetHit::Repository => open_project_link(app, settings_state::REPOSITORY),
        SetHit::License => open_project_link(app, settings_state::LICENSE_URL),
        SetHit::InspectorPage(delta) => {
            let pages = settings_panel::inspector_lines(app, &rt.fonts).len().div_ceil(settings_panel::inspector_page_size(app)).max(1);
            let page=app.settings_ui.inspector_page.min(pages-1);
            app.settings_ui.inspector_page = (page as i64 + delta as i64).clamp(0, pages as i64 - 1) as usize;
        }
        SetHit::InspectorClose => { app.show_diag = false; }
        SetHit::Quit => { app.confirm_quit = true; app.settings_ui.overlay_at = now_ms(); }
        SetHit::Done => { app.settings_ui.closing_at = now_ms(); }
        SetHit::SettingsPage(delta) => { ui::settings_change_page(app, delta); }
    }
    apply_input_shape(rt, app);
    let sidebar_preview_dirty = matches!(hit, SetHit::Tab(_) | SetHit::Theme(_) | SetHit::ResetSlider(_));
    rt.redraw_preferences(app, sidebar_preview_dirty);
}

fn handle_dialog(rt: &Rt, app: &mut App, hit: settings_state::DialogHit) {
    use settings_state::{Dialog, DialogHit, Inspector, Tab};
    if hit == DialogHit::Cancel {
        app.settings_ui.overlay_closing_at = now_ms();
    } else if let Some(dialog) = app.settings_ui.dialog.take() {
        match dialog {
            Dialog::Addins => {
                app.settings_ui.addins_unlocked = true;
                app.settings_ui.select(Tab::Addins);
                app.settings_page = 0;
            }
            Dialog::Register | Dialog::Remove => {
                let result = if dialog == Dialog::Register { wps::register_addin() } else { wps::remove_addin_registration() };
                let text = match result {
                    Ok(detail) => {
                        diag_log::info(&detail);
                        format!("{}已完成。\n请重新打开 WPS 使改动生效。\n\n{detail}", if dialog == Dialog::Register { "加载项登记" } else { "加载项清除" })
                    }
                    Err(error) => { diag_log::error(&error.to_string()); format!("操作未完成：\n{error}\n\n可在“关于 → 运行日志”查看记录。") }
                };
                show_inspector(app, Inspector::Notice, text);
            }
            Dialog::ClearLogs => {
                let _ = std::fs::remove_file(wps::log_file());
                diag_log::clear();
                show_inspector(app, Inspector::Logs, "日志已清空。".into());
            }
        }
    }
    apply_input_shape(rt, app);
    rt.redraw_preferences(app, true);
}

fn handle_picker(rt: &Rt, app: &mut App, ip: &mut Ip, hit: settings_state::PickerHit, x: i32, y: i32) {
    use settings_state::{ColorDraft, PickerHit};
    match hit {
        PickerHit::Edit(target) => open_value_input(rt,app,target),
        PickerHit::Field | PickerHit::Hue => { settings_panel::picker_drag(app, hit, x, y); ip.picker_drag = Some(hit); }
        PickerHit::Preset(index) => {
            if let Some(accent) = settings_panel::PRESETS.get(index) { app.settings_ui.picker = Some(ColorDraft::new(accent.rgb())); }
        }
        PickerHit::Recent(index) => {
            if let Some(rgb) = app.settings_ui.recent.get(index) { app.settings_ui.picker = Some(ColorDraft::new(*rgb)); }
        }
        PickerHit::Cancel => app.settings_ui.overlay_closing_at = now_ms(),
        PickerHit::Apply => {
            if let Some(rgb) = app.settings_ui.picker.as_ref().map(ColorDraft::rgb) {
                save_theme_selection(app, ui_theme::Theme { accent: ui_theme::Accent::Custom(rgb), ..app.theme });
                app.settings_ui.overlay_closing_at = now_ms();
            }
        }
    }
    rt.redraw_preferences(app, hit == PickerHit::Apply && app.previewing_sidebars());
}

fn first_hit(r: R, x: i32, y: i32) -> bool {
    r.contains(x, y)
}

fn set_sidebar_drag_feedback(rt: &Rt, app: &mut App, feedback: Option<ui::SidebarDragFeedback>) {
    if app.sidebar_drag.is_none() && feedback.is_none() { return; }
    app.sidebar_drag = feedback;
    for right in [false, true] { rt.redraw(app, ui::sidebar_rect(app, right)); }
}

pub fn on_press(
    rt: &Rt, app: &mut App, ip: &mut Ip, wps: Option<&WpsBridge>,
    x: i32, y: i32, button: u8,
) {
    if app.launch.active() { return; }
    if app.settings_ui.closing_at != 0 || app.settings_ui.overlay_closing_at != 0 { return; }
    if button == 4 || button == 5 {
        on_scroll(rt, app, ip, x, y, if button == 4 { -1.0 } else { 1.0 });
        return;
    }
    if app.page_picker.as_ref().is_some_and(|p| p.dragging()) { return; }
    // The press that started a drag owns it until release. A second button
    // must not replace it with a pending click and consume that release.
    if ip.pending_ui.is_some() || ip.slider.is_some() || ip.dragging || ip.picker_drag.is_some() || ip.panel_scroll.is_some() { return; }
    if button==1 && !app.show_input_group_prompt && app.settings_ui.editor.is_some()
        && !value_input::controls(app).iter().any(|(_,r)|r.contains(x,y)) {
        value_input_key(rt,app,value_input::Key::Enter);
        if app.settings_ui.editor.is_some() || ui::control_at(app,x,y).is_none() {return;}
    }
    if button == 1 && !app.show_input_group_prompt && !app.settings_ui.modal() && !app.confirm_quit {
        if app.process_block_open && settings_panel::process_list_rect(app).contains(x,y) {
            ip.panel_scroll = Some((y, app.process_scroll, false));
        } else if app.show_diag {
            if settings_panel::inspector_body_rect(app).contains(x,y) {
                ip.panel_scroll=Some((y,app.settings_ui.inspector_page,false));
            }
        } else if app.settings_open && settings_panel::content_rect(app).contains(x,y) && ui::settings_slider_hit(app,x,y).is_none() {
            ip.panel_scroll=Some((y,app.settings_page,false));
        }
    }
    if !app.settings_open && !app.confirm_quit && !app.show_diag
        && ui::sidebar_animating(app) && [false, true].into_iter()
        .any(|right| ui::sidebar_input_rect(app, right).contains(x, y)) { return; }
    if let Some(control) = ui::control_at(app, x, y) {
        if button != 1 || !ui::control_enabled(app, control) { return; }
        if let ui::Control::Setting(SetHit::InspectorPage(delta))=control {
            if !settings_panel::inspector_can_page(app,&rt.fonts,delta) {return;}
        }
        // The entire tucked tab shares its drag gesture; a stationary release expands it.
        if app.collapsed && matches!(control, ui::Control::Sidebar(Btn::Collapse, _)) {
            activate_press(rt, app, ip, wps, x, y, button);
            return;
        }
        if control == ui::Control::PagePicker(ui::PagePickerHit::Wheel) {
            app.press = None;
            if let Some(picker) = app.page_picker.as_mut() { picker.begin(y, Instant::now()); }
            rt.redraw(app, ui::page_picker_rect(app));
            return;
        }
        if let Some(old) = app.press.take() {
            if let Some(r) = ui::control_rect(app, old.control) { rt.redraw(app, r); }
        }
        app.press = Some(ui::PressFeedback::new(control));
        if let Some(r) = ui::control_rect(app, control) { rt.redraw(app, r); }
        if !matches!(control, ui::Control::Slider(_) | ui::Control::Whiteboard(ui::WhiteboardControl::DotSize)
            | ui::Control::Picker(settings_state::PickerHit::Field | settings_state::PickerHit::Hue)) {
            if control == ui::Control::Whiteboard(ui::WhiteboardControl::DotValue) {
                ip.dot_value_press_x = x;
            }
            ip.pending_ui = Some(control);
            return;
        }
    }
    activate_press(rt, app, ip, wps, x, y, button);
}

// Discrete controls activate on release; drawing and dragging start immediately.
fn activate_press(
    rt: &Rt,
    app: &mut App,
    ip: &mut Ip,
    wps: Option<&WpsBridge>,
    x: i32,
    y: i32,
    button: u8,
) {
    if !app.show_input_group_prompt && app.settings_ui.editor.is_some() {
        if let Some(ui::Control::Editor(hit))=ui::control_at(app,x,y){handle_value_hit(rt,app,hit);}
        return;
    }
    if !app.show_input_group_prompt && app.settings_ui.dialog.is_some() {
        if let Some(ui::Control::Dialog(hit)) = ui::control_at(app,x,y) { handle_dialog(rt,app,hit); }
        return;
    }
    if !app.show_input_group_prompt && app.settings_ui.picker.is_some() {
        if let Some(ui::Control::Picker(hit)) = ui::control_at(app,x,y) { handle_picker(rt,app,ip,hit,x,y); }
        return;
    }
    if app.show_input_group_prompt {
        if let Some(h) = ui::input_group_prompt_hit(app, x, y) {
            match h {
                ui::PromptHit::Authorize => {
                    if app.input_group_state == 0 {
                        start_input_group_auth(app);
                    }
                }
                ui::PromptHit::Close => {
                    app.show_input_group_prompt = false;
                    apply_input_shape(rt, app);
                }
            }
            rt.redraw_full(app);
        }
        return;
    }
    if app.confirm_quit {
        if let Some(h) = ui::confirm_hit(app, x, y) {
            match h {
                ui::ConfirmHit::Yes => {
                    process_block::stop_daemon();
                    std::process::exit(0);
                }
                ui::ConfirmHit::No => {
                    app.confirm_quit = false;
                    apply_input_shape(rt, app);
                    rt.redraw_full(app);
                }
            }
        }
        return;
    }
    if app.show_diag {
        if let Some(ui::Control::Setting(hit)) = ui::control_at(app,x,y) { handle_settings_hit(rt,app,hit); }
        return;
    }
    log::debug!(
        "[INPUT] press ({},{}) btn={} mode={} screen={}x{}",
        x, y, button, app.mode, app.screen_w, app.screen_h
    );
    if app.process_block_open {
        handle_process_hit(rt, app, ip, x, y);
        return;
    }
    if app.settings_open {
        // 先判断滑条（拖动），再判断开关/按钮
        if let Some(def) = ui::settings_slider_hit(app, x, y) {
            apply_slider(app, def.id, def.value_at(x));
            ip.slider = Some(def.id);
            rt.redraw_full(app);
            return;
        }
        if let Some(hit) = ui::settings_hit(app, x, y) {
            handle_settings_hit(rt, app, hit);
        }
        return;
    }
    if app.whiteboard {
        if let Some(hit) = ui::whiteboard_control_hit(app, x, y) {
            match hit {
                ui::WhiteboardControl::Dots => {
                    app.whiteboard_dots = !app.whiteboard_dots;
                    app::save_settings(app);
                }
                ui::WhiteboardControl::DotSize => {
                    app.whiteboard_dot_size = ui::whiteboard_dot_size_at(app, x);
                    ip.dot_slider = true;
                }
                ui::WhiteboardControl::DotValue => open_value_input(rt,app,value_input::Target::DotSize),
            }
            apply_input_shape(rt, app);
            rt.redraw_full(app);
            return;
        }
    }
    if app.page_picker.is_some() {
        if ui::page_picker_rect(app).contains(x, y) {
            if button == 1 {
                match ui::page_picker_hit(app, x, y) {
                    Some(ui::PagePickerHit::Cancel) => close_all_popups(rt, app),
                    Some(ui::PagePickerHit::Input) => open_value_input(rt,app,value_input::Target::Page),
                    Some(ui::PagePickerHit::Jump) => commit_page_picker(rt, app, wps),
                    _ => (),
                }
            }
            return;
        }
        if let Some(right) = ui::page_count_at(app, x, y) {
            let same_side = app.popup_on_right == right;
            close_all_popups(rt, app);
            if !same_side { open_page_picker(rt, app, right); }
            return;
        }
        close_all_popups(rt, app);
    }
    if let Some(right) = ui::page_count_at(app, x, y) {
        if button == 1 { open_page_picker(rt, app, right); }
        return;
    }
    if app.more_menu_open {
        if let Some(hit) = ui::more_menu_hit(app, x, y) {
            app.more_menu_open = false;
            match hit {
                MoreHit::Screenshot => do_screenshot(rt),
                MoreHit::Settings => open_settings(rt, app),
            }
        } else {
            app.more_menu_open = false;
            rt.redraw_full(app);
        }
        apply_input_shape(rt, app);
        rt.redraw_full(app);
        return;
    }
    if app.pen_popup_visible {
        if let Some(hit) = ui::pen_popup_hit(app, x, y) {
            match hit {
                PopupHit::Color(i) => {
                    app.cur_color = i;
                }
                PopupHit::PenSize(i) => {
                    app.cur_pen = i;
                }
                _ => {}
            }
            rt.redraw_full(app);
            return;
        }
    }
    if app.eraser_popup_visible {
        if let Some(hit) = ui::eraser_popup_hit(app, x, y) {
            match hit {
                PopupHit::EraserSize(i) => {
                    app.cur_eraser = i;
                    app::save_settings(app);
                }
                PopupHit::ClearAll => clear_drawing(rt, app),
                _ => {}
            }
            rt.redraw_full(app);
            return;
        }
    }

    // 侧边栏按钮
    for right in [false, true] {
        for (k, r) in ui::sidebar_buttons(app, right) {
            if app.collapsed && k == Btn::Collapse { continue; }
            if first_hit(r, x, y) {
                log::debug!("[INPUT] hit {:?} right={} rect=({},{} {}x{})", k, right, r.x, r.y, r.w, r.h);
                app.popup_on_right = right;
                handle_btn(rt, app, wps, k, right);
                return;
            }
        }
        let sb = ui::sidebar_rect(app, right);
        if sb.contains(x, y) {
            // UI releases are left-button only, so right/wheel presses must
            // never start a drag that their matching release cannot finish.
            if button != 1 { return; }
            log::debug!("[INPUT] sidebar bg drag right={}", right);
            close_all_popups(rt, app);
            app.press = None;
            ip.dragging = true;
            ip.moved = false;
            app.sb_y = sb.y;
            ip.drag_start_y = app.sb_y;
            ip.drag_mouse_y0 = y;
            ip.collapse_press = app.collapsed;
            set_sidebar_drag_feedback(rt, app, Some(ui::SidebarDragFeedback {
                right,
            }));
            return;
        }
    }

    log::debug!("[INPUT] 未命中 UI (mode={})", app.mode);
    if dismiss_capture(app) {
        close_all_popups(rt, app);
        rt.redraw_full(app);
        return;
    }
    // 画布区域
    if app.mode != 0 {
        if button == 3 {
            if !app.whiteboard && app.right_click_cursor_on && app.mode == 1 {
                switch_to_cursor(rt, app);
                if rt.backend.can_forward_right_click() { ip.pending_right_click = Some((x, y)); }
            }
            return;
        }
        // 落在弹窗/菜单等 UI 上：不落笔
        if over_ui(app, x, y) {
            return;
        }
        close_all_popups(rt, app);
        app.begin_undo();
        let restore_was_visible = app.can_restore_cleared();
        ip.stroke_skipped = false;
        app.is_drawing = true;
        app.last_pt = (x, y);
        app.stroke_start_ms = now_ms();
        app.last_pen_w = app.timed_pen_width();
        let w;
        if app.mode == 2 {
            w = app.eraser_width();
            paint::stroke_segment(app, (x, y), (x, y), true, w);
        } else {
            w = app.last_pen_w;
            // 增量落笔：直接画圆点到画布，不再攒 stroke_pts 每帧重建整条
            paint::stroke_tapered(app, (x, y), (x, y), w, w);
        }
        let _ = button;
        if restore_was_visible && !app.can_restore_cleared() {
            for right in [false, true] {
                if let Some(r) = ui::control_rect(app, ui::Control::Sidebar(Btn::Clear, right)) {
                    rt.redraw(app, r);
                }
            }
        }
        rt.redraw(
            app,
            R {
                x: x - w,
                y: y - w,
                w: w * 2,
                h: w * 2,
            },
        );
    }
}

pub fn clear_drawing(rt: &Rt, app: &mut App) {
    close_all_popups(rt, app);
    // Clearing desktop annotations ends drawing; a whiteboard remains ready to write.
    app.remember_before_clear();
    app.is_drawing = false;
    app.reset_palm_gesture();
    app.clear_current_strokes();
    app.mode = if app.whiteboard { 1 } else { 0 };
    // Keep the tool group visible so the new restore action is reachable.
    app.drawing_tools.set_expanded(app.whiteboard || app.can_restore_cleared());
    apply_input_shape(rt, app);
    rt.redraw_full(app);
}

pub fn handle_btn(rt: &Rt, app: &mut App, wps: Option<&WpsBridge>, k: Btn, right: bool) {
    if !ui::control_enabled(app, ui::Control::Sidebar(k, right)) { return; }
    match k {
        Btn::Collapse => toggle_collapse(rt, app),
        Btn::Cursor => switch_to_cursor(rt, app),
        Btn::Pen => {
            if app.mode == 1 && app.drawing_tools.expanded() {
                let was = app.pen_popup_visible;
                close_all_popups(rt, app);
                app.pen_popup_visible = !was;
                if app.pen_popup_visible {
                    app.open_anim = now_ms();
                }
            } else {
                switch_to_draw(rt, app, 1);
            }
            apply_input_shape(rt, app);
            rt.redraw_full(app);
        }
        Btn::Eraser => {
            if app.mode == 2 && app.drawing_tools.expanded() {
                let was = app.eraser_popup_visible;
                close_all_popups(rt, app);
                app.eraser_popup_visible = !was;
                if app.eraser_popup_visible {
                    app.open_anim = now_ms();
                }
            } else {
                switch_to_draw(rt, app, 2);
            }
            apply_input_shape(rt, app);
            rt.redraw_full(app);
        }
        Btn::Clear => clear_drawing(rt, app),
        Btn::Restore => {
            if app.restore_cleared() {
                rt.redraw_full(app);
                apply_input_shape(rt, app);
            }
        }
        Btn::Undo => {
            app.undo_last();
            rt.redraw_full(app);
        }
        Btn::Prev => go_prev(rt, app, wps),
        Btn::Next => go_next(rt, app, wps),
        Btn::Exit => exit_presentation(rt, app, wps),
        Btn::Whiteboard => {
            close_all_popups(rt, app);
            app.toggle_whiteboard();
            if let Some(w) = wps { w.set_whiteboard(app.whiteboard); }
            apply_input_shape(rt, app);
            rt.redraw_full(app);
        }
        Btn::More => {
            let was = app.more_menu_open;
            close_all_popups(rt, app);
            app.more_menu_open = !was;
            if app.more_menu_open { app.open_anim = now_ms(); }
            apply_input_shape(rt, app);
            rt.redraw_full(app);
        }
    }
    let _ = right;
}

pub fn on_motion(rt: &Rt, app: &mut App, ip: &mut Ip, x: i32, y: i32) {
    if let Some(hit)=ip.picker_drag {
        settings_panel::picker_drag(app,hit,x,y);
        rt.redraw(app,settings_panel::picker_rect(app));
        return;
    }
    if let Some((start, initial, moved))=ip.panel_scroll {
        if moved || (y-start).abs()>9 {
            ip.panel_scroll=Some((start,initial,true));
            ip.pending_ui=None;
            app.press=None;
            if app.process_block_open {
                let max=ui::process_rows(app).len().saturating_sub(ui::process_visible_rows(app));
                app.process_scroll=(initial as i64+(start-y) as i64/52).clamp(0,max as i64) as usize;
            } else if app.show_diag {
                let pages=settings_panel::inspector_lines(app,&rt.fonts).len().div_ceil(settings_panel::inspector_page_size(app)).max(1);
                app.settings_ui.inspector_page=(initial as i64+(start-y) as i64/72).clamp(0,pages as i64-1) as usize;
            } else if app.settings_open {
                let max=ui::settings_layout(app).pages.saturating_sub(1);
                app.settings_page=(initial as i64+(start-y) as i64/72).clamp(0,max as i64) as usize;
            }
            rt.redraw_full(app);
            return;
        }
    }
    // A tap on the number edits it. Dragging from the same spot still
    // moves the slider, even when the thumb sits underneath the number.
    if ip.pending_ui == Some(ui::Control::Whiteboard(ui::WhiteboardControl::DotValue))
        && app.whiteboard && app.whiteboard_dots && x >= 0
        && (x - ip.dot_value_press_x).abs() > 4 {
        ip.pending_ui = None;
        ip.dot_slider = true;
        app.press = None;
    }
    if ip.dot_slider {
        if app.whiteboard && app.whiteboard_dots {
            let size = ui::whiteboard_dot_size_at(app, x);
            if size != app.whiteboard_dot_size {
                app.whiteboard_dot_size = size;
                rt.redraw(app, ui::whiteboard_bar_rect(app));
                rt.redraw_full(app);
            }
        }
        return;
    }
    if app.page_picker.as_ref().is_some_and(|p| p.dragging()) {
        if let Some(picker) = app.page_picker.as_mut() { picker.motion(y, Instant::now()); }
        rt.redraw(app, ui::page_picker_rect(app));
        return;
    }
    // Moving an idle mouse has no visual effect. Only an active press reacts.
    if let Some(control) = ip.pending_ui {
        let active = ui::control_enabled(app, control) && ui::control_at(app, x, y) == Some(control);
        if let Some(press) = app.press.as_mut() {
            if press.active != active {
                press.active = active;
                if let Some(r) = ui::control_rect(app, control) { rt.redraw(app, r); }
            }
        }
        return;
    }
    // 设置面板滑条拖动：只重绘设置面板区域（约 5ms，而非整屏 ~58ms）
    if let Some(id) = ip.slider {
        if let Some(def) = ui::settings_sliders(app).into_iter().find(|s| s.id == id) {
            let old_sidebars = [ui::sidebar_input_rect(app, false), ui::sidebar_input_rect(app, true)];
            apply_slider(app, id, def.value_at(x));
            rt.redraw(app, ui::settings_rect(app));
            // 透明度/大小会改变侧栏本身，需一并重绘两侧栏
            if matches!(id, ui::SliderId::Alpha | ui::SliderId::Scale | ui::SliderId::Position) {
                for old in old_sidebars { rt.redraw(app, old); }
                for right in [false, true] {
                    rt.redraw(app, ui::sidebar_rect(app, right));
                }
            }
        }
        return;
    }
    if ip.dragging {
        let dy = y - ip.drag_mouse_y0;
        if !ip.moved && dy.abs() <= 4 {
            return;
        }
        ip.moved = true;
        let h = if app.collapsed {
            app.collapsed_h()
        } else {
            app.sb_height()
        };
        let old_y = app.sidebar_y(h);
        let new_y = (ip.drag_start_y + dy).clamp(0, (app.screen_h - h).max(0));
        // Rebase at each sample, including a clamped edge. Reversing direction
        // must move immediately rather than requiring the pointer to retrace
        // the distance it travelled past the screen boundary.
        ip.drag_start_y = new_y;
        ip.drag_mouse_y0 = y;
        app.sb_y = new_y;
        if new_y == old_y {
            return;
        }
        // 只重绘左右两条侧栏窄条（新旧位置并集），不整屏也不整宽
        let sbw = app.sb_width();
        let y0 = (old_y.min(new_y) - 8).max(0);
        let y1 = ((old_y.max(new_y) + h) + 8).min(app.screen_h);
        let hh = (y1 - y0).max(1);
        for right in [false, true] {
            let sbx = if right {
                app.sb_x_right()
            } else {
                app.sb_x_left()
            };
            rt.redraw(
                app,
                R {
                    x: sbx - 8,
                    y: y0,
                    w: sbw + 16,
                    h: hh,
                },
            );
        }
        // Keep the original input target alive until release. X11/layer-shell
        // retain their implicit grab; GNOME holds an explicit Clutter grab.
        return;
    }
    if !app.is_drawing || app.mode == 0 {
        return;
    }
    let cur = (x, y);
    let erase = app.mode == 2;
    let w = if erase {
        app.eraser_width()
    } else {
        app.timed_pen_width()
    };
    // 笔尖在 UI 上：暂停本笔画（不画、不动 last_pt），出了 UI 再续
    if over_ui(app, x, y) {
        ip.stroke_skipped = true;
        return;
    }
    if ip.stroke_skipped {
        // 刚从 UI 出来：从当前点新起一段（避免跨侧栏拉一条线）
        ip.stroke_skipped = false;
        app.last_pen_w = w;
        app.last_pt = cur;
        if erase {
            paint::stroke_segment(app, cur, cur, true, w);
        } else {
            paint::stroke_tapered(app, cur, cur, w, w);
        }
        rt.redraw(
            app,
            R {
                x: cur.0 - w,
                y: cur.1 - w,
                w: w * 2,
                h: w * 2,
            },
        );
        return;
    }
    let prev = app.last_pt;
    if erase {
        paint::stroke_segment(app, prev, cur, true, w);
    } else {
        // 增量绘制本段（前一宽度→当前宽度），O(1)，不再重建整条笔迹
        paint::stroke_tapered(app, prev, cur, app.last_pen_w, w);
    }
    app.last_pen_w = w;
    app.last_pt = cur;
    let x1 = prev.0.min(cur.0) - w;
    let y1 = prev.1.min(cur.1) - w;
    let x2 = prev.0.max(cur.0) + w;
    let y2 = prev.1.max(cur.1) + w;
    rt.redraw(
        app,
        R {
            x: x1,
            y: y1,
            w: x2 - x1,
            h: y2 - y1,
        },
    );
}

pub fn on_release(rt: &Rt, app: &mut App, ip: &mut Ip, wps: Option<&WpsBridge>, x: i32, y: i32, button: u8) {
    if button == 4 || button == 5 { return; }
    if button != 1 && (ip.panel_scroll.is_some() || ip.picker_drag.is_some()) { return; }
    if ip.picker_drag.take().is_some() { app.press=None; rt.redraw_full(app); return; }
    if ip.panel_scroll.take().is_some_and(|(_,_,moved)|moved) { ip.pending_ui=None; app.press=None; rt.redraw_full(app); return; }
    if ip.dot_slider {
        ip.dot_slider = false;
        app::save_settings(app);
        return;
    }
    if app.page_picker.as_ref().is_some_and(|p| p.dragging()) {
        if button != 1 { return; }
        let r = ui::page_picker_control_rect(app, ui::PagePickerHit::Wheel);
        if let Some(picker) = app.page_picker.as_mut() { picker.release(y, r.y + r.h / 2, Instant::now()); }
        rt.redraw(app, ui::page_picker_rect(app));
        return;
    }
    if button != 1 && (ip.pending_ui.is_some() || ip.slider.is_some() || ip.dragging) { return; }
    if let Some(control) = ip.pending_ui.take() {
        let activate = ui::control_enabled(app, control) && ui::control_at(app, x, y) == Some(control);
        if let Some(press) = app.press.as_mut() {
            press.active = activate;
            press.release();
        }
        if activate { activate_press(rt, app, ip, wps, x, y, 1); }
        if let Some(r) = ui::control_rect(app, control) { rt.redraw(app, r); }
        return;
    }
    if button == 3 {
        if let Some((x, y)) = ip.pending_right_click.take() {
            rt.backend.flush();
            rt.virtual_right_click(x, y);
        }
    }
    if ip.slider.is_some() {
        ip.slider = None;
        if let Some(press) = app.press.as_mut() { press.release(); }
        app::save_settings(app);
        apply_input_shape(rt, app);
        rt.redraw_full(app);
    }
    if app.is_drawing {
        app.is_drawing = false;
        paint::commit_stroke(app);
        let w = if app.mode == 2 {
            app.eraser_width()
        } else {
            app.pen_width()
        };
        let (px, py) = app.last_pt;
        rt.redraw(
            app,
            R {
                x: px - w,
                y: py - w,
                w: w * 2,
                h: w * 2,
            },
        );
    }
    if ip.dragging {
        // Include the release position even if its final motion was coalesced.
        on_motion(rt, app, ip, x, y);
        let released_inside = app.sidebar_drag.is_some_and(
            |drag| ui::sidebar_rect(app, drag.right).contains(x, y));
        ip.dragging = false;
        set_sidebar_drag_feedback(rt, app, None);
        if ip.collapse_press && !ip.moved && released_inside {
            toggle_collapse(rt, app);
        } else if ip.moved {
            // 拖动结束：按新位置更新输入区域
            apply_input_shape(rt, app);
        }
    }
}

/// 触摸事件（X11 与 Wayland 共用）
pub fn cancel_input(rt: &Rt, app: &mut App, ip: &mut Ip) {
    if let Some(picker) = app.page_picker.as_mut() { picker.cancel_drag(); }
    if app.is_drawing { paint::commit_stroke(app); }
    app.is_drawing = false;
    app.reset_palm_gesture();
    app.touch_frame_contacts = 0;
    app.press = None;
    app.sidebar_drag = None;
    *ip = Ip::default();
    apply_input_shape(rt, app);
    rt.redraw_full(app);
}

/// 触摸事件（X11 与 Wayland 共用）
pub fn handle_touch(
    rt: &Rt,
    app: &mut App,
    ip: &mut Ip,
    wps: Option<&WpsBridge>,
    id: i32,
    x: i32,
    y: i32,
    kind: TouchKind,
    diameter: f64,
) {
    ip.last_touch_ms = now_ms();
    if kind != TouchKind::End {
        ip.last_touch_pos = (x, y);
    }
    if app.launch.active() {
        // A finger held through the intro remains ignored until its own End;
        // it must not become a drawing stroke when the toolbar becomes ready.
        match kind {
            TouchKind::Begin => { ip.touch_ids.insert(id); ip.touch_over_ui.insert(id, true); }
            TouchKind::End => { ip.touch_ids.remove(&id); ip.touch_over_ui.remove(&id); ip.touch_shape.remove(&id); }
            TouchKind::Update => (),
        }
        return;
    }
    let wants_touch = app.launch.active() || app.whiteboard || app.mode != 0
        || app.settings_open || app.process_block_open || app.settings_ui.modal() || app.confirm_quit || app.show_diag || app.show_input_group_prompt
        || dismiss_capture(app) || over_ui(app, x, y);
    match kind {
        TouchKind::Begin => {
            if !wants_touch { return; }
            if dismiss_capture(app) && !over_ui(app, x, y) {
                close_all_popups(rt, app);
                rt.redraw_full(app);
                return;
            }
            ip.touch_ids.insert(id);
            if over_ui(app, x, y) || ip.ui_touch.is_some() {
                ip.touch_over_ui.insert(id, true);
                // 先清掉陈旧拖动状态，再交给 on_press：命中侧栏空白会置 dragging=true，
                // 命中按钮/滑条则不会，从而保留触屏拖动侧边栏的能力。
                // Only the first finger owns a control; other fingers cannot
                // toggle it twice or release somebody else's slider.
                if ip.touch_ids.len() == 1 && ip.pending_ui.is_none() && !app.is_drawing {
                    ip.ui_touch = Some(id);
                    ip.dragging = false;
                    set_sidebar_drag_feedback(rt, app, None);
                    on_press(rt, app, ip, wps, x, y, 1);
                }
            } else {
                ip.touch_over_ui.insert(id, false);
                close_all_popups(rt, app);
                if let Some(_r) = gesture::touch_event(app, id, x, y, TouchKind::Begin, diameter) {
                    rt.redraw_full(app);
                }
            }
        }
        TouchKind::Update => {
            if ip.rejected_touch.contains(&id) { return; }
            if !ip.touch_ids.contains(&id) { return; }
            if ip.ui_touch == Some(id) {
                on_motion(rt, app, ip, x, y);
                return;
            }
            if ip.touch_over_ui.get(&id).copied().unwrap_or(false) {
                return;
            }
            if over_ui(app, x, y) {
                app.reset_palm_gesture();
                return;
            }
            if let Some(r) = gesture::touch_event(app, id, x, y, TouchKind::Update, diameter) {
                rt.redraw(app, r);
            }
        }
        TouchKind::End => {
            if ip.rejected_touch.remove(&id) { return; }
            if !ip.touch_ids.contains(&id) { return; }
            ip.touch_ids.remove(&id);
            let over = ip.touch_over_ui.remove(&id).unwrap_or(false);
            if over {
                if ip.ui_touch == Some(id) {
                    ip.ui_touch = None;
                    on_release(rt, app, ip, wps, x, y, 1);
                }
                return;
            }
            if let Some(r) = gesture::touch_event(app, id, x, y, TouchKind::End, 0.0) {
                rt.redraw(app, r);
            }
        }
    }
}

pub struct Timers {
    last_wps: Instant,
    last_foreground: Instant,
    foreground: Option<bool>,
}

impl Timers {
    pub fn new() -> Self {
        Timers {
            last_wps: Instant::now() - Duration::from_secs(10),
            last_foreground: Instant::now() - Duration::from_secs(10),
            foreground: None,
        }
    }
}

/// 与具体后端无关的周期性任务：WPS 桥/事件/心跳/全屏检测、白板提示、侧栏动画
pub fn tick(
    rt: &Rt,
    app: &mut App,
    ip: &mut Ip,
    wps_bridge: &mut Option<WpsBridge>,
    t: &mut Timers,
    system_dark: bool,
) {
    #[cfg(unix)]
    settings_window::poll(rt, app, ip, wps_bridge.as_ref());
    let update = poll_wps(app.wps_debug, app.whiteboard, app.wps_test_mode, wps_bridge);
    tick_shared(rt, app, ip, wps_bridge.as_ref(), &update, t, system_dark);
    resolve_drawing_tools_auto_update(rt, app);
}

pub fn set_drawing_tools_expanded(rt: &Rt, app: &mut App, expanded: bool) {
    if app.drawing_tools.expanded() == expanded { return; }
    close_all_popups(rt, app);
    app.drawing_tools.set_expanded(expanded);
    apply_input_shape(rt, app);
    rt.redraw_full(app);
}

pub fn resolve_drawing_tools_auto_update(rt: &Rt, app: &mut App) {
    let Some(update) = app.drawing_tools.take_auto_update() else { return };
    let Some(has_ink) = app.drawing_tools_auto_state() else { return };
    if let Some(expanded) = update.expanded_for(has_ink) {
        set_drawing_tools_expanded(rt, app, expanded);
    }
    // input 组授权结果同步到界面
    let igs = INPUT_GROUP_STATE.load(std::sync::atomic::Ordering::Relaxed);
    if igs != app.input_group_state {
        app.input_group_state = igs;
        if app.show_input_group_prompt {
            rt.redraw(app, ui::input_group_prompt_rect(app));
        }
    }
}

pub struct WpsUpdate {
    connected: bool,
    events: Vec<WpsEvent>,
}

pub fn begin_launch(rt: &Rt, app: &mut App) -> bool {
    if std::env::var_os("SIDERA_NO_SPLASH").is_some() { return false; }
    app.launch = launch::Launch::begin(
        format!("{} · {} × {}", rt.backend.description(), app.screen_w, app.screen_h),
        !rt.fonts.list.is_empty() && rt.icon_big.is_some(), app.hotkey_ok, Instant::now());
    true
}

fn tick_launch(rt: &Rt, app: &mut App, bridge: Option<&WpsBridge>, connected: bool) {
    if !app.launch.active() { return; }
    let now = Instant::now();
    let old_alpha = app.launch.panel_alpha();
    let old_reveal = app.launch.reveal();
    let link = if !app.wps_debug { launch::LinkState::Disabled }
        else if connected { launch::LinkState::Connected }
        else if bridge.is_some() { launch::LinkState::Waiting }
        else { launch::LinkState::Unavailable };
    // poll_wps has completed a real service setup attempt before this call.
    let report_changed = app.launch.report_link(link, now);
    app.launch.advance_at(now);
    if report_changed || old_alpha != app.launch.panel_alpha() { rt.redraw(app, ui::launch_rect(app)); }
    if old_reveal != app.launch.reveal() {
        for right in [false, true] { rt.redraw(app, ui::sidebar_transition_rect(app, right)); }
    }
    if (old_alpha > 0.0) != (app.launch.panel_alpha() > 0.0)
        || (old_reveal > 0.0) != (app.launch.reveal() > 0.0) || !app.launch.active() {
        apply_input_shape(rt, app);
    }
}

pub fn poll_wps(enabled: bool, whiteboard: bool, test_mode: bool, wps_bridge: &mut Option<WpsBridge>) -> WpsUpdate {
    // 静止触点没有 Motion 事件，不按闲置时长猜测已经抬手。
    // 取消、设备移除和输出移除由后端明确调用 cancel_input。
    // WPS 桥存在性与设置同步
    if enabled && wps_bridge.is_none() {
        *wps_bridge = WpsBridge::start();
    } else if !enabled && wps_bridge.is_some() {
        *wps_bridge = None;
    }
    if let Some(w) = wps_bridge.as_ref() {
        w.set_whiteboard(whiteboard);
        w.set_test_mode(test_mode);
        w.tick();
        WpsUpdate { connected: w.connected(), events: w.take_events() }
    } else {
        WpsUpdate { connected: false, events: Vec::new() }
    }
}

pub fn tick_shared(rt: &Rt, app: &mut App, _ip: &mut Ip,
    wps_bridge: Option<&WpsBridge>, update: &WpsUpdate, t: &mut Timers, system_dark: bool) {
    if app.update_system_theme(system_dark) {
        rt.redraw_preferences(app, app.previewing_sidebars());
    }
    let now=now_ms();
    if app.settings_ui.animating() && now-app.settings_ui.last_frame>=16 {
        app.settings_ui.last_frame=now;
        if app.settings_ui.window_size().is_some() {
            // A modal fade changes only its own window. A theme fade also
            // changes the sidebar samples on the Appearance tab.
            rt.redraw_preferences(app, app.previewing_sidebars() && app.settings_ui.theme_from.is_some());
        } else if app.settings_ui.theme_from.is_some() || app.settings_ui.modal() {
            rt.redraw_full(app);
        } else {
            for (visible,r) in [(app.settings_open,ui::settings_rect(app)),
                (app.process_block_open,ui::process_block_rect(app)),
                (app.show_diag,ui::diag_rect(app)),(app.confirm_quit,ui::confirm_rect(app))] {
                if visible { rt.redraw(app,R {x:r.x-12,y:r.y-12,w:r.w+24,h:r.h+24}); }
            }
        }
    }
    if app.settings_ui.closing_at>0 && now-app.settings_ui.closing_at>=120 {
        #[cfg(unix)]
        settings_window::close(rt, app, _ip);
        app.settings_ui.closing_at=0;
        app.settings_open=false;
        apply_input_shape(rt,app);
        rt.redraw_full(app);
    }
    if app.settings_ui.overlay_closing_at>0 && now-app.settings_ui.overlay_closing_at>=120 {
        app.settings_ui.overlay_closing_at=0;
        app.settings_ui.dialog=None;
        app.settings_ui.picker=None;
        _ip.picker_drag=None;
        apply_input_shape(rt,app);
        rt.redraw_preferences(app, false);
    }
    if app.settings_ui.theme_from.is_some_and(|(_,start)|now-start>=220) {
        app.settings_ui.theme_from=None;
        app.launch.card.get_mut().take();
        rt.redraw_preferences(app, app.previewing_sidebars());
    }
    if app.settings_ui.toggle.is_some_and(|(_,_,start)|now-start>=180) {
        app.settings_ui.toggle=None;
        rt.redraw_preferences(app, false);
    }
    if app.settings_ui.toast_until>0 && now>=app.settings_ui.toast_until {
        app.settings_ui.toast_until=0;
        if app.settings_open {rt.redraw(app,ui::settings_rect(app));}
    }
    tick_launch(rt, app, wps_bridge, update.connected);
    if app.sidebar_expand_due() { expand_sidebars(rt, app); }
    // Limit X11 round trips in the periodic repaint path. User actions also
    // check focus directly so they cannot use stale navigation availability.
    let was_visible = t.foreground != Some(false);
    if t.last_foreground.elapsed() >= Duration::from_millis(250) {
        t.last_foreground = Instant::now();
        t.foreground = rt.wps_foreground();
        let now_visible = t.foreground != Some(false);
        if now_visible != was_visible {
            if now_visible {
                diag_log::info("WPS 演示进入前台，恢复放映联动");
            } else {
                if let Some(bridge) = wps_bridge { bridge.discard_pending_navigation(); }
                diag_log::info("WPS 演示离开前台，暂停放映联动");
            }
        }
    }
    let wps_visible = t.foreground != Some(false);
    let jump_epoch = if app.wps_debug && wps_visible { wps_bridge.and_then(WpsBridge::jump_epoch) } else { None };
    let jump_changed = app.wps_can_jump != jump_epoch.is_some() || app.wps_deck_epoch != jump_epoch.unwrap_or(-1);
    app.wps_can_jump = jump_epoch.is_some();
    app.wps_deck_epoch = jump_epoch.unwrap_or(-1);
    let pages = if app.wps_debug && wps_visible {
        wps_bridge.map(WpsBridge::page_progress).unwrap_or((-1, -1))
    } else { (-1, -1) };
    let show_pages = app.whiteboard || (app.wps_debug && update.connected && (pages.0 > 0 || pages.1 > 0));
    let layout_changed = app.sidebar_page_visible() != show_pages;
    let old_sidebars = [ui::sidebar_input_rect(app, false), ui::sidebar_input_rect(app, true)];
    // Popups follow toolbar geometry. Close them using their old bounds before
    // changing the layout so no stale panel pixels or hit regions remain.
    if layout_changed { close_all_popups(rt, app); }
    if app.wps_connected != update.connected {
        app.wps_connected = update.connected;
        if app.settings_open { rt.redraw(app, ui::settings_rect(app)); }
    }

    if (app.wps_page_index, app.wps_page_total) != pages {
        (app.wps_page_index, app.wps_page_total) = pages;
        if !layout_changed && show_pages && !app.collapsed {
            for right in [false, true] { rt.redraw(app, ui::sidebar_page_rect(app, right)); }
        }
    }
    if jump_changed && app.sidebar_page_visible() && !app.collapsed {
        for right in [false, true] { rt.redraw(app, ui::sidebar_page_rect(app, right)); }
    }
    if app.page_picker.as_ref().is_some_and(|picker| {
        app.page_picker_context().is_none_or(|(_, total, source)| total != picker.total || source != picker.source)
    }) { close_all_popups(rt, app); }
    if app.page_picker.as_mut().is_some_and(|p| p.advance_at(Instant::now())) {
        rt.redraw(app, ui::page_picker_rect(app));
    }
    if layout_changed {
        for (i, right) in [false, true].into_iter().enumerate() {
            rt.redraw(app, old_sidebars[i]);
            rt.redraw(app, ui::sidebar_input_rect(app, right));
        }
        apply_input_shape(rt, app);
    }

    // WPS 事件（真实页号 / 放映开始）
    // 后台放映的开始/翻页事件不切换当前画布；结束事件仍处理，避免残留放映状态。
    {
        for ev in update.events.iter().cloned() {
            if !wps_visible && !matches!(ev, WpsEvent::SlideshowEnd) {
                continue;
            }
            match ev {
                WpsEvent::SlideshowBegin(pos) => {
                    app.clear_slide_pages();
                    app.wps_real_pos = pos;
                    if app.whiteboard {
                        app.saved_slide = pos;
                    } else {
                        app.current_slide = pos;
                    }
                    rt.redraw_full(app);
                }
                WpsEvent::SlideshowEnd => {
                    app.clear_slide_pages();
                    app.wps_real_pos = -1;
                    rt.redraw_full(app);
                }
                WpsEvent::RealPos(pos) => {
                    if app.whiteboard {
                        app.saved_slide = pos;
                        app.wps_real_pos = pos;
                        continue;
                    }
                    if app.current_slide == pos && app.wps_real_pos == pos { continue; }
                    if app.wps_real_pos > 0 {
                        app.save_current_page();
                    }
                    app.wps_real_pos = pos;
                    app.current_slide = pos;
                    app.load_page(pos);
                    rt.redraw_full(app);
                }
            }
        }
    }



    // 全屏检测（1s；始终扫描，避免加载项连着但已退出放映时状态变陈旧）
    if t.last_wps.elapsed() >= Duration::from_millis(1000) {
        t.last_wps = Instant::now();
        let fs = rt.is_fullscreen();
        if fs != app.wps_fullscreen {
            app.wps_fullscreen = fs;
            if !app.whiteboard && !app.wps_debug {
                app.clear_slide_pages();
            }
            rt.redraw_full(app);
        }
        if app.mode == 0 && !app.settings_open {
            apply_input_shape(rt, app);
        }
    }

    let linked_show = app.wps_debug && wps_visible && wps_bridge.is_some_and(|w| w.showing());
    // Reconnecting to the same reported slide need not emit a new WPS event.
    // Reconcile the locally estimated page without filing its ink under an old page.
    if app.wps_debug && wps_visible && !app.whiteboard {
        if let Some(pos) = wps_bridge.and_then(|w| w.position()) {
            if app.current_slide != pos {
                app.save_current_page();
                app.current_slide = pos;
                app.load_page(pos);
                rt.redraw_full(app);
            }
            app.wps_real_pos = pos;
        }
    }
    let navigation_ready = (!app.wps_debug || !update.connected || wps_visible)
        && navigation_route(linked_show, wps_bridge.is_some_and(|w| w.ready()),
            rt.backend.can_inject_keys_without_prompt()) != NavigationRoute::Unavailable;
    let action = if linked_show {
        ui::PresentationAction::End
    } else if app.wps_debug && wps_visible && (
        (!app.wps_test_mode && update.connected)
            || (app.wps_test_mode && wps_bridge.is_some_and(|w| w.start_available()))
    ) {
        ui::PresentationAction::Start
    } else {
        ui::PresentationAction::Escape
    };
    if app.wps_navigation_ready != navigation_ready {
        app.wps_navigation_ready = navigation_ready;
        rt.redraw_full(app);
    }
    app.presentation_action = action;
    let presentation_frame = Instant::now();
    app.presentation_transition.set_action_at(action, presentation_frame);
    if app.presentation_transition.advance_at(presentation_frame)
        && !app.whiteboard && !ui::sidebar_animating(app) {
        // This visual transition does not change geometry, input regions or
        // click availability. Moving sidebars already repaint their full strip.
        for right in [false, true] {
            if let Some(r) = ui::control_rect(app, ui::Control::Sidebar(Btn::Exit, right)) {
                rt.redraw(app, r);
            }
        }
    }

    if app.press.is_some_and(|p| p.expired()) {
        let old = app.press.take().unwrap();
        if let Some(r) = ui::control_rect(app, old.control) { rt.redraw(app, r); }
    }

    // 白板上限提示到点重绘
    if app.wb_limit_msg_until != 0 && now_ms() > app.wb_limit_msg_until {
        app.wb_limit_msg_until = 0;
        rt.redraw_full(app);
    }
    if app.log_cleared_until != 0 && now_ms() > app.log_cleared_until {
        app.log_cleared_until = 0;
        if app.settings_open {
            rt.redraw(app, ui::settings_rect(app));
        }
    }

    if ui::sidebar_animating(app) {
        app.drawing_tools.advance_at(Instant::now());
        app.finish_sidebar_animation();
        for right in [false, true] { rt.redraw(app, ui::sidebar_transition_rect(app, right)); }
        if !ui::sidebar_animating(app) { apply_input_shape(rt, app); }
    }

    // 弹窗/菜单淡入动画期间持续重绘（只重绘对应弹窗区域）
    if app.open_anim != 0 && now_ms() < app.open_anim + 180 {
        if app.page_picker.is_some() { rt.redraw(app, ui::page_picker_rect(app)); }
        if app.pen_popup_visible {
            rt.redraw(app, ui::pen_popup_rect(app));
        }
        if app.eraser_popup_visible {
            rt.redraw(app, ui::eraser_popup_rect(app));
        }
        if app.more_menu_open {
            rt.redraw(app, ui::more_menu_rect(app));
        }
    }
}
