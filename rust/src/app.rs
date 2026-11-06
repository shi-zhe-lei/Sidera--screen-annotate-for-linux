// ============================================================
// Sidera - 全局状态 / 常量 / 配置持久化
// 纯 Rust 重写，不依赖 Qt；X11 由 x11.rs 提供
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
// ============================================================
use std::collections::{HashMap, VecDeque};
use crate::page_cache::PageCache;

use tiny_skia::{Color, Pixmap};

// ---------------- 常量（与原 C++ app.h 对齐） ----------------
pub const VERSION: &str = "3.1-Electro-stable";
// 侧边栏基准尺寸（与 C++ 版对齐：56/34/24/19；可在设置里用「侧边栏大小」缩放）
pub const SB_BASE_W: f64 = 56.0;
pub const SB_BASE_BTN: f64 = 34.0;
pub const SB_BASE_ICON: f64 = 24.0;
pub const SB_BASE_DOT: f64 = 19.0;
pub const DEFAULT_SB_SCALE: f64 = 1.0;
pub const DEFAULT_SIDEBAR_ALPHA: u8 = 80;
pub const DEFAULT_LARGE_TOUCH_THRESHOLD: f64 = 64.0;
pub const DEFAULT_LARGE_ERASE_SCALE10: i32 = 12;
pub const DEFAULT_STARTUP_DELAY: u32 = 10;
pub const MIN_STARTUP_DELAY: u32 = 5;
pub const MAX_STARTUP_DELAY: u32 = 20;
pub const MAX_UNDO: usize = 12;
/// 撤回瓦片边长（像素）；内存与笔迹覆盖面积成正比，而非整张画布
pub const UNDO_TILE: i32 = 128;
pub const CACHE_BOARD: i32 = 10;
pub const PALM_ERASE_WIDTH: i32 = 64;
pub const STROKE_RAMP_MS: i64 = 600;
pub const STROKE_MIN_SCALE: f64 = 0.85;

pub const WHITEBOARD_COLORS: [(u8, u8, u8); 2] = [(0x0F, 0x3D, 0x2E), (255, 255, 255)];
pub const WHITEBOARD_DOT_PITCH: i32 = 32;
pub const DEFAULT_WHITEBOARD_DOT_SIZE: i32 = 3;

pub const COLORS: [(u8, u8, u8); 10] = [
    (255, 40, 40),
    (50, 120, 255),
    (40, 200, 60),
    (255, 210, 30),
    (240, 240, 240),
    (0x0A, 0xBA, 0xB5),
    (0xAB, 0x47, 0xBC),
    (0xC9, 0xDD, 0x22),
    (0xFF, 0x6D, 0x00),
    (0x79, 0x55, 0x48),
];
pub const PEN_SIZES: [i32; 3] = [3, 5, 10];
pub const ERASER_SIZES: [i32; 3] = [30, 60, 80];

pub fn now_ms() -> i64 {
    use std::sync::OnceLock;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    // Keep nonzero, epoch-like values for existing sentinels; subsequent deadlines
    // use elapsed time, so correcting the system clock cannot freeze input/queues.
    static CLOCK: OnceLock<(Instant, i64)> = OnceLock::new();
    let (start, base) = CLOCK.get_or_init(|| (Instant::now(),
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(1)));
    base.saturating_add(start.elapsed().as_millis() as i64)
}

pub fn color(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

pub fn clone_pixmap(p: &Pixmap) -> Pixmap {
    let mut n = Pixmap::new(p.width(), p.height()).expect("pixmap");
    n.data_mut().copy_from_slice(p.data());
    n
}

/// 一次撤回记录：按瓦片保存落笔前像素，只存被笔迹覆盖到的瓦片
#[derive(Default)]
pub struct UndoEntry {
    tiles: HashMap<(i32, i32), Vec<u8>>,
}

// ---------------- 全局状态 ----------------
pub struct App {
    // 模式 0=光标 1=画笔 2=橡皮
    pub mode: i32,
    pub pen_popup_visible: bool,
    pub eraser_popup_visible: bool,
    pub more_menu_open: bool,
    pub more_settings_entry: bool,
    pub process_block_open: bool,
    pub process_block_enabled: bool,
    pub process_rules: Vec<String>,
    pub process_scan: Vec<crate::process_block::ProcInfo>,
    pub process_input: String,
    pub process_input_active: bool,
    pub process_scroll: usize,
    pub page_picker: Option<crate::page_picker::PagePicker>,
    pub settings_open: bool,
    pub settings_page: usize,
    pub popup_on_right: bool,

    // 触屏按下反馈 / 弹窗淡入动画 / 设置里的确认与诊断
    pub press: Option<crate::ui::PressFeedback>,
    pub open_anim: i64,
    pub confirm_quit: bool,
    pub show_diag: bool,
    pub diag_text: String,
    // input 组授权弹窗：0=待授权 1=授权中 2=成功 3=失败
    pub show_input_group_prompt: bool,
    pub input_group_state: i32,

    // 白板
    pub whiteboard: bool,
    pub whiteboard_bg_index: i32,
    pub whiteboard_dots: bool,
    pub whiteboard_dot_size: i32,
    pub saved_mode: i32, // 退出白板时恢复进入前的工具。
    pub saved_drawing_tools: bool,

    // 侧边栏
    pub launch: crate::launch::Launch,
    pub collapsed: bool,
    pub drawing_tools: crate::toolbar::DrawingTools,
    pub sidebar_drag: Option<crate::ui::SidebarDragFeedback>,
    pub sb_y: i32,
    pub sidebar_anim_start: Option<std::time::Instant>,
    pub sidebar_anim_from: f32,
    pub sidebar_collapsed_since: Option<std::time::Instant>,

    // 颜色/粗细
    pub cur_color: usize,
    pub cur_pen: usize,
    pub cur_eraser: usize,

    // 绘制状态
    pub is_drawing: bool,
    pub last_pt: (i32, i32),
    pub stroke_start_ms: i64,
    pub last_pen_w: i32,

    // 侧边栏外观
    pub sb_scale: f64,
    pub sidebar_alpha: u8,
    pub theme: crate::ui_theme::Theme,

    // 平台
    pub hotkey_ok: bool,

    // WPS
    pub wps_fullscreen: bool,
    pub wps_debug: bool,
    pub wps_test_mode: bool,
    pub startup_delay: u32,
    pub wps_start_sent_until: i64,
    pub wps_connected: bool,
    pub wps_real_pos: i32,
    pub wps_page_index: i32,
    pub wps_page_total: i32,
    pub wps_can_jump: bool,
    pub wps_deck_epoch: i32,
    pub wps_navigation_ready: bool,
    pub presentation_action: crate::ui::PresentationAction,
    pub presentation_transition: crate::toolbar::PresentationTransition,
    pub touch_frame_contacts: usize,

    // 多页缓存
    pub slide_cache: PageCache,
    pub whiteboard_cache: PageCache,
    pub current_slide: i32,
    // 两种模式各自记住上次页码；切换模式不会销毁另一份缓存。
    pub saved_slide: i32,
    pub whiteboard_slide: i32,
    pub page_has_ink: bool,
    pub wb_limit_msg_until: i64,

    // 手掌/大触点橡皮
    pub palm_erase_preview: Vec<(i32, i32)>,
    pub palm_erase_w: i32,
    pub right_click_cursor_on: bool,
    pub large_touch_threshold: f64,
    pub large_erase_scale10: i32,

    // 手势引擎
    pub t_prev_pos: HashMap<i32, (i32, i32)>,
    pub t_prev_role: HashMap<i32, i32>,
    pub t_prev_preview: Vec<(i32, i32)>,
    pub t_palm_latched: bool,
    pub t_moved: bool,
    pub t_begin_pos: (i32, i32),
    pub t_begin_role: i32,

    // 进行中的画笔笔画（采样点 x,y,宽度），松手时一次性提交，保证边缘平滑
    pub stroke_pts: Vec<(f32, f32, f32)>,

    // 撤回栈：每个条目只保存本次操作覆盖到的瓦片“原始像素”
    pub undo: VecDeque<UndoEntry>,
    // 手动清空前的整页快照（容量 1，压缩存储）
    pub clear_history: crate::clear_history::ClearHistory,

    // 画布
    pub canvas: Pixmap,
    pub screen_w: i32,
    pub screen_h: i32,
    /// 设备缩放（Wayland fractional scale）；逻辑坐标 → 物理像素 = *scale
    pub scale: f64,

    // 撤销/调试提示：设置面板里“清空日志”反馈
    pub log_cleared_until: i64,
}

pub fn valid_geometry(w: i32, h: i32, scale: f64) -> bool {
    w > 0 && h > 0 && scale.is_finite() && (1.0..=4.0).contains(&scale)
        && ((w as f64 * scale).round() * (h as f64 * scale).round()) <= 67_108_864.0
}

fn fit_pixmap(source: &Pixmap, w: u32, h: u32) -> Pixmap {
    if (source.width(), source.height()) == (w, h) {
        return clone_pixmap(source);
    }
    let mut target = Pixmap::new(w, h).expect("validated canvas geometry");
    target.draw_pixmap(0, 0, source.as_ref(), &tiny_skia::PixmapPaint {
        quality: tiny_skia::FilterQuality::Bilinear, ..Default::default()
    }, tiny_skia::Transform::from_scale(w as f32 / source.width() as f32,
                                        h as f32 / source.height() as f32), None);
    target
}

pub fn whiteboard_bg(sel: i32) -> Color {
    let i = sel.clamp(0, WHITEBOARD_COLORS.len() as i32 - 1) as usize;
    let (r, g, b) = WHITEBOARD_COLORS[i];
    color(r, g, b, 255)
}

impl App {
    pub fn new(canvas: Pixmap, screen_w: i32, screen_h: i32) -> Self {
        App {
            mode: 0,
            pen_popup_visible: false,
            eraser_popup_visible: false,
            more_menu_open: false,
            more_settings_entry: false,
            process_block_open: false,
            process_block_enabled: false,
            process_rules: Vec::new(),
            process_scan: Vec::new(),
            process_input: String::new(),
            process_input_active: false,
            process_scroll: 0,
            page_picker: None,
            settings_open: false,
            settings_page: 0,
            popup_on_right: false,
            press: None,
            open_anim: 0,
            confirm_quit: false,
            show_diag: false,
            diag_text: String::new(),
            show_input_group_prompt: false,
            input_group_state: 0,
            whiteboard: false,
            whiteboard_bg_index: 1,
            whiteboard_dots: false,
            whiteboard_dot_size: DEFAULT_WHITEBOARD_DOT_SIZE,
            saved_mode: 0,
            launch: crate::launch::Launch::default(),
            collapsed: false,
            drawing_tools: crate::toolbar::DrawingTools::default(),
            sidebar_drag: None,
            saved_drawing_tools: false,
            sb_y: ((screen_h - sb_height_of(DEFAULT_SB_SCALE)) / 2).max(0),
            sidebar_anim_start: None,
            sidebar_anim_from: 0.0,
            sidebar_collapsed_since: None,
            cur_color: 0,
            cur_pen: 1,
            cur_eraser: 1,
            is_drawing: false,
            last_pt: (0, 0),
            stroke_start_ms: 0,
            last_pen_w: 0,
            sb_scale: DEFAULT_SB_SCALE,
            sidebar_alpha: DEFAULT_SIDEBAR_ALPHA,
            theme: crate::ui_theme::Theme::default(),
            hotkey_ok: false,
            wps_fullscreen: false,
            wps_debug: true,
            wps_test_mode: true,
            startup_delay: DEFAULT_STARTUP_DELAY,
            wps_start_sent_until: 0,
            wps_connected: false,
            wps_real_pos: -1,
            wps_page_index: -1,
            wps_page_total: -1,
            wps_can_jump: false,
            wps_deck_epoch: -1,
            wps_navigation_ready: false,
            presentation_action: crate::ui::PresentationAction::Escape,
            presentation_transition: crate::toolbar::PresentationTransition::default(),
            touch_frame_contacts: 0,
            slide_cache: PageCache::default(),
            whiteboard_cache: PageCache::default(),
            current_slide: 1,
            saved_slide: 1,
            whiteboard_slide: 1,
            page_has_ink: false,
            wb_limit_msg_until: 0,
            palm_erase_preview: Vec::new(),
            palm_erase_w: PALM_ERASE_WIDTH,
            right_click_cursor_on: true,
            large_touch_threshold: DEFAULT_LARGE_TOUCH_THRESHOLD,
            large_erase_scale10: DEFAULT_LARGE_ERASE_SCALE10,
            t_prev_pos: HashMap::new(),
            t_prev_role: HashMap::new(),
            t_prev_preview: Vec::new(),
            t_palm_latched: false,
            t_moved: false,
            t_begin_pos: (0, 0),
            t_begin_role: 0,
            stroke_pts: Vec::new(),
            undo: VecDeque::new(),
            clear_history: crate::clear_history::ClearHistory::default(),
            canvas,
            screen_w,
            screen_h,
            scale: 1.0,
            log_cleared_until: 0,
        }
    }

    pub fn canvas_w(&self) -> u32 {
        ((self.screen_w as f64) * self.scale).round().max(1.0) as u32
    }
    pub fn canvas_h(&self) -> u32 {
        ((self.screen_h as f64) * self.scale).round().max(1.0) as u32
    }
    fn rebuild_canvas(&mut self) {
        self.canvas = fit_pixmap(&self.canvas, self.canvas_w(), self.canvas_h());
        self.stroke_pts.clear();
        self.is_drawing = false;
        self.reset_palm_gesture();
        self.clear_undo();
    }
    /// 设置设备缩放（改变时重建物理分辨率画布）
    pub fn set_scale(&mut self, s: f64) {
        if !s.is_finite() { return; }
        let s = s.clamp(1.0, 4.0);
        self.configure_geometry(self.screen_w, self.screen_h, s);
    }

    // ---- 侧边栏尺寸 ----
    pub fn sb_width(&self) -> i32 {
        (SB_BASE_W * self.sb_scale) as i32
    }
    pub fn sb_btn(&self) -> i32 {
        (SB_BASE_BTN * self.sb_scale) as i32
    }
    pub fn sb_icon(&self) -> i32 {
        (SB_BASE_ICON * self.sb_scale) as i32
    }
    pub fn sb_dot(&self) -> i32 {
        (SB_BASE_DOT * self.sb_scale) as i32
    }
    pub fn sb_height(&self) -> i32 {
        sb_compact_height_of(self.sb_scale) + self.drawing_tools_height()
            - self.sb_page_h() - 6 + self.sidebar_page_space()
    }
    /// Keep the visible toolbar on screen without losing its preferred anchor.
    /// Height changes (tools, page count, scale) share these bounds with dragging.
    pub fn sidebar_y(&self, height: i32) -> i32 {
        self.sb_y.clamp(0, (self.screen_h - height).max(0))
    }
    pub fn sidebar_page_visible(&self) -> bool {
        self.whiteboard || (self.wps_debug && self.wps_connected
            && (self.wps_page_index > 0 || self.wps_page_total > 0))
    }
    pub fn sidebar_page_space(&self) -> i32 {
        if self.sidebar_page_visible() { self.sb_page_h() + 6 } else { 0 }
    }
    pub fn page_picker_context(&self) -> Option<(i32, i32, crate::page_picker::Source)> {
        if self.whiteboard {
            Some((self.current_slide, CACHE_BOARD, crate::page_picker::Source::Whiteboard))
        } else if self.wps_debug && self.wps_connected && self.wps_can_jump
            && self.wps_deck_epoch > 0 && self.wps_page_index > 0 && self.wps_page_total >= self.wps_page_index {
            Some((self.wps_page_index, self.wps_page_total, crate::page_picker::Source::Wps(self.wps_deck_epoch)))
        } else { None }
    }
    pub fn drawing_tools_height(&self) -> i32 {
        (self.drawing_tools_full_height() as f32 * self.drawing_tools.fraction()).round() as i32
    }
    pub fn drawing_tools_full_height(&self) -> i32 {
        4 * (self.sb_btn() + 6)
    }
    pub fn sb_page_h(&self) -> i32 {
        page_counter_height(self.sb_scale)
    }
    pub fn collapsed_h(&self) -> i32 {
        (64.0 * self.sb_scale).round().clamp(56.0, 72.0) as i32
    }
    pub fn collapsed_w(&self) -> i32 {
        (24.0 * self.sb_scale).round().clamp(24.0, 30.0) as i32
    }
    pub fn sidebar_retract(&self) -> f32 {
        let target = if self.collapsed { 1.0 } else { 0.0 };
        let Some(start) = self.sidebar_anim_start else { return target };
        let t = (start.elapsed().as_secs_f32() / 0.18).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.sidebar_anim_from + (target - self.sidebar_anim_from) * eased
    }
    pub fn set_sidebar_collapsed(&mut self, collapsed: bool) {
        if self.collapsed == collapsed { return; }
        self.sidebar_anim_from = self.sidebar_retract();
        self.sidebar_anim_start = Some(std::time::Instant::now());
        self.collapsed = collapsed;
        self.sidebar_collapsed_since = collapsed.then(std::time::Instant::now);
    }

    /// 手动收起两分钟后自动展开，避免课堂中把工具栏忘在贴边状态。
    pub fn sidebar_expand_due(&self) -> bool {
        self.collapsed && self.sidebar_collapsed_since.is_some_and(
            |start| start.elapsed() >= std::time::Duration::from_secs(120))
    }
    pub fn finish_sidebar_animation(&mut self) -> bool {
        if self.sidebar_anim_start.is_some_and(|start| start.elapsed().as_millis() >= 180) {
            self.sidebar_anim_start = None;
            return true;
        }
        false
    }
    pub fn sb_x_left(&self) -> i32 {
        4
    }
    pub fn sb_x_right(&self) -> i32 {
        self.screen_w - self.sb_width() - 4
    }

    pub fn pen_color(&self) -> Color {
        let (r, g, b) = COLORS[self.cur_color];
        color(r, g, b, 255)
    }
    pub fn pen_width(&self) -> i32 {
        PEN_SIZES[self.cur_pen]
    }
    pub fn eraser_width(&self) -> i32 {
        ERASER_SIZES[self.cur_eraser]
    }
    pub fn whiteboard_bg_color(&self) -> Color {
        whiteboard_bg(self.whiteboard_bg_index)
    }

    /// A theme switch only invalidates UI artwork, never canvas/undo/page caches.
    pub fn apply_theme(&mut self, theme: crate::ui_theme::Theme) -> bool {
        let changed = self.theme.dark() != theme.dark() || self.theme.accent != theme.accent;
        self.theme = theme;
        if changed { self.launch.card.get_mut().take(); }
        changed
    }

    pub fn update_system_theme(&mut self, dark: bool) -> bool {
        self.apply_theme(crate::ui_theme::Theme { system_dark: dark, ..self.theme })
    }

    pub fn timed_pen_width(&self) -> i32 {
        let base = self.pen_width();
        if self.stroke_start_ms <= 0 {
            return base;
        }
        let minw = ((base as f64 * STROKE_MIN_SCALE).round() as i32).max(1);
        let dt = now_ms() - self.stroke_start_ms;
        let t = (dt as f64 / STROKE_RAMP_MS as f64).clamp(0.0, 1.0);
        minw + ((base - minw) as f64 * t).round() as i32
    }

    // ---- 撤回 ----
    pub fn clear_undo(&mut self) {
        self.undo.clear();
    }
    /// 开始一次新的撤回记录（落笔前调用）；不立即拷贝像素，避免整屏快照
    pub fn begin_undo(&mut self) {
        if self.undo.len() >= MAX_UNDO {
            self.undo.pop_front();
        }
        self.undo.push_back(UndoEntry::default());
    }
    /// 在绘制前保存 rect 覆盖到、且本条目尚未保存的瓦片。
    /// rect 使用画布物理像素坐标，右、下边界不包含在区域内。
    pub fn save_undo_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let cw = self.canvas.width() as i32;
        let ch = self.canvas.height() as i32;
        let x0 = x0.clamp(0, cw);
        let y0 = y0.clamp(0, ch);
        let x1 = x1.clamp(0, cw);
        let y1 = y1.clamp(0, ch);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let Some(entry) = self.undo.back_mut() else {
            return;
        };
        let tc0 = x0 / UNDO_TILE;
        let tr0 = y0 / UNDO_TILE;
        let tc1 = (x1 - 1) / UNDO_TILE;
        let tr1 = (y1 - 1) / UNDO_TILE;
        let data = self.canvas.data();
        for tr in tr0..=tr1 {
            for tc in tc0..=tc1 {
                if entry.tiles.contains_key(&(tc, tr)) {
                    continue;
                }
                let tx = tc * UNDO_TILE;
                let ty = tr * UNDO_TILE;
                let tw = UNDO_TILE.min(cw - tx) as usize;
                let th = UNDO_TILE.min(ch - ty) as usize;
                let mut buf = Vec::with_capacity(tw * th * 4);
                for row in 0..th {
                    let start = ((ty as usize + row) * cw as usize + tx as usize) * 4;
                    buf.extend_from_slice(&data[start..start + tw * 4]);
                }
                entry.tiles.insert((tc, tr), buf);
            }
        }
    }
    pub fn undo_last(&mut self) {
        let Some(entry) = self.undo.pop_back() else {
            return;
        };
        let cw = self.canvas.width() as i32;
        let ch = self.canvas.height() as i32;
        let data = self.canvas.data_mut();
        for (&(tc, tr), buf) in &entry.tiles {
            let tx = tc * UNDO_TILE;
            let ty = tr * UNDO_TILE;
            let tw = UNDO_TILE.min(cw - tx) as usize;
            let th = UNDO_TILE.min(ch - ty) as usize;
            for row in 0..th {
                let dst = ((ty as usize + row) * cw as usize + tx as usize) * 4;
                let src = row * tw * 4;
                data[dst..dst + tw * 4].copy_from_slice(&buf[src..src + tw * 4]);
            }
        }
    }

    // ---- 手势复位 ----
    pub fn reset_palm_gesture(&mut self) {
        self.t_prev_pos.clear();
        self.t_prev_role.clear();
        self.t_prev_preview.clear();
        self.t_palm_latched = false;
        self.t_moved = false;
        self.t_begin_role = 0;
        self.palm_erase_preview.clear();
        self.palm_erase_w = PALM_ERASE_WIDTH;
    }

    // ---- 画布 ----
    pub fn request_drawing_tools_collapse(&mut self) {
        if !self.whiteboard && self.mode == 0 {
            self.drawing_tools.request_auto_collapse();
        }
    }

    /// Actual page content for an automatic drawer update; None preserves a
    /// manual sidebar tuck, active drawing, or the whiteboard's own tool state.
    pub fn drawing_tools_auto_state(&mut self) -> Option<bool> {
        if self.whiteboard || self.collapsed || self.mode != 0
            || self.is_drawing || !self.stroke_pts.is_empty() {
            return None;
        }
        // A cleared page keeps the drawer open so its restore action stays reachable.
        if self.can_restore_cleared() { return Some(true); }
        // Erasing/undo can leave the conservative ink flag set. Inspect actual
        // alpha only at a requested transition, never on idle animation frames.
        self.page_has_ink = self.canvas.pixels().iter().any(|pixel| pixel.alpha() != 0);
        Some(self.page_has_ink)
    }

    pub fn clear_canvas(&mut self) {
        self.stroke_pts.clear();
        self.canvas.fill(Color::TRANSPARENT);
        self.page_has_ink = false;
        self.clear_undo();
    }

    pub fn can_restore_cleared(&self) -> bool {
        !self.page_has_ink && self.clear_history.available(self.current_slide, self.whiteboard)
    }

    /// 用户点“清除”前调用；空白页不记录，避免恢复出一张空快照。
    pub fn remember_before_clear(&mut self) {
        if !self.canvas.pixels().iter().any(|pixel| pixel.alpha() != 0) {
            return;
        }
        let snapshot = clone_pixmap(&self.canvas);
        self.clear_history.push(self.current_slide, self.whiteboard, snapshot);
    }

    pub fn restore_cleared(&mut self) -> bool {
        if self.page_has_ink {
            return false;
        }
        let Some(snapshot) = self.clear_history.take(self.current_slide, self.whiteboard) else {
            return false;
        };
        self.stroke_pts.clear();
        self.is_drawing = false;
        self.reset_palm_gesture();
        self.canvas = fit_pixmap(&snapshot, self.canvas_w(), self.canvas_h());
        self.page_has_ink = true;
        self.clear_undo();
        true
    }

    /// 显示变化时按画面比例保留笔迹；物理像素撤回记录不能跨尺寸复用。
    pub fn resize_screen(&mut self, w: i32, h: i32) {
        self.configure_geometry(w, h, self.scale);
    }

    pub fn configure_geometry(&mut self, w: i32, h: i32, scale: f64) {
        if !valid_geometry(w, h, scale) {
            return;
        }
        // Wayland 首次 configure 可能与默认逻辑尺寸相同，但画布仍是 1×1 占位图。
        if self.screen_w == w
            && self.screen_h == h
            && (self.scale - scale).abs() < 1e-3
            && self.canvas.width() == self.canvas_w()
            && self.canvas.height() == self.canvas_h()
        {
            return;
        }
        let resized = self.screen_w != w || self.screen_h != h;
        self.drawing_tools.settle();
        self.scale = scale;
        self.screen_w = w;
        self.screen_h = h;
        self.rebuild_canvas();
        let sbh = if self.collapsed { self.collapsed_h() } else { self.sb_height() };
        if resized { self.sb_y = ((h - sbh) / 2).clamp(0, (h - sbh).max(0)); }
    }

    // ---- 当前生效缓存 ----
    pub fn save_current_page(&mut self) {
        if !self.page_has_ink {
            // 清空后的空白页也要覆盖旧快照，否则重开/翻回该页会复活旧笔迹。
            if self.whiteboard {
                self.whiteboard_cache.remove(&self.current_slide);
            } else {
                self.slide_cache.remove(&self.current_slide);
            }
            crate::wps::wps_log(&format!("[PAGE] save 跳过 page={} (无笔迹)", self.current_slide));
            return;
        }
        crate::wps::wps_log(&format!("[PAGE] save page={}", self.current_slide));
        let slide = self.current_slide;
        if self.whiteboard {
            let snap = clone_pixmap(&self.canvas);
            self.whiteboard_cache.insert(slide, snap);
        } else {
            let snap = clone_pixmap(&self.canvas);
            self.slide_cache.insert(slide, snap);
        }
    }

    pub fn load_page(&mut self, page: i32) {
        let cached = if self.whiteboard {
            self.whiteboard_cache.contains_key(&page)
        } else {
            self.slide_cache.contains_key(&page)
        };
        crate::wps::wps_log(&format!("[PAGE] load page={} has_cache={}", page, cached));
        self.stroke_pts.clear();
        self.canvas.fill(Color::TRANSPARENT);
        let cached = if self.whiteboard {
            self.whiteboard_cache.get(&page)
        } else {
            self.slide_cache.get(&page)
        };
        if let Some(c) = cached {
            // 缓存保留原始分辨率；恢复时适配当前输出，不能把旧尺寸当作新画布。
            self.canvas = fit_pixmap(&c, self.canvas_w(), self.canvas_h());
            self.page_has_ink = true;
        } else {
            self.page_has_ink = false;
        }
        self.clear_undo();
        if !self.whiteboard { self.drawing_tools.request_page_update(); }
    }

    /// 放映开始/结束或全屏状态变化只重置透明批注，不影响独立白板。
    pub fn clear_slide_pages(&mut self) {
        crate::wps::wps_log("[PAGE] clear_slide_pages");
        self.slide_cache.clear();
        self.saved_slide = 1;
        if self.whiteboard { return; }
        self.clear_history.clear();
        self.current_slide = 1;
        self.is_drawing = false;
        self.clear_canvas();
        self.request_drawing_tools_collapse();
    }

    pub fn clear_current_strokes(&mut self) {
        self.stroke_pts.clear();
        if self.whiteboard {
            self.whiteboard_cache.remove(&self.current_slide);
        } else {
            self.slide_cache.remove(&self.current_slide);
        }
        self.canvas.fill(Color::TRANSPARENT);
        self.page_has_ink = false;
        self.clear_undo();
    }

    pub fn toggle_whiteboard(&mut self) {
        if self.is_drawing { crate::paint::commit_stroke(self); }
        self.is_drawing = false;
        self.stroke_pts.clear();
        self.reset_palm_gesture();
        if !self.whiteboard {
            self.save_current_page();
            self.saved_slide = self.current_slide;
            self.saved_mode = self.mode;
            self.saved_drawing_tools = self.drawing_tools.expanded();
            self.whiteboard = true;
            self.mode = 1;
            self.drawing_tools.set_expanded(true);
            self.current_slide = self.whiteboard_slide;
        } else {
            // 手动切回普通模式时丢弃白板全部笔迹，但保留透明批注页缓存。
            self.whiteboard_cache.clear();
            self.clear_history.clear_whiteboard();
            self.whiteboard_slide = 1;
            self.whiteboard = false;
            self.mode = self.saved_mode;
            self.drawing_tools.set_expanded(self.saved_drawing_tools);
            self.current_slide = self.saved_slide;
        }
        self.wb_limit_msg_until = 0;
        self.load_page(self.current_slide);
    }
}

pub fn sb_height_of(scale: f64) -> i32 {
    let btn = (SB_BASE_BTN * scale) as i32;
    22 + 12 * btn + 66 + page_counter_height(scale) + 6
}

fn sb_compact_height_of(scale: f64) -> i32 {
    sb_height_of(scale) - 4 * ((SB_BASE_BTN * scale) as i32 + 6)
}

fn page_counter_height(scale: f64) -> i32 {
    (28.0 * scale).round().clamp(24.0, 36.0) as i32
}

// ---------------- 配置持久化（遵循 XDG，兼容原配置） ----------------
fn config_file() -> Option<std::path::PathBuf> {
    dirs::config_dir().filter(|p| p.is_absolute()).map(|p| p.join("sidera/config"))
}

pub fn config_path() -> Option<std::path::PathBuf> {
    config_file()
}

pub fn launched_by_autostart() -> bool {
    std::env::args().any(|arg| arg == "--autostart")
        || std::env::var("SIDERA_AUTOSTART").as_deref() == Ok("1")
}

pub fn load_startup_delay() -> u32 {
    let path = config_file();
    let legacy = dirs::home_dir().map(|home| home.join(".config/sidera/config"));
    let content = path
        .and_then(|path| std::fs::read_to_string(path).ok())
        .or_else(|| legacy.and_then(|path| std::fs::read_to_string(path).ok()));
    let Some(content) = content else { return DEFAULT_STARTUP_DELAY; };
    for line in content.lines() {
        if let Some(value) = line.trim().strip_prefix("startupDelay=") {
            if let Ok(n) = value.trim().parse::<u32>() {
                return n.clamp(MIN_STARTUP_DELAY, MAX_STARTUP_DELAY);
            }
        }
    }
    DEFAULT_STARTUP_DELAY
}

pub fn load_settings(app: &mut App) {
    let Some(path) = config_file() else {
        crate::diag_log::warn("设置未加载：无法确定配置目录");
        return;
    };
    let (content, loaded_path) = match std::fs::read_to_string(&path) {
        Ok(content) => (content, path.clone()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Migrate on the next save without deleting the previous user's file.
            let Some(legacy) = dirs::home_dir().map(|p| p.join(".config/sidera/config")) else {
                crate::diag_log::info("设置未加载：配置文件不存在");
                return;
            };
            let Ok(content) = std::fs::read_to_string(&legacy) else {
                crate::diag_log::info("设置未加载：配置文件不存在");
                return;
            };
            (content, legacy)
        }
        Err(e) => {
            log::warn!("读取设置失败：{e}");
            crate::diag_log::warn(&format!("读取设置失败：{}：{e}", path.display()));
            return;
        }
    };
    parse_settings(app, &content);
    crate::diag_log::info(&format!(
        "设置已加载：{}（theme={} accent={}）",
        loaded_path.display(),
        app.theme.preference.key(),
        app.theme.accent.key()
    ));
}

fn parse_settings(app: &mut App, content: &str) {
    for line in content.lines() {
        let line = line.trim();
        let Some(eq) = line.find('=') else { continue };
        let k = &line[..eq];
        let v = &line[eq + 1..];
        match k {
            "theme" => {
                if let Some(preference) = crate::ui_theme::Preference::parse(v) {
                    app.apply_theme(crate::ui_theme::Theme { preference, ..app.theme });
                }
            }
            "wpsDebug" => app.wps_debug = v == "1",
            "themeAccent" => {
                if let Some(accent) = crate::ui_theme::Accent::parse(v) { app.theme.accent = accent; }
            }
            "wpsTestMode" => app.wps_test_mode = true,
            "startupDelay" => {
                if let Ok(n) = v.parse::<u32>() {
                    app.startup_delay = n.clamp(MIN_STARTUP_DELAY, MAX_STARTUP_DELAY);
                }
            }
            "sbScale" => {
                if let Ok(f) = v.parse::<f64>() {
                    if f.is_finite() { app.sb_scale = f.clamp(0.6, 1.4); }
                }
            }
            "sidebarAlpha" => {
                if let Ok(n) = v.parse::<i32>() {
                    app.sidebar_alpha = n.clamp(30, 255) as u8;
                }
            }
            "rightClickCursor" => app.right_click_cursor_on = v == "1",
            "whiteboardDots" => app.whiteboard_dots = v == "1",
            "whiteboardDotSize" => {
                if let Ok(n) = v.parse::<i32>() { app.whiteboard_dot_size = n.clamp(1, 10); }
            }
            "processBlock" => app.process_block_enabled = v == "1",
            "penColor" => {
                if let Ok(n) = v.parse::<usize>() { app.cur_color = n.min(COLORS.len() - 1); }
            }
            "penSize" => {
                if let Ok(n) = v.parse::<usize>() { app.cur_pen = n.min(PEN_SIZES.len() - 1); }
            }
            "eraserSize" => {
                if let Ok(n) = v.parse::<usize>() { app.cur_eraser = n.min(ERASER_SIZES.len() - 1); }
            }
            "largeTouchThreshold" => {
                if let Ok(n) = v.parse::<i32>() {
                    app.large_touch_threshold = n.clamp(20, 260) as f64;
                }
            }
            "largeEraseScale10" => {
                if let Ok(n) = v.parse::<i32>() {
                    app.large_erase_scale10 = n.clamp(8, 25);
                }
            }
            _ => {}
        }
    }
}

pub fn save_settings(app: &App) {
    // Unit tests exercise the real settings handlers; never let `cargo test`
    // rewrite the developer's live ~/.config/sidera/config (which previously
    // reset the saved theme accent).
    if cfg!(test) { return; }
    let Some(path) = config_file() else {
        log::warn!("无法确定设置目录");
        crate::diag_log::warn("保存设置失败：无法确定配置目录");
        return;
    };
    if let Err(e) = crate::persistence::atomic_write(&path, serialize_settings(app).as_bytes()) {
        log::warn!("保存设置失败：{e}");
        crate::diag_log::warn(&format!("保存设置失败：{}：{e}", path.display()));
    }
}

fn serialize_settings(app: &App) -> String {
    format!(
        "wpsDebug={}\nwpsTestMode={}\nstartupDelay={}\nsbScale={:.2}\nsidebarAlpha={}\nrightClickCursor={}\nwhiteboardDots={}\nwhiteboardDotSize={}\nprocessBlock={}\npenColor={}\npenSize={}\neraserSize={}\nlargeTouchThreshold={}\nlargeEraseScale10={}\ntheme={}\nthemeAccent={}\n",
        if app.wps_debug { 1 } else { 0 },
        1,
        app.startup_delay,
        app.sb_scale,
        app.sidebar_alpha,
        if app.right_click_cursor_on { 1 } else { 0 },
        if app.whiteboard_dots { 1 } else { 0 },
        app.whiteboard_dot_size,
        if app.process_block_enabled { 1 } else { 0 },
        app.cur_color,
        app.cur_pen,
        app.cur_eraser,
        app.large_touch_threshold as i32,
        app.large_erase_scale10,
        app.theme.preference.key(),
        app.theme.accent.key(),
    )
}

#[cfg(test)]
mod canvas_tests {
    use super::*;

    #[test]
    fn first_configure_repairs_placeholder_canvas_and_allows_edge_drawing() {
        for (cw, ch) in [(1, 1), (1920, 1), (1, 1080)] {
            let mut app = App::new(Pixmap::new(cw, ch).unwrap(), 1920, 1080);
            app.resize_screen(1920, 1080);
            app.set_scale(1.0);
            assert_eq!((app.canvas.width(), app.canvas.height()), (1920, 1080));

            app.begin_undo();
            crate::paint::stroke_tapered(&mut app, (1890, 1060), (1910, 1060), 6, 6);
            assert!(app.canvas.pixel(1900, 1060).unwrap().alpha() > 0);
            app.undo_last();
            assert!(app.canvas.data().iter().all(|&byte| byte == 0));
        }
    }

    #[test]
    fn repeated_configure_preserves_ink_undo_and_sidebar_position() {
        let mut app = App::new(Pixmap::new(320, 240).unwrap(), 320, 240);
        app.begin_undo();
        crate::paint::stroke_tapered(&mut app, (270, 220), (310, 220), 6, 6);
        app.sb_y = 37;
        let before = app.canvas.data().to_vec();

        app.resize_screen(320, 240);

        assert_eq!(app.canvas.data(), before.as_slice());
        assert!(app.page_has_ink);
        assert_eq!(app.sb_y, 37);
        assert_eq!(app.undo.len(), 1);
        app.undo_last();
        assert!(app.canvas.data().iter().all(|&byte| byte == 0));
    }

    #[test]
    fn configure_and_scale_in_either_order_produce_expected_canvas_size() {
        for (w, h, scale, expected) in [
            (1920, 1080, 1.0, (1920, 1080)),
            (1920, 1080, 1.25, (2400, 1350)),
            (1280, 720, 2.0, (2560, 1440)),
            (1919, 1079, 1.25, (2399, 1349)),
        ] {
            for scale_first in [false, true] {
                let mut app = App::new(Pixmap::new(1, 1).unwrap(), 1920, 1080);
                if scale_first {
                    app.set_scale(scale);
                    app.resize_screen(w, h);
                } else {
                    app.resize_screen(w, h);
                    app.set_scale(scale);
                }
                assert_eq!((app.screen_w, app.screen_h), (w, h));
                assert_eq!((app.canvas.width(), app.canvas.height()), expected);
            }
        }
    }

    #[test]
    fn invalid_configure_dimensions_leave_existing_canvas_untouched() {
        let mut app = App::new(Pixmap::new(320, 240).unwrap(), 320, 240);
        app.begin_undo();
        crate::paint::stroke_tapered(&mut app, (20, 20), (40, 20), 6, 6);
        let before = app.canvas.data().to_vec();

        for (w, h) in [(0, 240), (-1, 240), (320, 0), (320, -1)] {
            app.resize_screen(w, h);
            assert_eq!((app.screen_w, app.screen_h), (320, 240));
            assert_eq!(app.canvas.data(), before.as_slice());
            assert!(app.page_has_ink);
            assert_eq!(app.undo.len(), 1);
        }
    }
}

#[cfg(test)]
mod whiteboard_tests {
    use super::*;

    fn app_with_ink() -> App {
        let mut app = App::new(Pixmap::new(100, 80).unwrap(), 100, 80);
        app.current_slide = 7;
        draw(&mut app, 20);
        app
    }

    fn draw(app: &mut App, y: i32) {
        app.begin_undo();
        crate::paint::stroke_tapered(app, (20, y), (80, y), 6, 6);
    }

    #[test]
    fn switching_back_discards_whiteboard_ink_and_keeps_overlay() {
        let mut app = app_with_ink();
        let overlay = app.canvas.data().to_vec();
        app.toggle_whiteboard();
        assert_eq!(app.current_slide, 1);
        assert!(!app.page_has_ink);
        draw(&mut app, 40);
        app.save_current_page();
        app.current_slide = 2;
        app.load_page(2);
        draw(&mut app, 60);
        app.whiteboard_bg_index = 0;

        app.toggle_whiteboard();
        assert!(!app.whiteboard);
        assert_eq!(app.current_slide, 7);
        assert_eq!(app.canvas.data(), overlay.as_slice());
        assert!(app.whiteboard_cache.is_empty());
        assert_eq!(app.whiteboard_slide, 1);
        assert_eq!(app.whiteboard_bg_index, 0);

        // Re-entering starts from a blank whiteboard, not the previous session.
        app.toggle_whiteboard();
        assert!(app.whiteboard);
        assert_eq!(app.current_slide, 1);
        assert!(!app.page_has_ink);
        assert!(app.canvas.data().iter().all(|&b| b == 0));
        app.toggle_whiteboard();
        assert_eq!(app.canvas.data(), overlay.as_slice());
    }

    #[test]
    fn cleared_whiteboard_page_stays_blank_and_other_pages_survive() {
        for clear in [App::clear_canvas, App::clear_current_strokes] {
            let mut app = app_with_ink();
            app.toggle_whiteboard();
            draw(&mut app, 30);
            let first = app.canvas.data().to_vec();
            app.save_current_page();
            app.current_slide = 2;
            app.load_page(2);
            draw(&mut app, 50);
            clear(&mut app);
            assert_eq!(app.current_slide, 2);
            assert!(!app.page_has_ink);
            assert!(app.canvas.data().iter().all(|&b| b == 0));
            assert!(!app.whiteboard_cache.contains_key(&2));
            app.current_slide = 1;
            app.load_page(1);
            assert_eq!(app.canvas.data(), first.as_slice());
        }
    }

    #[test]
    fn slide_reset_preserves_visible_whiteboard() {
        let mut app = app_with_ink();
        app.toggle_whiteboard();
        app.current_slide = 3;
        draw(&mut app, 40);
        let board = app.canvas.data().to_vec();
        app.clear_slide_pages();
        assert!(app.slide_cache.is_empty());
        assert_eq!(app.current_slide, 3);
        assert_eq!(app.canvas.data(), board.as_slice());
        assert_eq!(app.undo.len(), 1);
    }

    #[test]
    fn pending_stroke_is_discarded_when_leaving_whiteboard() {
        let mut app = app_with_ink();
        let overlay = app.canvas.data().to_vec();
        app.toggle_whiteboard();
        app.current_slide = 3;
        app.is_drawing = true;
        app.stroke_pts = vec![(20.0, 40.0, 6.0), (80.0, 40.0, 6.0)];
        app.toggle_whiteboard();
        assert!(!app.is_drawing);
        assert!(app.stroke_pts.is_empty());
        assert!(!app.whiteboard);
        assert_eq!(app.current_slide, 7);
        assert_eq!(app.canvas.data(), overlay.as_slice());
        app.toggle_whiteboard();
        assert!(app.whiteboard);
        assert_eq!(app.current_slide, 1);
        assert!(!app.page_has_ink);
        assert!(app.canvas.data().iter().all(|&b| b == 0));
    }
}

#[cfg(test)]
mod clear_restore_tests {
    use super::*;

    fn draw(app: &mut App, y: i32) {
        app.begin_undo();
        crate::paint::stroke_tapered(app, (20, y), (80, y), 6, 6);
    }

    #[test]
    fn clearing_then_restoring_returns_the_exact_canvas_once() {
        let mut app = App::new(Pixmap::new(100, 80).unwrap(), 100, 80);
        app.current_slide = 3;
        draw(&mut app, 30);
        let before = app.canvas.data().to_vec();
        app.remember_before_clear();
        app.clear_current_strokes();
        assert!(app.can_restore_cleared());
        assert!(app.restore_cleared());
        assert_eq!(app.canvas.data(), before.as_slice());
        assert!(app.page_has_ink);
        assert!(!app.can_restore_cleared());
        assert!(!app.restore_cleared());
    }

    #[test]
    fn new_ink_after_clearing_disables_restore_until_the_next_clear() {
        let mut app = App::new(Pixmap::new(100, 80).unwrap(), 100, 80);
        draw(&mut app, 30);
        app.remember_before_clear();
        app.clear_current_strokes();
        draw(&mut app, 50);
        assert!(!app.can_restore_cleared());
        assert!(!app.restore_cleared());
        let drawn = app.canvas.data().to_vec();
        app.remember_before_clear();
        app.clear_current_strokes();
        assert!(app.restore_cleared());
        assert_eq!(app.canvas.data(), drawn.as_slice());
    }

    #[test]
    fn clearing_an_empty_page_keeps_the_previous_snapshot() {
        let mut app = App::new(Pixmap::new(100, 80).unwrap(), 100, 80);
        draw(&mut app, 30);
        let before = app.canvas.data().to_vec();
        app.remember_before_clear();
        app.clear_current_strokes();
        app.remember_before_clear(); // empty canvas must not replace the snapshot
        assert!(app.restore_cleared());
        assert_eq!(app.canvas.data(), before.as_slice());
    }

    #[test]
    fn restore_is_bound_to_its_page_and_whiteboard_mode() {
        let mut app = App::new(Pixmap::new(100, 80).unwrap(), 100, 80);
        app.current_slide = 3;
        draw(&mut app, 30);
        app.remember_before_clear();
        app.current_slide = 4;
        assert!(!app.can_restore_cleared());
        assert!(!app.restore_cleared());
        app.current_slide = 3;
        app.toggle_whiteboard();
        assert!(!app.can_restore_cleared());
        assert!(!app.restore_cleared());
    }

    #[test]
    fn whiteboard_clear_can_be_restored_without_reaching_overlay_ink() {
        let mut app = App::new(Pixmap::new(100, 80).unwrap(), 100, 80);
        draw(&mut app, 20);
        app.toggle_whiteboard();
        draw(&mut app, 50);
        let board = app.canvas.data().to_vec();
        app.remember_before_clear();
        app.clear_current_strokes();
        assert!(app.restore_cleared());
        assert!(app.whiteboard);
        assert_eq!(app.canvas.data(), board.as_slice());
    }
}

#[cfg(test)]
mod sidebar_idle_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn collapsed_sidebar_becomes_due_after_two_minutes_and_resets_on_expand() {
        let mut app = App::new(Pixmap::new(100, 100).unwrap(), 100, 100);
        app.set_sidebar_collapsed(true);
        assert!(!app.sidebar_expand_due());
        app.sidebar_collapsed_since = Some(Instant::now() - Duration::from_secs(120));
        assert!(app.sidebar_expand_due());
        app.set_sidebar_collapsed(false);
        assert!(!app.sidebar_expand_due());
        assert!(app.sidebar_collapsed_since.is_none());
    }
}

#[cfg(test)]
mod undo_tests {
    use super::*;
    use tiny_skia::Color;

    #[test]
    fn tiled_undo_restores_only_saved_tiles() {
        let mut pm = Pixmap::new(300, 300).unwrap();
        pm.fill(Color::TRANSPARENT);
        let mut app = App::new(pm, 300, 300);
        let red = Color::from_rgba8(255, 0, 0, 255);
        let blue = Color::from_rgba8(0, 0, 255, 255);
        app.canvas.fill(red);
        app.begin_undo();
        app.save_undo_rect(100, 100, 130, 130); // 覆盖瓦片 (0,0)
        app.canvas.fill(blue); // 之后整屏变蓝
        app.undo_last();
        let w = app.canvas.width() as usize;
        let px = |x: usize, y: usize| {
            let d = app.canvas.data();
            let i = (y * w + x) * 4;
            (d[i], d[i + 1], d[i + 2], d[i + 3])
        };
        assert_eq!(px(10, 10), (255, 0, 0, 255), "已保存瓦片应恢复为红");
        assert_eq!(px(280, 280), (0, 0, 255, 255), "未保存瓦片区域应保持蓝");
    }
}

#[cfg(test)]
mod theme_tests {
    use super::*;
    use crate::ui_theme::{Preference, Theme};

    #[test]
    fn preferences_round_trip_without_persisting_system_or_board_state() {
        let make = || App::new(Pixmap::new(120, 100).unwrap(), 120, 100);
        let mut legacy = make();
        parse_settings(&mut legacy, "sbScale=1.13\nsidebarAlpha=99\n");
        assert_eq!(legacy.theme.preference, Preference::Light);
        assert_eq!(legacy.whiteboard_bg_index, 1);
        for preference in [Preference::Light, Preference::Dark, Preference::System] {
            for background in 0..WHITEBOARD_COLORS.len() as i32 {
                let mut source = make();
                source.theme = Theme { preference, system_dark: true, ..Default::default() };
                source.whiteboard_bg_index = background;
                source.sb_scale = 1.13;
                let mut loaded = make();
                parse_settings(&mut loaded, &serialize_settings(&source));
                assert_eq!(loaded.theme.preference, preference);
                assert!(!loaded.theme.system_dark);
                assert_eq!(loaded.whiteboard_bg_index, 1);
                assert_eq!(loaded.sb_scale, source.sb_scale);
                assert!(!serialize_settings(&source).contains("whiteboardBackground"));
                // Ignore the withdrawn .32 option, including files saved by it.
                loaded.whiteboard_bg_index = background;
                for old_option in ["theme", "white", "green", "999"] {
                    parse_settings(&mut loaded, &format!("theme=unknown\nwhiteboardBackground={old_option}\n"));
                    assert_eq!(loaded.theme.preference, preference);
                    assert_eq!(loaded.whiteboard_bg_index, background);
                }
            }
        }
    }

    #[test]
    fn paint_tool_choices_roundtrip_through_settings() {
        let mut source = App::new(Pixmap::new(10, 10).unwrap(), 10, 10);
        source.cur_color = 5;
        source.cur_pen = 2;
        source.cur_eraser = 0;
        let mut loaded = App::new(Pixmap::new(10, 10).unwrap(), 10, 10);
        parse_settings(&mut loaded, &serialize_settings(&source));
        assert_eq!(loaded.cur_color, 5);
        assert_eq!(loaded.cur_pen, 2);
        assert_eq!(loaded.cur_eraser, 0);
    }

    #[test]
    fn startup_delay_roundtrips_and_clamps() {
        let mut source = App::new(Pixmap::new(10, 10).unwrap(), 10, 10);
        source.startup_delay = 15;
        let mut loaded = App::new(Pixmap::new(10, 10).unwrap(), 10, 10);
        parse_settings(&mut loaded, &serialize_settings(&source));
        assert_eq!(loaded.startup_delay, 15);
        for (value, expected) in [("1", 5), ("5", 5), ("10", 10), ("20", 20), ("30", 20), ("bad", 10)] {
            let mut app = App::new(Pixmap::new(10, 10).unwrap(), 10, 10);
            parse_settings(&mut app, &format!("startupDelay={value}\n"));
            assert_eq!(app.startup_delay, expected, "startupDelay={value}");
        }
    }

    #[test]
    fn theme_accent_roundtrips_through_settings() {
        for accent in [
            crate::ui_theme::Accent::Teal,
            crate::ui_theme::Accent::Blue,
            crate::ui_theme::Accent::Green,
            crate::ui_theme::Accent::Amber,
            crate::ui_theme::Accent::Violet,
            crate::ui_theme::Accent::Rose,
        ] {
            let mut source = App::new(Pixmap::new(10, 10).unwrap(), 10, 10);
            source.theme.accent = accent;
            let mut loaded = App::new(Pixmap::new(10, 10).unwrap(), 10, 10);
            parse_settings(&mut loaded, &serialize_settings(&source));
            assert_eq!(loaded.theme.accent, accent);
        }
    }

    #[test]
    fn switching_themes_preserves_ink_pages_undo_and_manual_board_colour() {
        let mut app = App::new(Pixmap::new(120, 100).unwrap(), 120, 100);
        app.begin_undo();
        crate::paint::stroke_tapered(&mut app, (20, 20), (90, 20), 8, 8);
        app.save_current_page();
        let ink = app.canvas.data().to_vec();
        let light_board = app.whiteboard_bg_color();
        app.launch.card.get_mut().replace(Pixmap::new(8, 8).unwrap());
        assert!(app.apply_theme(Theme { preference: Preference::Dark, system_dark: false, ..Default::default() }));
        assert!(app.launch.card.borrow().is_none());
        assert_eq!(app.whiteboard_bg_color(), light_board);
        assert_eq!(app.canvas.data(), ink);
        assert_eq!(app.slide_cache.get(&1).unwrap().data(), ink);
        assert_eq!(app.undo.len(), 1);
        app.toggle_whiteboard();
        crate::paint::stroke_tapered(&mut app, (20, 40), (90, 40), 8, 8);
        app.save_current_page();
        let board_ink = app.canvas.data().to_vec();
        app.whiteboard_bg_index = 0;
        let manual_board = app.whiteboard_bg_color();
        app.apply_theme(Theme { preference: Preference::System, system_dark: true, ..Default::default() });
        assert!(app.update_system_theme(false));
        assert_eq!(app.whiteboard_bg_color(), manual_board);
        assert_eq!(app.canvas.data(), board_ink);
        assert_eq!(app.whiteboard_cache.get(&1).unwrap().data(), board_ink);
        app.toggle_whiteboard();
        assert_eq!(app.canvas.data(), ink);
        app.apply_theme(Theme { preference: Preference::Light, system_dark: false, ..Default::default() });
        assert!(!app.update_system_theme(true));
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn resize_and_scale_preserve_ink_and_restore_cached_dimensions() {
        let mut canvas = Pixmap::new(20, 10).unwrap();
        canvas.fill(Color::from_rgba8(160, 20, 30, 255));
        let mut app = App::new(canvas, 20, 10);
        app.page_has_ink = true;
        app.slide_cache.insert(7, app.canvas.clone());
        app.configure_geometry(30, 20, 1.5);
        assert_eq!((app.canvas.width(), app.canvas.height()), (45, 30));
        assert!(app.page_has_ink);
        assert!(app.canvas.pixels().iter().all(|p| p.alpha() == 255));
        app.clear_canvas();
        app.load_page(7);
        assert_eq!((app.canvas.width(), app.canvas.height()), (45, 30));
        assert!(app.canvas.pixels().iter().all(|p| p.red() == 160));
        let cached = app.slide_cache.get(&7).unwrap();
        assert_eq!((cached.width(), cached.height()), (20, 10));
    }

    #[test]
    fn invalid_geometry_keeps_existing_canvas() {
        let mut app = App::new(Pixmap::new(20, 10).unwrap(), 20, 10);
        for geometry in [(0, 5, 1.0), (20, 10, f64::NAN), (i32::MAX, i32::MAX, 4.0)] {
            app.configure_geometry(geometry.0, geometry.1, geometry.2);
            assert_eq!((app.screen_w, app.screen_h, app.scale), (20, 10, 1.0));
            assert_eq!((app.canvas.width(), app.canvas.height()), (20, 10));
        }
    }
}
    #[test]
    fn invalid_settings_cannot_produce_nonfinite_toolbar_geometry() {
        let mut app = App::new(Pixmap::new(20, 20).unwrap(), 20, 20);
        for value in ["NaN", "inf", "-inf", "invalid"] {
            parse_settings(&mut app, &format!("sbScale={value}\n"));
            assert_eq!(app.sb_scale, DEFAULT_SB_SCALE);
        }
        parse_settings(&mut app, "sbScale=1.13\nlargeTouchThreshold=9999\nlargeEraseScale10=-9\n");
        assert_eq!(app.sb_scale, 1.13);
        assert_eq!(app.large_touch_threshold, 260.0);
        assert_eq!(app.large_erase_scale10, 8);
    }
