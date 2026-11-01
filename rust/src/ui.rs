// ============================================================
// Sidera - 手绘 UI（侧边栏 / 弹窗 / 设置面板）与命中测试
// 不再使用 QWidget：所有控件都是几何矩形 + tiny-skia 绘制
// ============================================================
use tiny_skia::{
    BlendMode, Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Shader, Stroke,
    Transform,
};

use crate::app::{color, App};
use crate::text::Fonts;
use crate::ui_icons::{self as icons, Glyph};
use crate::ui_theme::{self as theme, alpha, c};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct R {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl R {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Btn {
    Collapse,
    Cursor,
    Pen,
    Eraser,
    Clear,
    Restore,
    Undo,
    Prev,
    Next,
    Exit,
    Whiteboard,
    More,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PopupHit {
    Color(usize),
    PenSize(usize),
    EraserSize(usize),
    ClearAll,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MoreHit {
    Screenshot,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SetHit {
    Theme(theme::Preference),
    ResetSlider(SliderId),
    Autostart,
    WpsDebug,
    WpsRemove,
    WpsRegister,
    ProcessBlock,
    ThemeAccent,
    RightClick,
    ClearLog,
    SysInfo,
    Quit,
    Done,
    SettingsPage(i32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliderId {
    Alpha,
    Scale,
    Threshold,
    EraseScale,
}

impl SliderId {
    pub fn default_value(self) -> f32 {
        match self {
            Self::Alpha => crate::app::DEFAULT_SIDEBAR_ALPHA as f32,
            Self::Scale => crate::app::DEFAULT_SB_SCALE as f32,
            Self::Threshold => crate::app::DEFAULT_LARGE_TOUCH_THRESHOLD as f32,
            Self::EraseScale => crate::app::DEFAULT_LARGE_ERASE_SCALE10 as f32 / 10.0,
        }
    }
}

pub use crate::toolbar::PresentationAction;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PagePickerHit { Wheel, Cancel, Jump }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WhiteboardControl {
    Dots,
    DotSize,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Sidebar(Btn, bool), Popup(PopupHit), More(MoreHit), Setting(SetHit),
    Slider(SliderId), Confirm(ConfirmHit), DiagClose,
    PageCount(bool), PagePicker(PagePickerHit), Whiteboard(WhiteboardControl),
}

pub fn control_enabled(app: &App, control: Control) -> bool {
    if let Control::Setting(SetHit::SettingsPage(delta)) = control {
        let l = settings_layout(app);
        return (delta < 0 && l.page > 0) || (delta > 0 && l.page + 1 < l.pages);
    }
    if matches!(control, Control::PageCount(_)) { return app.page_picker_context().is_some(); }
    if matches!(control, Control::Sidebar(Btn::Restore, _)) { return app.can_restore_cleared(); }
    !(app.whiteboard && matches!(control, Control::Sidebar(Btn::Cursor, _)))
}

fn sidebar_button_selected(app: &App, button: Btn) -> bool {
    control_enabled(app, Control::Sidebar(button, false))
        && (matches!((button, app.mode), (Btn::Cursor, 0) | (Btn::Pen, 1) | (Btn::Eraser, 2))
            || (button == Btn::Whiteboard && app.whiteboard))
}

#[derive(Clone, Copy, Debug)]
pub struct SidebarDragFeedback {
    pub right: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct PressFeedback {
    pub control: Control,
    pub active: bool,
    pub until: Option<std::time::Instant>,
}

impl PressFeedback {
    pub fn new(control: Control) -> Self { Self { control, active: true, until: None } }
    pub fn expired(self) -> bool { self.until.is_some_and(|t| std::time::Instant::now() >= t) }
    pub fn release(&mut self) {
        // A quick tap must remain visible after the finger lifts.
        self.until = Some(std::time::Instant::now() + std::time::Duration::from_millis(160));
    }
}

// Painting and hit testing use the same sampled toolbar layout.
pub fn control_at(app: &App, x: i32, y: i32) -> Option<Control> {
    if app.launch.active() { return None; }
    if app.confirm_quit { return confirm_hit(app, x, y).map(Control::Confirm); }
    if app.show_diag { return diag_close_hit(app, x, y).then_some(Control::DiagClose); }
    if app.settings_open {
        return settings_slider_hit(app, x, y).map(|s| Control::Slider(s.id))
            .or_else(|| settings_hit(app, x, y).map(Control::Setting));
    }
    if app.more_menu_open { return more_menu_hit(app, x, y).map(Control::More); }
    if app.whiteboard {
        if let Some(hit) = whiteboard_control_hit(app, x, y) { return Some(Control::Whiteboard(hit)); }
    }
    if app.page_picker.is_some() && page_picker_rect(app).contains(x, y) {
        return page_picker_hit(app, x, y).map(Control::PagePicker);
    }
    if app.pen_popup_visible {
        if let Some(hit) = pen_popup_hit(app, x, y) { return Some(Control::Popup(hit)); }
    }
    if app.eraser_popup_visible {
        if let Some(hit) = eraser_popup_hit(app, x, y) { return Some(Control::Popup(hit)); }
    }
    page_count_at(app, x, y).map(Control::PageCount)
        .or_else(|| sidebar_button_at(app, x, y).map(|(b, right)| Control::Sidebar(b, right)))
}

pub fn control_rect(app: &App, control: Control) -> Option<R> {
    if app.confirm_quit && !matches!(control, Control::Confirm(_)) { return None; }
    if app.show_diag && control != Control::DiagClose { return None; }
    if app.settings_open && !matches!(control, Control::Setting(_) | Control::Slider(_) | Control::Confirm(_) | Control::DiagClose) { return None; }
    match control {
        Control::PageCount(right) if app.sidebar_page_visible() && !app.collapsed => Some(sidebar_page_rect(app, right)),
        Control::PagePicker(hit) if app.page_picker.is_some() => Some(page_picker_control_rect(app, hit)),
        Control::Sidebar(b, right) => sidebar_buttons(app, right).into_iter().find(|(k, _)| *k == b).map(|(_, r)| r),
        Control::Setting(hit) if app.settings_open => settings_controls(app).into_iter().find(|(k, _)| *k == hit).map(|(_, r)| r),
        Control::Slider(id) if app.settings_open => settings_sliders(app).into_iter().find(|s| s.id == id).map(|s| s.hit),
        Control::More(hit) if app.more_menu_open => {
            let m = more_menu_rect(app);
            let offset = if hit == MoreHit::Screenshot { 0 } else { 48 };
            Some(R { x: m.x, y: m.y + offset, w: m.w, h: if offset == 0 { 48 } else { m.h - 48 } })
        }
        Control::Whiteboard(WhiteboardControl::Dots) if app.whiteboard => Some(whiteboard_dots_toggle_rect(app)),
        Control::Whiteboard(WhiteboardControl::DotSize) if app.whiteboard && app.whiteboard_dots => Some(whiteboard_dot_slider_rect(app)),
        Control::Popup(PopupHit::Color(i)) if app.pen_popup_visible && i < 10 => {
            let p = pen_popup_rect(app);
            Some(R { x: p.x + 16 + (i % 4) as i32 * 48, y: p.y + 46 + (i / 4) as i32 * 48, w: 38, h: 38 })
        }
        Control::Popup(PopupHit::PenSize(i)) if app.pen_popup_visible && i < 3 => {
            let p = pen_popup_rect(app); let bw = (p.w - 52) / 3;
            Some(R { x: p.x + 16 + i as i32 * (bw + 10), y: p.y + 214, w: bw, h: 48 })
        }
        Control::Popup(PopupHit::EraserSize(i)) if app.eraser_popup_visible && i < 3 => {
            let p = eraser_popup_rect(app); let bw = (p.w - 52) / 3;
            Some(R { x: p.x + 16 + i as i32 * (bw + 10), y: p.y + 46, w: bw, h: 48 })
        }
        Control::Popup(PopupHit::ClearAll) if app.eraser_popup_visible => {
            let p = eraser_popup_rect(app);
            Some(R { x: p.x + 16, y: p.y + 110, w: p.w - 32, h: 48 })
        }
        Control::Confirm(hit) if app.confirm_quit => {
            let p = confirm_rect(app); let half = (p.w - 60) / 2;
            Some(R { x: p.x + 20 + if hit == ConfirmHit::Yes { half + 20 } else { 0 }, y: p.y + 92, w: half, h: 40 })
        }
        Control::DiagClose if app.show_diag => {
            let p = diag_rect(app);
            Some(R { x: p.x + 20, y: p.y + p.h - 52, w: p.w - 40, h: 38 })
        }
        _ => None,
    }
}

fn draw_press_feedback(app: &App, ctx: &mut Ctx) {
    let palette = ctx.palette;
    let Some(press) = app.press.filter(|p| p.active && !p.expired()) else { return };
    if !control_enabled(app, press.control) { return; }
    if sidebar_animating(app) && matches!(press.control, Control::Sidebar(_, _)) { return; }
    let Some(r) = control_rect(app, press.control) else { return };
    if matches!(press.control, Control::Slider(_) | Control::PagePicker(PagePickerHit::Wheel)) { return; }
    let radius = match press.control {
        Control::Popup(PopupHit::Color(_)) => 19.0,
        Control::Sidebar(_, _) => (r.w as f32 * 0.24).min(10.0),
        _ => 9.0,
    };
    let inset = R { x: r.x + 2, y: r.y + 2, w: r.w - 4, h: r.h - 4 };
    let filled_action = matches!(press.control, Control::Setting(SetHit::Done)
        | Control::DiagClose | Control::PagePicker(PagePickerHit::Jump))
        || matches!(press.control, Control::Setting(SetHit::Theme(p)) if p == app.theme.preference);
    let feedback = if filled_action || matches!(press.control, Control::Sidebar(button, _) if sidebar_button_selected(app, button)) {
        palette.on_accent
    } else {
        palette.accent
    };
    ctx.fill_round(inset, radius, alpha(feedback, 32));
    ctx.stroke_round(inset, radius, c(feedback), 2.0);
}

#[derive(Clone, Copy, Debug)]
pub struct SliderDef {
    pub id: SliderId,
    pub track: R, // 轨道（画 + 取值映射）
    pub hit: R,   // 轨道命中区域；不包含数值和恢复默认按钮
    pub badge: R,
    pub reset: R,
    pub min: f32,
    pub max: f32,
    pub value: f32,
    pub step: f32,
    pub label: &'static str,
    pub signed: bool, // 是否显示一位小数
}

impl SliderDef {
    pub fn t(&self) -> f32 {
        if self.max <= self.min {
            return 0.0;
        }
        ((self.value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }
    pub fn value_at(&self, x: i32) -> f32 {
        let t = ((x - self.track.x) as f32 / self.track.w.max(1) as f32).clamp(0.0, 1.0);
        let v = self.min + t * (self.max - self.min);
        (v / self.step).round() * self.step
    }
}

fn anim_t(app: &App) -> f32 {
    let el = (crate::app::now_ms() - app.open_anim) as f32;
    (el / 160.0).clamp(0.0, 1.0)
}

fn solid(c: Color) -> Paint<'static> {
    Paint {
        shader: Shader::SolidColor(c),
        anti_alias: true,
        blend_mode: BlendMode::SourceOver,
        ..Default::default()
    }
}

fn rounded_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

pub struct Ctx<'a> {
    pub pm: &'a mut Pixmap,
    pub tr: Transform,
    pub palette: theme::Palette,
}

impl<'a> Ctx<'a> {
    pub fn fill_round(&mut self, r: R, radius: f32, c: Color) {
        if let Some(p) = rounded_path(r.x as f32, r.y as f32, r.w as f32, r.h as f32, radius) {
            self.pm
                .fill_path(&p, &solid(c), FillRule::Winding, self.tr, None);
        }
    }
    pub fn stroke_round(&mut self, r: R, radius: f32, c: Color, width: f32) {
        if let Some(p) = rounded_path(r.x as f32, r.y as f32, r.w as f32, r.h as f32, radius) {
            let st = Stroke {
                width,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Default::default()
            };
            self.pm
                .stroke_path(&p, &solid(c), &st, self.tr, None);
        }
    }
    pub fn fill_circle(&mut self, cx: f32, cy: f32, rad: f32, c: Color) {
        let mut pb = PathBuilder::new();
        pb.push_circle(cx, cy, rad);
        if let Some(p) = pb.finish() {
            self.pm
                .fill_path(&p, &solid(c), FillRule::Winding, self.tr, None);
        }
    }
    pub fn stroke_line(&mut self, a: (f32, f32), b: (f32, f32), c: Color, w: f32) {
        let st = Stroke {
            width: w,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Default::default()
        };
        let mut pb = PathBuilder::new();
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        if let Some(p) = pb.finish() {
            self.pm.stroke_path(&p, &solid(c), &st, self.tr, None);
        }
    }
    pub fn text(&mut self, fonts: &Fonts, s: &str, x: f32, baseline: f32, px: f32, c: Color) {
        fonts.draw(self.pm, s, x, baseline, px, c, self.tr);
    }
    pub fn text_center(&mut self, fonts: &Fonts, r: R, s: &str, px: f32, c: Color) {
        let w = fonts.text_width(s, px);
        let x = r.x as f32 + (r.w as f32 - w) / 2.0;
        let baseline = r.y as f32 + r.h as f32 / 2.0 + px * 0.35;
        self.text(fonts, s, x, baseline, px, c);
    }
    pub fn text_multiline_center(&mut self, fonts: &Fonts, r: R, s: &str, px: f32, c: Color) {
        let lines: Vec<&str> = s.split('\n').collect();
        let line_h = px * 1.15;
        let total = line_h * lines.len() as f32;
        let mut y = r.y as f32 + (r.h as f32 - total) / 2.0 + px * 0.9;
        for line in lines {
            let w = fonts.text_width(line, px);
            let x = r.x as f32 + (r.w as f32 - w) / 2.0;
            self.text(fonts, line, x, y, px, c);
            y += line_h;
        }
    }
}

// Draw decoration inside the existing bounds: partial redraws must not leave
// shadows behind when a popup closes or a toolbar moves.
fn surface(ctx: &mut Ctx, r: R, radius: f32, fill: Color, border: Color) {
    ctx.fill_round(r, radius, fill);
    ctx.stroke_round(R { x: r.x + 1, y: r.y + 1, w: r.w - 2, h: r.h - 2 },
        (radius - 1.0).max(1.0), border, 1.0);
}

fn panel(ctx: &mut Ctx, r: R, radius: f32, opacity: f32) {
    let palette = ctx.palette;
    let a = (255.0 * opacity.clamp(0.0, 1.0)) as u8;
    surface(ctx, r, radius, alpha(palette.paper, a), alpha(palette.border, a));
}

fn choice(ctx: &mut Ctx, r: R, selected: bool) {
    let palette = ctx.palette;
    surface(ctx, r, 10.0, c(if selected { palette.accent_light } else { palette.surface }),
        c(if selected { palette.accent } else { palette.border }));
}

// ---------------- 几何 ----------------
pub fn launch_rect(app: &App) -> R {
    let factor = ((app.screen_w - 24).max(1) as f32 / crate::launch::CARD_WIDTH as f32)
        .min((app.screen_h - 24).max(1) as f32 / crate::launch::CARD_HEIGHT as f32).min(1.0);
    let w = (crate::launch::CARD_WIDTH as f32 * factor).round().max(1.0) as i32;
    let h = (crate::launch::CARD_HEIGHT as f32 * factor).round().max(1.0) as i32;
    R { x: (app.screen_w - w) / 2, y: (app.screen_h - h) / 2, w, h }
}

pub fn sidebar_launch_rect(app: &App, right: bool) -> R {
    let target = sidebar_rect(app, right);
    let p = app.launch.reveal();
    let h = if p <= 0.0 { 0 } else { (16.0 + (target.h - 16) as f32 * p).round() as i32 };
    R { y: target.y + (target.h - h) / 2, h, ..target }
}

pub fn sidebar_rect(app: &App, right: bool) -> R {
    if app.collapsed { return sidebar_tab_rect(app, right); }
    expanded_sidebar_rect(app, right)
}

fn sidebar_tab_rect(app: &App, right: bool) -> R {
    let w = app.collapsed_w();
    let h = app.collapsed_h();
    R { x: if right { app.screen_w - w } else { 0 }, y: app.sidebar_y(h), w, h }
}

fn expanded_sidebar_rect(app: &App, right: bool) -> R {
    let x = if right { app.sb_x_right() } else { app.sb_x_left() };
    R {
        x,
        y: app.sidebar_y(app.sb_height()),
        w: app.sb_width(),
        h: app.sb_height(),
    }
}

// Cover only the edge strip while moving, then release the hidden toolbar area.
pub fn sidebar_transition_rect(app: &App, right: bool) -> R {
    let w = app.sb_width() + 4;
    let full = crate::app::sb_height_of(app.sb_scale);
    // Include every position between the full toolbar and its tucked tab.
    // Near the bottom, expanding tools rise into view instead of being clipped.
    let y = app.sidebar_y(full);
    let bottom = (y + full).max(app.sidebar_y(app.collapsed_h()) + app.collapsed_h()).min(app.screen_h);
    R { x: if right { app.screen_w - w } else { 0 }, y, w, h: bottom - y }
}

pub fn sidebar_animating(app: &App) -> bool {
    app.sidebar_anim_start.is_some() || app.drawing_tools.animating()
}

pub fn sidebar_input_rect(app: &App, right: bool) -> R {
    if sidebar_animating(app) { sidebar_transition_rect(app, right) }
    else { sidebar_rect(app, right) }
}

/// 命中检测：点 (x,y) 落在哪个侧栏按钮上
pub fn sidebar_button_at(app: &App, x: i32, y: i32) -> Option<(Btn, bool)> {
    if sidebar_animating(app) { return None; }
    for right in [false, true] {
        for (k, r) in sidebar_buttons(app, right) {
            if r.contains(x, y) {
                return Some((k, right));
            }
        }
    }
    None
}

pub fn sidebar_buttons(app: &App, right: bool) -> Vec<(Btn, R)> {
    let sb = sidebar_rect(app, right);
    if app.collapsed {
        return vec![(
            Btn::Collapse,
            R {
                x: sb.x,
                y: sb.y + 6,
                w: sb.w,
                h: sb.h - 12,
            },
        )];
    }
    expanded_sidebar_buttons(app, right)
}

fn expanded_sidebar_buttons(app: &App, right: bool) -> Vec<(Btn, R)> {
    let sb = expanded_sidebar_rect(app, right);
    let btn = app.sb_btn();
    let bx = sb.x + (sb.w - btn) / 2;
    let mut y = sb.y + 12;
    let mut v = Vec::new();
    for k in [Btn::Collapse, Btn::Cursor, Btn::Pen] {
        v.push((k, R { x: bx, y, w: btn, h: btn }));
        y += btn + 6;
    }
    if app.drawing_tools.fraction() >= 1.0 { v.extend(drawing_tool_buttons(app, right)); }
    y += app.drawing_tools_height();
    for k in [Btn::Prev, Btn::Next, Btn::Exit, Btn::Whiteboard, Btn::More] {
        v.push((k, R { x: bx, y, w: btn, h: btn }));
        y += btn + 6;
        if k == Btn::Prev { y += app.sidebar_page_space(); }
    }
    v
}

fn drawing_tool_buttons(app: &App, right: bool) -> Vec<(Btn, R)> {
    let sb = expanded_sidebar_rect(app, right);
    let btn = app.sb_btn();
    [Btn::Eraser, Btn::Clear, Btn::Restore, Btn::Undo].into_iter().enumerate().map(|(i, k)|
        (k, R { x: sb.x + (sb.w - btn) / 2, y: sb.y + 12 + (3 + i as i32) * (btn + 6),
            w: btn, h: btn })).collect()
}

pub fn sidebar_page_rect(app: &App, right: bool) -> R {
    let sb = expanded_sidebar_rect(app, right);
    R { x: sb.x + 4, y: sb.y + 12 + 4 * (app.sb_btn() + 6) + app.drawing_tools_height(),
        w: sb.w - 8, h: app.sb_page_h() }
}

pub fn sidebar_page_values(app: &App) -> (String, String) {
    let (page, total) = if app.whiteboard {
        (app.current_slide, crate::app::CACHE_BOARD)
    } else if app.wps_debug && app.wps_connected {
        (app.wps_page_index, app.wps_page_total)
    } else { (-1, -1) };
    let value = |n: i32| if n > 0 { n.to_string() } else { "—".to_string() };
    (value(page), value(total))
}

pub fn page_count_at(app: &App, x: i32, y: i32) -> Option<bool> {
    if app.collapsed || !app.sidebar_page_visible() || sidebar_animating(app) { return None; }
    [false, true].into_iter().find(|right| sidebar_page_rect(app, *right).contains(x, y))
}

pub fn page_picker_rect(app: &App) -> R {
    let (w, h) = (260, 376);
    let sb = sidebar_rect(app, app.popup_on_right);
    let anchor = sidebar_page_rect(app, app.popup_on_right);
    R { x: (if app.popup_on_right { sb.x - w - 8 } else { sb.x + sb.w + 8 })
            .clamp(0, (app.screen_w - w).max(0)),
        y: (anchor.y + anchor.h / 2 - h / 2).clamp(0, (app.screen_h - h).max(0)), w, h }
}

pub fn page_picker_control_rect(app: &App, hit: PagePickerHit) -> R {
    let p = page_picker_rect(app);
    match hit {
        PagePickerHit::Wheel => R { x: p.x + 16, y: p.y + 74, w: p.w - 32, h: 220 },
        PagePickerHit::Cancel => R { x: p.x + 16, y: p.y + 314, w: 92, h: 44 },
        PagePickerHit::Jump => R { x: p.x + 118, y: p.y + 314, w: 126, h: 44 },
    }
}

pub fn page_picker_hit(app: &App, x: i32, y: i32) -> Option<PagePickerHit> {
    [PagePickerHit::Wheel, PagePickerHit::Cancel, PagePickerHit::Jump].into_iter()
        .find(|hit| page_picker_control_rect(app, *hit).contains(x, y))
}

pub fn pen_popup_rect(app: &App) -> R {
    let w = 260;
    let h = 286;
    let sb = sidebar_rect(app, app.popup_on_right);
    let x = if app.popup_on_right {
        sb.x - w - 8
    } else {
        sb.x + sb.w + 8
    };
    let y = sb.y + (sb.h - h) / 2;
    R {
        x: x.clamp(0, (app.screen_w - w).max(0)),
        y: y.clamp(0, (app.screen_h - h).max(0)),
        w,
        h,
    }
}

pub fn eraser_popup_rect(app: &App) -> R {
    let w = 280;
    let h = 184;
    let sb = sidebar_rect(app, app.popup_on_right);
    let x = if app.popup_on_right {
        sb.x - w - 8
    } else {
        sb.x + sb.w + 8
    };
    let y = sb.y + (sb.h - h) / 2;
    R {
        x: x.clamp(0, (app.screen_w - w).max(0)),
        y: y.clamp(0, (app.screen_h - h).max(0)),
        w,
        h,
    }
}

pub fn more_menu_rect(app: &App) -> R {
    let sb = sidebar_rect(app, app.popup_on_right);
    let w = 170;
    let h = if app.more_settings_entry { 104 } else { 56 };
    let x = if app.popup_on_right {
        sb.x - w - 8
    } else {
        sb.x + sb.w + 8
    };
    let more_y = sidebar_buttons(app, app.popup_on_right)
        .iter()
        .find(|(k, _)| *k == Btn::More)
        .map(|(_, r)| r.y)
        .unwrap_or(sb.y);
    let y = more_y.clamp(0, (app.screen_h - h).max(0));
    R {
        x: x.clamp(0, (app.screen_w - w).max(0)),
        y,
        w,
        h,
    }
}

pub fn pen_popup_hit(app: &App, x: i32, y: i32) -> Option<PopupHit> {
    let p = pen_popup_rect(app);
    if !p.contains(x, y) {
        return None;
    }
    let start_y = p.y + 46;
    for i in 0..10 {
        let row = (i / 4) as i32;
        let col = (i % 4) as i32;
        let cx = p.x + 16 + col * 48 + 19;
        let cy = start_y + row * 48 + 19;
        let rr = R {
            x: cx - 19,
            y: cy - 19,
            w: 38,
            h: 38,
        };
        if rr.contains(x, y) {
            return Some(PopupHit::Color(i));
        }
    }
    let sy = p.y + 214;
    let bw = (p.w - 32 - 20) / 3;
    for i in 0..3 {
        let bx = p.x + 16 + i as i32 * (bw + 10);
        let rr = R { x: bx, y: sy, w: bw, h: 48 };
        if rr.contains(x, y) {
            return Some(PopupHit::PenSize(i as usize));
        }
    }
    None
}

pub fn eraser_popup_hit(app: &App, x: i32, y: i32) -> Option<PopupHit> {
    let p = eraser_popup_rect(app);
    if !p.contains(x, y) {
        return None;
    }
    let sy = p.y + 46;
    let bw = (p.w - 32 - 20) / 3;
    for i in 0..3 {
        let bx = p.x + 16 + i as i32 * (bw + 10);
        let rr = R { x: bx, y: sy, w: bw, h: 48 };
        if rr.contains(x, y) {
            return Some(PopupHit::EraserSize(i as usize));
        }
    }
    let cy = p.y + 110;
    let rr = R { x: p.x + 16, y: cy, w: p.w - 32, h: 48 };
    if rr.contains(x, y) {
        return Some(PopupHit::ClearAll);
    }
    None
}

pub fn more_menu_hit(app: &App, x: i32, y: i32) -> Option<MoreHit> {
    let m = more_menu_rect(app);
    if !m.contains(x, y) {
        return None;
    }
    if y < m.y + 48 {
        Some(MoreHit::Screenshot)
    } else if app.more_settings_entry {
        Some(MoreHit::Settings)
    } else {
        None
    }
}

pub fn whiteboard_bar_rect(app: &App) -> R {
    let w = 340.min((app.screen_w - 96).max(240));
    R {
        x: (app.screen_w - w) / 2,
        y: app.screen_h - 66,
        w,
        h: 54,
    }
}

pub fn whiteboard_dots_toggle_rect(app: &App) -> R {
    let b = whiteboard_bar_rect(app);
    R {
        x: b.x + 12,
        y: b.y + 8,
        w: 116,
        h: 38,
    }
}

pub fn whiteboard_dot_slider_rect(app: &App) -> R {
    let b = whiteboard_bar_rect(app);
    R {
        x: b.x + 142,
        y: b.y + 14,
        w: (b.w - 158).max(80),
        h: 26,
    }
}

pub fn whiteboard_control_hit(app: &App, x: i32, y: i32) -> Option<WhiteboardControl> {
    if !app.whiteboard || !whiteboard_bar_rect(app).contains(x, y) {
        return None;
    }
    if whiteboard_dots_toggle_rect(app).contains(x, y) {
        return Some(WhiteboardControl::Dots);
    }
    if app.whiteboard_dots && whiteboard_dot_slider_rect(app).contains(x, y) {
        return Some(WhiteboardControl::DotSize);
    }
    None
}

pub fn whiteboard_dot_size_at(app: &App, x: i32) -> i32 {
    let r = whiteboard_dot_slider_rect(app);
    let t = ((x - r.x) as f32 / r.w.max(1) as f32).clamp(0.0, 1.0);
    (1.0 + t * 9.0).round() as i32
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProcessHit {
    Enable,
    Input,
    Add,
    Paste,
    Scan,
    Done,
    ScrollUp,
    ScrollDown,
    Rule(usize),
    Proc(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProcessRow {
    Rule(usize),
    Proc(usize),
}


pub fn process_block_rect(app: &App) -> R {
    let w = 620.min((app.screen_w - 24).max(320));
    let h = 560.min((app.screen_h - 24).max(320));
    R {
        x: (app.screen_w - w) / 2,
        y: (app.screen_h - h) / 2,
        w,
        h,
    }
}

fn process_row_height() -> i32 {
    34
}

pub fn process_visible_rows(app: &App) -> usize {
    ((process_block_rect(app).h - 304) / process_row_height()).max(1) as usize
}

pub fn process_rows(app: &App) -> Vec<(ProcessRow, String, String)> {
    let mut rows = Vec::new();
    for (index, rule) in app.process_rules.iter().enumerate() {
        rows.push((ProcessRow::Rule(index), rule.clone(), "点击移除".into()));
    }
    for (index, proc) in app.process_scan.iter().enumerate() {
        let added = app
            .process_rules
            .iter()
            .any(|rule| crate::process_block::matches(rule, proc));
        let detail = if added {
            format!("已添加 · {}", proc.exe)
        } else {
            format!("pid {} · {}", proc.pid, proc.exe)
        };
        rows.push((ProcessRow::Proc(index), proc.name.clone(), detail));
    }
    rows
}

pub fn process_block_controls(app: &App) -> Vec<(ProcessHit, R)> {
    let p = process_block_rect(app);
    let mut controls = vec![
        (
            ProcessHit::Enable,
            R {
                x: p.x + 20,
                y: p.y + 60,
                w: 220,
                h: 36,
            },
        ),
        (
            ProcessHit::Input,
            R {
                x: p.x + 20,
                y: p.y + 110,
                w: p.w - 190,
                h: 38,
            },
        ),
        (
            ProcessHit::Add,
            R {
                x: p.x + p.w - 158,
                y: p.y + 110,
                w: 64,
                h: 38,
            },
        ),
        (
            ProcessHit::Paste,
            R {
                x: p.x + p.w - 88,
                y: p.y + 110,
                w: 68,
                h: 38,
            },
        ),
        (
            ProcessHit::Scan,
            R {
                x: p.x + 20,
                y: p.y + 160,
                w: 112,
                h: 34,
            },
        ),
        (
            ProcessHit::ScrollUp,
            R {
                x: p.x + p.w - 126,
                y: p.y + 160,
                w: 48,
                h: 34,
            },
        ),
        (
            ProcessHit::ScrollDown,
            R {
                x: p.x + p.w - 72,
                y: p.y + 160,
                w: 52,
                h: 34,
            },
        ),
        (
            ProcessHit::Done,
            R {
                x: p.x + p.w - 118,
                y: p.y + p.h - 54,
                w: 98,
                h: 36,
            },
        ),
    ];
    let rows = process_rows(app);
    let start = app.process_scroll.min(rows.len().saturating_sub(1));
    let visible = process_visible_rows(app);
    for (offset, (row, _, _)) in rows.iter().enumerate().skip(start).take(visible) {
        let hit = match row {
            ProcessRow::Rule(i) => ProcessHit::Rule(*i),
            ProcessRow::Proc(i) => ProcessHit::Proc(*i),
        };
        controls.push((
            hit,
            R {
                x: p.x + 20,
                y: p.y + 206 + (offset - start) as i32 * process_row_height(),
                w: p.w - 40,
                h: process_row_height() - 4,
            },
        ));
    }
    controls
}

pub fn process_block_hit(app: &App, x: i32, y: i32) -> Option<ProcessHit> {
    if !app.process_block_open || !process_block_rect(app).contains(x, y) {
        return None;
    }
    process_block_controls(app)
        .into_iter()
        .find_map(|(hit, rect)| rect.contains(x, y).then_some(hit))
}

pub fn settings_rect(app: &App) -> R {
    let w = 460.min((app.screen_w - 16).max(1));
    let stride = if w < 420 { 72 } else { 48 };
    let h = (152 + settings_row_count(app) as i32 * stride).min((app.screen_h - 16).max(1));
    R { x: (app.screen_w - w) / 2, y: (app.screen_h - h) / 2, w, h }
}

#[derive(Clone, Copy, Debug)]
pub struct SettingsLayout {
    pub rows: usize,
    pub page: usize,
    pub pages: usize,
    pub stride: i32,
}

fn settings_row_count(app: &App) -> usize { (settings_action_start(app) + 3) as usize }

pub fn settings_layout(app: &App) -> SettingsLayout {
    let p = settings_rect(app);
    let stride = if p.w < 420 { 72 } else { 48 };
    let rows = ((p.h - 152) / stride).max(1) as usize;
    let pages = settings_row_count(app).div_ceil(rows);
    SettingsLayout { rows, page: app.settings_page.min(pages - 1), pages, stride }
}

pub fn settings_change_page(app: &mut App, delta: i32) -> bool {
    let l = settings_layout(app);
    let next = (l.page as i64 + i64::from(delta)).clamp(0, l.pages as i64 - 1) as usize;
    app.settings_page = next;
    if next == l.page { return false; }
    app.press = None;
    true
}

fn settings_row_visible(app: &App, row: i32) -> bool {
    let l = settings_layout(app);
    (l.page * l.rows..(l.page + 1) * l.rows).contains(&(row as usize))
}

fn settings_row_y(app: &App, row: i32) -> i32 {
    let l = settings_layout(app);
    settings_rect(app).y + 84 + (row - (l.page * l.rows) as i32) * l.stride
}

fn settings_accent_row(_app: &App) -> i32 {
    10
}

fn settings_action_start(app: &App) -> i32 {
    settings_accent_row(app) + 1
}

pub fn settings_sliders(app: &App) -> Vec<SliderDef> {
    let p = settings_rect(app);
    let mk = |id: SliderId,
              row: i32,
              min: f32,
              max: f32,
              value: f32,
              step: f32,
              label: &'static str,
              signed: bool| {
        let narrow = p.w < 420;
        let ry = settings_row_y(app, row) + if narrow { 28 } else { 0 };
        let reset = R { x: p.x + p.w - 80, y: ry, w: 58, h: 38 };
        let badge = R { x: reset.x - 72, y: ry + 4, w: 62, h: 30 };
        let tx = p.x + if narrow { 30 } else { 190 };
        SliderDef {
            id,
            track: R {
                x: tx,
                y: ry + 10,
                w: (badge.x - 18 - tx).max(1),
                h: 18,
            },
            hit: R {
                x: tx - 10,
                y: ry,
                w: (badge.x - 8 - (tx - 10)).max(1),
                h: 42,
            },
            badge,
            reset,
            min,
            max,
            value,
            step,
            label,
            signed,
        }
    };
    let mut v = vec![
        mk(SliderId::Alpha, 0, 30.0, 255.0, app.sidebar_alpha as f32, 1.0, "侧边栏透明度", false),
        mk(SliderId::Scale, 1, 0.6, 1.4, app.sb_scale as f32, 0.01, "侧边栏大小", true),
    ];
    v.push(mk(SliderId::Threshold, 5, 20.0, 260.0, app.large_touch_threshold as f32, 10.0, "大触点阈值", false));
    v.push(mk(SliderId::EraseScale, 6, 0.8, 2.5, app.large_erase_scale10 as f32 / 10.0, 0.1, "大触点橡皮倍率", true));
    v.retain(|s| settings_row_visible(app, match s.id {
        SliderId::Alpha => 0, SliderId::Scale => 1,
        SliderId::Threshold => 5, SliderId::EraseScale => 6,
    }));
    v
}

pub fn settings_slider_hit(app: &App, x: i32, y: i32) -> Option<SliderDef> {
    settings_sliders(app).into_iter().find(|s| s.hit.contains(x, y))
}

pub fn settings_controls(app: &App) -> Vec<(SetHit, R)> {
    let p = settings_rect(app);
    let full = |row: i32| R {
        x: p.x + 18,
        y: settings_row_y(app, row),
        w: p.w - 36,
        h: 38,
    };
    let a = settings_action_start(app);
    let accent_row = settings_accent_row(app);
    let mut controls = vec![
        (SetHit::Autostart, full(2)),
        (SetHit::WpsDebug, full(3)),
        (SetHit::WpsRemove, full(7)),
        (SetHit::WpsRegister, full(8)),
        (SetHit::ProcessBlock, full(9)),
        (SetHit::ThemeAccent, full(accent_row)),
        (SetHit::RightClick, full(4)),
        (SetHit::ClearLog, full(a)),
        (SetHit::SysInfo, full(a + 1)),
        (SetHit::Quit, full(a + 2)),
    ];
    controls.retain(|(hit, _)| settings_row_visible(app, match hit {
        SetHit::Autostart => 2, SetHit::WpsDebug => 3,
        SetHit::WpsRemove => 7, SetHit::WpsRegister => 8, SetHit::ProcessBlock => 9,
        SetHit::ThemeAccent => accent_row,
        SetHit::RightClick => 4,
        SetHit::ClearLog => a, SetHit::SysInfo => a + 1,
        SetHit::Quit => a + 2, _ => unreachable!(),
    }));
    let theme_x = p.x + if p.w < 420 { 18 } else { 84 };
    let available = p.x + p.w - 24 - theme_x;
    let unit = (available - 12) / 7;
    for (preference, offset, width) in [(theme::Preference::Light, 0, unit * 2),
        (theme::Preference::Dark, unit * 2 + 6, unit * 2),
        (theme::Preference::System, unit * 4 + 12, available - unit * 4 - 12)] {
        controls.push((SetHit::Theme(preference), R { x: theme_x + offset, y: p.y + 46, w: width, h: 32 }));
    }
    let l = settings_layout(app);
    let footer = p.y + p.h - 68;
    controls.push((SetHit::Done, R { x: p.x + p.w - 118, y: footer, w: 100, h: 38 }));
    if l.pages > 1 {
        for (delta, offset) in [(-1, 0), (1, 52)] {
            controls.push((SetHit::SettingsPage(delta), R { x: p.x + 18 + offset, y: footer, w: 44, h: 38 }));
        }
    }
    controls.extend(settings_sliders(app).into_iter().map(|s| (SetHit::ResetSlider(s.id), s.reset)));
    controls
}

pub fn settings_hit(app: &App, x: i32, y: i32) -> Option<SetHit> {
    if !settings_rect(app).contains(x, y) {
        return None;
    }
    for (k, r) in settings_controls(app) {
        if r.contains(x, y) {
            return Some(k);
        }
    }
    None
}

// ---------------- 退出确认 / 系统诊断 覆盖层 ----------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConfirmHit {
    Yes,
    No,
}

pub fn confirm_rect(app: &App) -> R {
    let w = 380.min((app.screen_w - 16).max(1));
    let h = 150;
    R {
        x: (app.screen_w - w) / 2,
        y: (app.screen_h - h) / 2,
        w,
        h,
    }
}

pub fn confirm_hit(app: &App, x: i32, y: i32) -> Option<ConfirmHit> {
    let p = confirm_rect(app);
    let by = p.y + 92;
    let half = (p.w - 60) / 2;
    let no = R { x: p.x + 20, y: by, w: half, h: 40 };
    let yes = R { x: p.x + 20 + half + 20, y: by, w: half, h: 40 };
    if no.contains(x, y) {
        Some(ConfirmHit::No)
    } else if yes.contains(x, y) {
        Some(ConfirmHit::Yes)
    } else {
        None
    }
}

pub fn draw_confirm(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let palette = ctx.palette;
    let p = confirm_rect(app);
    panel(ctx, p, 16.0, 1.0);
    ctx.text_center(
        fonts,
        R { x: p.x, y: p.y + 24, w: p.w, h: 40 },
        "确定要退出 Sidera 吗？",
        20.0,
        c(palette.text),
    );
    let half = (p.w - 60) / 2;
    let no = R { x: p.x + 20, y: p.y + 92, w: half, h: 40 };
    let yes = R { x: p.x + 20 + half + 20, y: p.y + 92, w: half, h: 40 };
    surface(ctx, no, 10.0, c(palette.surface), c(palette.border));
    ctx.text_center(fonts, no, "取消", 19.0, c(palette.text));
    surface(ctx, yes, 10.0, c(palette.danger_light), alpha(palette.danger, 85));
    ctx.text_center(fonts, yes, "确定退出", 19.0, c(palette.danger));
}

pub fn diag_rect(app: &App) -> R {
    let w = 620.min((app.screen_w - 16).max(1));
    let h = 620.min((app.screen_h - 16).max(1));
    R {
        x: (app.screen_w - w) / 2,
        y: ((app.screen_h - h) / 2).max(8),
        w,
        h,
    }
}

pub fn diag_close_hit(app: &App, x: i32, y: i32) -> bool {
    let p = diag_rect(app);
    let r = R { x: p.x + 20, y: p.y + p.h - 52, w: p.w - 40, h: 38 };
    r.contains(x, y)
}

pub fn draw_diag(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let palette = ctx.palette;
    let p = diag_rect(app);
    panel(ctx, p, 16.0, 1.0);
    ctx.text_center(
        fonts,
        R { x: p.x, y: p.y + 12, w: p.w, h: 30 },
        "诊断信息与日志",
        18.0,
        c(palette.text),
    );
    let mut yy = p.y + 52;
    for line in app.diag_text.split('\n') {
        if yy > p.y + p.h - 70 {
            break;
        }
        let mut visible = line.to_string();
        if fonts.text_width(&visible, 13.0) > (p.w - 40) as f32 {
            while fonts.text_width(&(visible.clone() + "…"), 13.0) > (p.w - 40) as f32 {
                if visible.pop().is_none() { break; }
            }
            visible.push('…');
        }
        let ink = if visible.contains("[ERROR]") { c(palette.danger) }
            else if visible.contains("[WARN]") { c(palette.warning) }
            else { c(palette.muted) };
        ctx.text(fonts, &visible, (p.x + 20) as f32, yy as f32, 13.0, ink);
        yy += 17;
    }
    let close = R { x: p.x + 20, y: p.y + p.h - 52, w: p.w - 40, h: 38 };
    surface(ctx, close, 10.0, c(palette.accent), c(palette.accent));
    ctx.text_center(fonts, close, "关闭", 19.0, c(palette.on_accent));
}


// ---------------- input 组授权弹窗 ----------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PromptHit {
    Authorize,
    Close,
}

pub fn input_group_prompt_rect(app: &App) -> R {
    let w = 580;
    let h = 224;
    R {
        x: (app.screen_w - w) / 2,
        y: (app.screen_h - h) / 2,
        w,
        h,
    }
}

pub fn input_group_prompt_hit(app: &App, x: i32, y: i32) -> Option<PromptHit> {
    let p = input_group_prompt_rect(app);
    if !p.contains(x, y) {
        return None;
    }
    let by = p.y + p.h - 58;
    if app.input_group_state == 0 {
        let bw = (p.w - 60) / 2;
        let a = R { x: p.x + 20, y: by, w: bw, h: 40 };
        let b = R {
            x: p.x + 20 + bw + 20,
            y: by,
            w: bw,
            h: 40,
        };
        if a.contains(x, y) {
            return Some(PromptHit::Authorize);
        }
        if b.contains(x, y) {
            return Some(PromptHit::Close);
        }
        None
    } else {
        let c = R {
            x: p.x + 20,
            y: by,
            w: p.w - 40,
            h: 40,
        };
        if c.contains(x, y) {
            return Some(PromptHit::Close);
        }
        None
    }
}

pub fn draw_input_group_prompt(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let p = input_group_prompt_rect(app);
    ctx.fill_round(p, 12.0, color(0x2b, 0x2b, 0x33, 255));
    ctx.stroke_round(p, 12.0, color(0x66, 0x66, 0x66, 255), 2.0);
    ctx.text_center(
        fonts,
        R {
            x: p.x,
            y: p.y + 16,
            w: p.w,
            h: 30,
        },
        "需要输入设备权限",
        22.0,
        color(0xee, 0xee, 0xee, 255),
    );
    let user = std::env::var("USER").unwrap_or_default();
    let body = match app.input_group_state {
        0 => format!(
            "当前用户不在 input 组，触屏批注无法读取触摸设备。\n是否加入 input 组？将执行：\npkexec usermod -aG input {}",
            user
        ),
        1 => "正在请求授权，请在弹出的对话框中输入密码…".to_string(),
        2 => "已加入 input 组。\n请注销或重启后重新登录，权限才会生效。".to_string(),
        _ => format!("加入失败。可手动执行：\nsudo usermod -aG input {}", user),
    };
    ctx.text_multiline_center(
        fonts,
        R {
            x: p.x + 16,
            y: p.y + 50,
            w: p.w - 32,
            h: 106,
        },
        &body,
        15.0,
        color(0xcc, 0xcc, 0xcc, 255),
    );
    let by = p.y + p.h - 58;
    if app.input_group_state == 0 {
        let bw = (p.w - 60) / 2;
        let a = R {
            x: p.x + 20,
            y: by,
            w: bw,
            h: 40,
        };
        let b = R {
            x: p.x + 20 + bw + 20,
            y: by,
            w: bw,
            h: 40,
        };
        ctx.fill_round(a, 8.0, color(0x2a, 0x6e, 0x3f, 255));
        ctx.text_center(fonts, a, "授权加入", 18.0, Color::WHITE);
        ctx.fill_round(b, 8.0, color(0x44, 0x44, 0x44, 255));
        ctx.text_center(fonts, b, "稍后", 18.0, Color::WHITE);
    } else {
        let c = R {
            x: p.x + 20,
            y: by,
            w: p.w - 40,
            h: 40,
        };
        ctx.fill_round(c, 8.0, color(0x33, 0x77, 0xcc, 255));
        ctx.text_center(fonts, c, "关闭", 18.0, Color::WHITE);
    }
}

// ---------------- 设置面板用到的外部状态 ----------------
#[cfg(unix)]
pub fn autostart_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("autostart/sidera.desktop")
}

#[cfg(unix)]
pub fn is_autostart() -> bool {
    autostart_path().exists()
}

#[cfg(unix)]
fn autostart_icon() -> String {
    for path in ["/usr/share/pixmaps/sidera.png",
                 "/usr/share/icons/hicolor/256x256/apps/sidera.png",
                 "/usr/share/pixmaps/sidera.svg"] {
        if std::path::Path::new(path).is_file() {
            return path.to_string();
        }
    }
    "sidera".to_string()
}

#[cfg(unix)]
pub fn set_autostart(on: bool) {
    let p = autostart_path();
    if on {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=Sidera\nComment=Sidera软件\nExec=sidera\nIcon={}\nTerminal=false\n",
            autostart_icon());
        let _ = std::fs::write(p, entry);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(windows)]
fn wide_autostart(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
pub fn is_autostart() -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let subkey = wide_autostart(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let value = wide_autostart("Sidera");
    let mut data = [0u16; 1024];
    let mut size = (data.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            data.as_mut_ptr() as *mut std::ffi::c_void,
            &mut size,
        )
    };
    status == 0 && size > 2
}

#[cfg(windows)]
pub fn set_autostart(on: bool) {
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW,
    };
    let subkey = wide_autostart(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let value = wide_autostart("Sidera");
    unsafe {
        if on {
            let exe = std::env::current_exe().unwrap_or_default();
            let data = wide_autostart(&format!("\"{}\"", exe.display()));
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                data.as_ptr() as *const std::ffi::c_void,
                (data.len() * 2) as u32,
            );
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), value.as_ptr());
        }
    }
}

// ---------------- 绘制 ----------------
// ---------------- 绘制 ----------------
fn draw_whiteboard_dots(
    pm: &mut Pixmap,
    app: &App,
    x: i32,
    y: i32,
    x2: i32,
    y2: i32,
    tr: Transform,
) {
    let pitch = crate::app::WHITEBOARD_DOT_PITCH.max(4);
    let radius = app.whiteboard_dot_size.clamp(1, 10) as f32;
    let mut dot = theme::on_color(app.whiteboard_bg_color());
    dot.set_alpha(0.20);
    let paint = Paint {
        shader: Shader::SolidColor(dot),
        anti_alias: true,
        ..Default::default()
    };
    let sx = (x / pitch) * pitch;
    let sy = (y / pitch) * pitch;
    let mut path = PathBuilder::new();
    let mut gy = sy;
    while gy <= y2 {
        let mut gx = sx;
        while gx <= x2 {
            path.push_circle(gx as f32, gy as f32, radius);
            gx += pitch;
        }
        gy += pitch;
    }
    if let Some(path) = path.finish() {
        pm.fill_path(&path, &paint, FillRule::Winding, tr, None);
    }
}

pub fn render_region(
    app: &App,
    fonts: &Fonts,
    icon: Option<&Pixmap>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Option<Pixmap> {
    let palette = app.theme.palette();
    let x = x.max(0);
    let y = y.max(0);
    let x2 = (x + w).min(app.screen_w);
    let y2 = (y + h).min(app.screen_h);
    let rw = (x2 - x).max(0) as u32;
    let rh = (y2 - y).max(0) as u32;
    if rw == 0 || rh == 0 {
        return None;
    }
    let sc = app.scale as f32;
    let pw = ((rw as f32) * sc).round().max(1.0) as u32;
    let ph = ((rh as f32) * sc).round().max(1.0) as u32;
    let mut pm = Pixmap::new(pw, ph)?;
    // world(逻辑) -> region(物理)：先平移再缩放
    let tr = Transform::from_scale(sc, sc).pre_translate(-x as f32, -y as f32);

    // 背景
    if app.whiteboard {
        pm.fill(app.whiteboard_bg_color());
        if app.whiteboard_dots { draw_whiteboard_dots(&mut pm, app, x, y, x2, y2, tr); }
    }

    // 画布（本身已是物理分辨率，按逻辑原点换算后 1:1 贴入）
    pm.draw_pixmap(
        -((x as f32) * sc).round() as i32,
        -((y as f32) * sc).round() as i32,
        app.canvas.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::identity(),
        None,
    );

    if app.launch.active() {
        let mut ctx = Ctx { palette: app.theme.palette(), pm: &mut pm, tr };
        for right in [false, true] { draw_launch_sidebar(app, fonts, &mut ctx, right); }
        let r = launch_rect(app);
        if app.launch.panel_alpha() > 0.0 && x < r.x + r.w && x2 > r.x && y < r.y + r.h && y2 > r.y {
            let (cw, ch) = (((r.w as f32) * sc).round().max(1.0) as u32,
                ((r.h as f32) * sc).round().max(1.0) as u32);
            let mut cache = app.launch.card.borrow_mut();
            if cache.as_ref().is_none_or(|pm| pm.width() != cw || pm.height() != ch) {
                *cache = Some(crate::splash_paint::draw(fonts, icon, &app.launch, cw, ch, palette));
            }
            if let Some(card) = cache.as_ref() {
                ctx.pm.draw_pixmap(0, 0, card.as_ref(), &tiny_skia::PixmapPaint {
                    opacity: app.launch.panel_alpha(), ..Default::default()
                }, tr.pre_translate(r.x as f32, r.y as f32).pre_scale(1.0 / sc, 1.0 / sc), None);
            }
        }
        return Some(pm);
    }

    {
        let mut ctx = Ctx {
            palette,
            pm: &mut pm,
            tr,
        };

        // 手掌/大触点橡皮圆形预览
        if !app.palm_erase_preview.is_empty() {
            let ew = if app.mode == 2 {
                app.eraser_width()
            } else {
                app.palm_erase_w
            } as f32;
            for &(cx, cy) in &app.palm_erase_preview {
                ctx.fill_circle(cx as f32, cy as f32, ew / 2.0, alpha(palette.accent, 28));
                let half = (ew / 2.0) as i32;
                ctx.stroke_round(
                    R {
                        x: cx - half,
                        y: cy - half,
                        w: half * 2,
                        h: half * 2,
                    },
                    ew / 2.0,
                    c(palette.accent_line),
                    2.0,
                );
            }
        }

        // 白板页码
        if app.whiteboard {
            let s = format!("第 {} 页 / {}", app.current_slide, crate::app::CACHE_BOARD);
            let mut page_ink = theme::on_color(app.whiteboard_bg_color());
            page_ink.set_alpha(0.70);
            ctx.text(
                fonts,
                &s,
                16.0,
                app.screen_h as f32 - 22.0,
                20.0,
                page_ink,
            );
            if crate::app::now_ms() < app.wb_limit_msg_until {
                let r = R {
                    x: 0,
                    y: 0,
                    w: app.screen_w,
                    h: app.screen_h,
                };
                ctx.text_center(
                    fonts,
                    r,
                    "已达到白板页数上限",
                    34.0,
                    page_ink,
                );
            }
        }

        if app.whiteboard { draw_whiteboard_controls(app, fonts, &mut ctx); }

        let sidebar_progress = app.sidebar_retract();
        draw_sidebar(app, fonts, &mut ctx, false, sidebar_progress);
        draw_sidebar(app, fonts, &mut ctx, true, sidebar_progress);

        if app.pen_popup_visible {
            draw_pen_popup(app, fonts, &mut ctx);
        }
        if app.eraser_popup_visible {
            draw_eraser_popup(app, fonts, &mut ctx);
        }
        if app.page_picker.is_some() { draw_page_picker(app, fonts, &mut ctx); }
        if app.more_menu_open {
            draw_more_menu(fonts, &mut ctx, app);
        }
        if app.settings_open {
            draw_settings(app, fonts, &mut ctx, icon);
        }
        if app.process_block_open {
            draw_process_block(app, fonts, &mut ctx);
        }
        if app.confirm_quit {
            draw_confirm(app, fonts, &mut ctx);
        }
        if app.show_diag {
            draw_diag(app, fonts, &mut ctx);
        }
        if app.show_input_group_prompt {
            draw_input_group_prompt(app, fonts, &mut ctx);
        }
        draw_press_feedback(app, &mut ctx);
    }

    Some(pm)
}

fn draw_launch_piece(app: &App, ctx: &mut Ctx, target: R, visible: R, r: R,
    draw: impl FnOnce(&mut Ctx)) {
    if r.w <= 0 || r.h <= 0 { return; }
    let space = (r.y - visible.y).min(visible.y + visible.h - r.y - r.h);
    let final_space = (r.y - target.y).min(target.y + target.h - r.y - r.h).clamp(1, 18);
    let opacity = (space as f32 / final_space as f32).clamp(0.0, 1.0);
    // A button fades in only after it fits completely: no squashed icons or
    // sliced labels along the moving top/bottom edges.
    if opacity <= 0.0 { return; }
    let scale = app.scale as f32;
    let Some(mut tile) = Pixmap::new((r.w as f32 * scale).ceil() as u32,
        (r.h as f32 * scale).ceil() as u32) else { return; };
    draw(&mut Ctx { palette: app.theme.palette(), pm: &mut tile,
        tr: Transform::from_scale(scale, scale).pre_translate(-r.x as f32, -r.y as f32) });
    ctx.pm.draw_pixmap(0, 0, tile.as_ref(), &tiny_skia::PixmapPaint { opacity, ..Default::default() },
        ctx.tr.pre_translate(r.x as f32, r.y as f32).pre_scale(1.0 / scale, 1.0 / scale), None);
}

fn draw_launch_sidebar(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool) {
    let palette = ctx.palette;
    let p = app.launch.reveal();
    if p <= 0.0 { return; }
    if p >= 1.0 { draw_sidebar(app, fonts, ctx, right, app.sidebar_retract()); return; }
    let target = sidebar_rect(app, right);
    let visible = sidebar_launch_rect(app, right);
    let radius = (visible.w as f32 * 0.30).min(20.0).min(visible.h as f32 / 2.0);
    surface(ctx, visible, radius, alpha(palette.surface, app.sidebar_alpha), alpha(palette.border, 225));
    let cx = visible.x as f32 + visible.w as f32 / 2.0;
    let grip = app.sb_dot() as f32 * 0.25;
    for y in [visible.y + 6, visible.y + visible.h - 5] {
        ctx.stroke_line((cx - grip, y as f32), (cx + grip, y as f32), c(palette.muted), 1.8);
    }
    if app.collapsed { return; }
    for (button, r) in expanded_sidebar_buttons(app, right) {
        draw_launch_piece(app, ctx, target, visible, r, |local| draw_btn(app, fonts, local, button, r));
    }
    if app.sidebar_page_visible() {
        draw_launch_piece(app, ctx, target, visible, sidebar_page_rect(app, right),
            |local| draw_sidebar_pages(app, fonts, local, right));
    }
}

fn draw_sidebar(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool, progress: f32) {
    let palette = ctx.palette;
    let drag = app.sidebar_drag.filter(|drag| drag.right == right);
    let held = drag.is_some();
    let stroke_width = 2.0;
    if progress < 1.0 {
        let sb = expanded_sidebar_rect(app, right);
        let dx = (sb.w + 4) as f32 * progress * if right { 1.0 } else { -1.0 };
        let mut moving = Ctx { palette: app.theme.palette(), pm: &mut *ctx.pm, tr: ctx.tr.pre_translate(dx, 0.0) };
        let radius = (sb.w as f32 * 0.30).min(20.0);
        let fill = if held { alpha(palette.accent_light, app.sidebar_alpha.max(200)) }
            else { alpha(palette.surface, app.sidebar_alpha) };
        surface(&mut moving, sb, radius, fill, if held { c(palette.accent) } else { alpha(palette.border, 225) });
        if held {
            moving.stroke_round(R { x: sb.x + 2, y: sb.y + 2, w: sb.w - 4, h: sb.h - 4 },
                (radius - 2.0).max(1.0), c(palette.accent), stroke_width);
        }
        let cx = sb.x as f32 + sb.w as f32 / 2.0;
        let grip = app.sb_dot() as f32 * 0.25;
        for y in [sb.y + 6, sb.y + sb.h - 5] {
            if held {
                let w = (app.sb_dot() + 6).clamp(20, sb.w - 6);
                moving.fill_round(R { x: sb.x + (sb.w - w) / 2, y: y - 4, w, h: 8 }, 4.0, c(palette.accent));
            }
            moving.stroke_line((cx - grip, y as f32), (cx + grip, y as f32),
                c(if held { palette.on_accent } else { palette.muted }), if held { 2.2 } else { 1.8 });
        }
        for (k, r) in expanded_sidebar_buttons(app, right) { draw_btn(app, fonts, &mut moving, k, r); }
        draw_opening_tools(app, fonts, &mut moving, right);
        draw_sidebar_pages(app, fonts, &mut moving, right);
    }
    if progress > 0.0 {
        let tab = sidebar_tab_rect(app, right);
        let dx = tab.w as f32 * (1.0 - progress) * if right { 1.0 } else { -1.0 };
        let mut moving = Ctx { palette: app.theme.palette(), pm: &mut *ctx.pm, tr: ctx.tr.pre_translate(dx, 0.0) };
        // Extend under the screen edge so only the inward corners are rounded.
        let outline = R { x: tab.x - if right { 0 } else { 12 }, w: tab.w + 12, ..tab };
        surface(&mut moving, outline, 12.0, c(if held { palette.accent_light } else { palette.paper }),
            c(if held { palette.accent } else { palette.accent_line }));
        if held {
            moving.stroke_round(R { x: outline.x + 2, y: outline.y + 2, w: outline.w - 4, h: outline.h - 4 },
                10.0, c(palette.accent), stroke_width);
        }
        let cx = tab.x as f32 + tab.w as f32 / 2.0;
        let cy = tab.y as f32 + tab.h as f32 / 2.0;
        let direction = if right { -1.0 } else { 1.0 };
        moving.stroke_line((cx - direction * 3.0, cy - 6.0), (cx + direction * 3.0, cy), c(palette.accent), 2.4);
        moving.stroke_line((cx + direction * 3.0, cy), (cx - direction * 3.0, cy + 6.0), c(palette.accent), 2.4);
    }
}

fn draw_opening_tools(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool) {
    let fraction = app.drawing_tools.fraction();
    let h = app.drawing_tools_height();
    if fraction <= 0.0 || fraction >= 1.0 || h == 0 { return; }
    let sb = expanded_sidebar_rect(app, right);
    let y = sb.y + 12 + 3 * (app.sb_btn() + 6);
    let scale = app.scale as f32;
    for (button, r) in drawing_tool_buttons(app, right) {
        let bottom = r.y + r.h - y;
        // Fade only complete controls into the revealed space. Never cut a
        // label or squeeze an icon against the moving navigation controls.
        let opacity = ((h - bottom) as f32 / (app.drawing_tools_full_height() - bottom) as f32).clamp(0.0, 1.0);
        if opacity <= 0.0 { continue; }
        let Some(mut tile) = Pixmap::new((r.w as f32 * scale).ceil() as u32,
            (r.h as f32 * scale).ceil() as u32) else { continue };
        let mut local = Ctx { palette: app.theme.palette(), pm: &mut tile,
            tr: Transform::from_scale(scale, scale).pre_translate(-r.x as f32, -r.y as f32) };
        draw_btn(app, fonts, &mut local, button, r);
        ctx.pm.draw_pixmap(0, 0, tile.as_ref(), &tiny_skia::PixmapPaint {
            opacity, ..Default::default()
        }, ctx.tr.pre_translate(r.x as f32, r.y as f32).pre_scale(1.0 / scale, 1.0 / scale), None);
    }
}

fn draw_sidebar_pages(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool) {
    let palette = ctx.palette;
    if !app.sidebar_page_visible() { return; }
    let r = sidebar_page_rect(app, right);
    let (page, total) = sidebar_page_values(app);
    let text = format!("{page} / {total}");
    let available_picker = app.page_picker_context().is_some();
    let opened = app.page_picker.is_some() && app.popup_on_right == right;
    surface(ctx, r, 6.0, c(if opened { palette.accent_light } else { palette.surface }),
        c(if available_picker { palette.accent_line } else { palette.border }));
    let available = (r.w - 6).max(1) as f32;
    let nominal = (13.0 * app.sb_scale as f32).clamp(11.0, 16.0);
    let fit = |text: &str, size: f32| size * (available / fonts.text_width(text, size).max(1.0)).min(1.0);
    let size = fit(&text, nominal);
    let ink = c(if page == "—" { palette.muted } else { palette.text });
    if size >= 10.5 {
        ctx.text_center(fonts, r, &text, size, ink);
    } else {
        // Long decks and small toolbar scales retain both complete numbers.
        let top = R { h: r.h / 2, ..r };
        let bottom = R { y: r.y + top.h, h: r.h - top.h, ..r };
        let total = format!("/ {total}");
        let size = nominal.min(top.h as f32 - 1.0);
        ctx.text_center(fonts, top, &page, fit(&page, size), ink);
        ctx.text_center(fonts, bottom, &total, fit(&total, size), c(palette.muted));
    }
}

fn draw_btn(app: &App, fonts: &Fonts, ctx: &mut Ctx, k: Btn, r: R) {
    let palette = ctx.palette;
    let btn = app.sb_btn() as f32;
    let radius = (r.w as f32 * 0.24).min(10.0);
    if k == Btn::Exit && !app.whiteboard {
        draw_presentation_button(app, fonts, ctx, r, radius);
        return;
    }
    let selected = sidebar_button_selected(app, k);
    let mut fill = c(if selected { palette.accent } else { palette.paper });
    let mut edge = c(if selected { palette.text } else { palette.border });
    let mut ink = c(if selected { palette.on_accent } else { palette.text });
    if !control_enabled(app, Control::Sidebar(k, false)) {
        fill = c(palette.disabled_surface);
        edge = c(palette.disabled_border);
        ink = c(palette.disabled_ink);
    }
    if matches!(k, Btn::Prev | Btn::Next) && !app.whiteboard && !app.wps_navigation_ready {
        fill = c(palette.warning_light); edge = c(palette.warning); ink = c(palette.warning);
    }
    if k == Btn::Exit {
        fill = app.whiteboard_bg_color();
        ink = theme::on_color(fill);
    }
    surface(ctx, r, radius, fill, edge);
    match k {
        Btn::Collapse => {
            ctx.text_multiline_center(fonts, r, "收起", (btn * 0.49).max(11.0), c(palette.muted));
        }
        Btn::Cursor | Btn::Pen | Btn::Eraser | Btn::Prev | Btn::Next | Btn::More => {
            let glyph = match k {
                Btn::Cursor => Glyph::Cursor, Btn::Pen => Glyph::Pen, Btn::Eraser => Glyph::Eraser,
                Btn::Prev => Glyph::Up, Btn::Next => Glyph::Down, _ => Glyph::More,
            };
            // 21 / 24 / 27 logical pixels at default scale: visible size cues,
            // with a fixed centre and fixed hit target at every toolbar scale.
            let factor = if k == Btn::Eraser { [0.88, 1.0, 1.12][app.cur_eraser.min(2)] } else { 1.0 };
            let size = (app.sb_icon() as f32 * factor).min(r.w.min(r.h) as f32 * 0.82);
            let x = r.x as f32 + (r.w as f32 - size) / 2.0;
            let y = r.y as f32 + (r.h as f32 - size) / 2.0;
            // Filled stationery keeps its own outline, while plain arrows/dots
            // use the label contrast of the selected button.
            let glyph_ink = if selected && matches!(k, Btn::Cursor | Btn::Pen | Btn::Eraser) {
                c(palette.text)
            } else { ink };
            icons::draw(ctx, glyph, x, y, size, glyph_ink, app.pen_color());
        }
        Btn::Clear => ctx.text_center(fonts, r, "清除", (btn * 0.49).max(11.0), c(palette.danger)),
        Btn::Restore => ctx.text_center(fonts, r, "恢复", (btn * 0.49).max(11.0), ink),
        Btn::Undo => ctx.text_center(fonts, r, "撤回", (btn * 0.49).max(11.0), ink),
        Btn::Whiteboard => ctx.text_center(fonts, r, "白板", (btn * 0.49).max(11.0), ink),
        Btn::Exit => {
            ctx.text_multiline_center(fonts, r, "背景\n颜色", (btn * 0.42).max(10.0), ink);
        }
    }
}

fn draw_presentation_button(app: &App, fonts: &Fonts, ctx: &mut Ctx, r: R, radius: f32) {
    let palette = ctx.palette;
    let weights = app.presentation_transition.weights();
    let blend = |colors: [u32; 3]| {
        let channel = |shift: u32| -> u8 {
            colors.iter().zip(weights).map(|(rgb, weight)| ((rgb >> shift) & 255) as f32 * weight)
                .sum::<f32>().round().clamp(0.0, 255.0) as u8
        };
        Color::from_rgba8(channel(16), channel(8), channel(0), 255)
    };
    surface(ctx, r, radius,
        blend([palette.paper, palette.success, palette.danger_light]),
        blend([palette.border, palette.success, palette.danger]));
    // Fade through: only one label is visible, so Esc and the two-line Chinese
    // captions never overlap. Deriving opacity from the same sampled weights
    // also keeps interrupted transitions continuous, on both sidebars.
    for (index, (label, ink, factor)) in [
        ("Esc", palette.text, 0.50),
        ("开始\n放映", palette.on_success, 0.42),
        ("结束\n放映", palette.danger, 0.42),
    ].into_iter().enumerate() {
        let opacity = (2.0 * weights[index] - 1.0).clamp(0.0, 1.0);
        if opacity <= 0.0 { continue; }
        ctx.text_multiline_center(fonts, r, label, (app.sb_btn() as f32 * factor).max(10.0),
            alpha(ink, (255.0 * opacity).round() as u8));
    }
}

fn draw_page_picker(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let opacity = anim_t(app);
    if opacity >= 1.0 { draw_page_picker_content(app, fonts, ctx); return; }
    if opacity <= 0.0 { return; }
    let p = page_picker_rect(app);
    let scale = app.scale as f32;
    let Some(mut tile) = Pixmap::new((p.w as f32 * scale).ceil() as u32,
        (p.h as f32 * scale).ceil() as u32) else { return };
    let mut local = Ctx { palette: app.theme.palette(), pm: &mut tile,
        tr: Transform::from_scale(scale, scale).pre_translate(-p.x as f32, -p.y as f32) };
    draw_page_picker_content(app, fonts, &mut local);
    ctx.pm.draw_pixmap(0, 0, tile.as_ref(), &tiny_skia::PixmapPaint { opacity, ..Default::default() },
        ctx.tr.pre_translate(p.x as f32, p.y as f32).pre_scale(1.0 / scale, 1.0 / scale), None);
}

fn draw_page_picker_content(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let palette = ctx.palette;
    let Some(picker) = &app.page_picker else { return };
    let p = page_picker_rect(app);
    panel(ctx, p, 16.0, 1.0);
    ctx.text(fonts, "跳转页面", (p.x + 18) as f32, (p.y + 32) as f32, 23.0, c(palette.text));
    let hint = format!("共 {} 页 · 上下拨动选页", picker.total);
    let hint_size = 16.0 * ((p.w - 36) as f32 / fonts.text_width(&hint, 16.0).max(1.0)).min(1.0);
    ctx.text(fonts, &hint, (p.x + 18) as f32, (p.y + 56) as f32, hint_size, c(palette.muted));
    let wheel = page_picker_control_rect(app, PagePickerHit::Wheel);
    surface(ctx, wheel, 12.0, c(palette.surface), c(palette.border));
    let center = wheel.y as f64 + wheel.h as f64 / 2.0;
    let band = R { x: wheel.x + 8, y: center as i32 - 22, w: wheel.w - 16, h: 44 };
    surface(ctx, band, 9.0, c(palette.accent_light), c(palette.accent_line));
    if picker.dragging() { ctx.stroke_round(band, 9.0, c(palette.accent), 2.0); }
    let nearest = picker.position.round() as i32;
    // Only five nearby rows are painted, independent of the deck's page count.
    for page in nearest.saturating_sub(3).max(1)..=nearest.saturating_add(3).min(picker.total) {
        let distance = page as f64 - picker.position;
        if distance.abs() > 2.2 { continue; }
        let size = (30.0 - 7.0 * distance.abs()).max(17.0) as f32;
        // Size and the selection band supply depth; keep neighbouring page
        // numbers readable in a classroom instead of fading them almost away.
        let opacity = (1.0 - distance.abs() / 10.0).clamp(0.80, 1.0) as f32;
        let ink = if distance.abs() < 0.5 { palette.accent } else { palette.text };
        let text = page.to_string();
        let fit = size * ((wheel.w - 36) as f32 / fonts.text_width(&text, size).max(1.0)).min(1.0);
        let x = wheel.x as f32 + (wheel.w as f32 - fonts.text_width(&text, fit)) / 2.0;
        ctx.text(fonts, &text, x, (center + distance * crate::page_picker::ROW) as f32 + fit * 0.35,
            fit, alpha(ink, (opacity * 255.0) as u8));
    }
    for (hit, title, selected) in [
        (PagePickerHit::Cancel, "取消".to_string(), false),
        (PagePickerHit::Jump, format!("跳至第 {} 页", picker.selected()), true),
    ] {
        let r = page_picker_control_rect(app, hit);
        surface(ctx, r, 10.0, c(if selected { palette.accent } else { palette.paper }), c(palette.accent_line));
        let size = 18.0 * ((r.w - 12) as f32 / fonts.text_width(&title, 18.0).max(1.0)).min(1.0);
        ctx.text_center(fonts, r, &title, size, c(if selected { palette.on_accent } else { palette.text }));
    }
}

fn draw_pen_popup(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let palette = ctx.palette;
    let p = pen_popup_rect(app);
    let t = anim_t(app);
    panel(ctx, p, 16.0, t);
    if t < 0.02 { return; }
    ctx.text(fonts, "画笔颜色", (p.x + 16) as f32, (p.y + 32) as f32, 21.0, c(palette.text));
    icons::draw(ctx, Glyph::Pen, (p.x + p.w - 42) as f32, (p.y + 12) as f32, 24.0, c(palette.text), app.pen_color());
    let start_y = p.y + 46;
    for i in 0..10usize {
        let (r, g, b) = crate::app::COLORS[i];
        let cx = (p.x + 16 + (i % 4) as i32 * 48 + 19) as f32;
        let cy = (start_y + (i / 4) as i32 * 48 + 19) as f32;
        ctx.fill_circle(cx, cy, 16.0, color(r, g, b, 255));
        ctx.stroke_round(R { x: cx as i32 - 16, y: cy as i32 - 16, w: 32, h: 32 },
            16.0, alpha(palette.text, 55), 1.0);
        if i == app.cur_color {
            ctx.stroke_round(R { x: cx as i32 - 19, y: cy as i32 - 19, w: 38, h: 38 },
                19.0, c(palette.accent), 2.0);
            let check = theme::on_color(color(r, g, b, 255));
            icons::draw(ctx, Glyph::Check, cx - 10.0, cy - 10.0, 20.0, check, check);
        }
    }
    let names = ["红色", "蓝色", "绿色", "黄色", "白色", "青色", "紫色", "黄绿", "橙色", "棕色"];
    ctx.text(fonts, "当前颜色", (p.x + 124) as f32, (p.y + 156) as f32, 15.0, c(palette.muted));
    ctx.text(fonts, names[app.cur_color.min(9)], (p.x + 124) as f32, (p.y + 177) as f32, 20.0, c(palette.text));
    let sy = p.y + 214;
    ctx.text(fonts, "画笔粗细", (p.x + 16) as f32, (sy - 14) as f32, 19.0, c(palette.muted));
    let bw = (p.w - 52) / 3;
    for (i, label) in ["细 3", "中 5", "粗 10"].iter().enumerate() {
        let r = R { x: p.x + 16 + i as i32 * (bw + 10), y: sy, w: bw, h: 48 };
        choice(ctx, r, i == app.cur_pen);
        ctx.text_center(fonts, R { h: 28, ..r }, label, 18.0, c(palette.text));
        let a = ((r.x + r.w / 2 - 12) as f32, (r.y + 36) as f32);
        let b = ((r.x + r.w / 2 + 12) as f32, (r.y + 36) as f32);
        let width = crate::app::PEN_SIZES[i] as f32;
        if app.cur_color == 4 { ctx.stroke_line(a, b, c(palette.border), width + 2.0); }
        ctx.stroke_line(a, b, app.pen_color(), width);
    }
}

fn draw_eraser_popup(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let palette = ctx.palette;
    let p = eraser_popup_rect(app);
    let t = anim_t(app);
    panel(ctx, p, 16.0, t);
    if t < 0.02 { return; }
    ctx.text(fonts, "橡皮大小", (p.x + 16) as f32, (p.y + 32) as f32, 21.0, c(palette.text));
    let size = [22.0, 27.0, 32.0][app.cur_eraser.min(2)];
    icons::draw(ctx, Glyph::Eraser, p.x as f32 + p.w as f32 - 30.0 - size / 2.0,
        p.y as f32 + 24.0 - size / 2.0, size, c(palette.text), c(palette.eraser));
    let bw = (p.w - 52) / 3;
    for (i, label) in ["小 30", "中 60", "大 80"].iter().enumerate() {
        let r = R { x: p.x + 16 + i as i32 * (bw + 10), y: p.y + 46, w: bw, h: 48 };
        choice(ctx, r, i == app.cur_eraser);
        let size = [20.0, 25.0, 30.0][i];
        icons::draw(ctx, Glyph::Eraser, r.x as f32 + 19.0 - size / 2.0, r.y as f32 + 24.0 - size / 2.0,
            size, c(palette.text), c(palette.eraser));
        ctx.text_center(fonts, R { x: r.x + 33, w: r.w - 37, ..r }, label, 18.0, c(palette.text));
    }
    let r = R { x: p.x + 16, y: p.y + 110, w: p.w - 32, h: 48 };
    surface(ctx, r, 10.0, c(palette.danger_light), alpha(palette.danger, 75));
    ctx.text_center(fonts, r, "清空当前页", 20.0, c(palette.danger));
}

fn draw_more_menu(fonts: &Fonts, ctx: &mut Ctx, app: &App) {
    let palette = ctx.palette;
    let m = more_menu_rect(app);
    let t = anim_t(app);
    panel(ctx, m, 14.0, t);
    if t < 0.02 { return; }
    let mut rows = vec![(0, Glyph::Camera, "截图")];
    if app.more_settings_entry { rows.push((1, Glyph::Settings, "设置")); }
    if rows.len() > 1 {
        ctx.stroke_line(((m.x + 14) as f32, (m.y + 48) as f32),
            ((m.x + m.w - 14) as f32, (m.y + 48) as f32), c(palette.border), 1.0);
    }
    for (row, glyph, label) in rows {
        let y = m.y + row * 48;
        icons::draw(ctx, glyph, (m.x + 18) as f32, (y + 12) as f32, 24.0, c(palette.accent), c(palette.accent));
        ctx.text(fonts, label, (m.x + 58) as f32, (y + 31) as f32, 21.0, c(palette.text));
    }
}

fn draw_whiteboard_controls(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let palette = ctx.palette;
    let bar = whiteboard_bar_rect(app);
    panel(ctx, bar, 12.0, 1.0);
    let toggle = whiteboard_dots_toggle_rect(app);
    ctx.text(
        fonts,
        "点阵",
        (toggle.x + 4) as f32,
        (toggle.y + 25) as f32,
        19.0,
        c(palette.text),
    );
    let sw = R {
        x: toggle.x + toggle.w - 52,
        y: toggle.y + 8,
        w: 44,
        h: 22,
    };
    ctx.fill_round(
        sw,
        11.0,
        c(if app.whiteboard_dots {
            palette.accent
        } else {
            palette.muted
        }),
    );
    let cx = sw.x + if app.whiteboard_dots { 33 } else { 11 };
    ctx.fill_circle(cx as f32, (sw.y + 11) as f32, 8.0, c(palette.on_accent));
    if app.whiteboard_dots {
        let r = whiteboard_dot_slider_rect(app);
        let t = ((app.whiteboard_dot_size - 1) as f32 / 9.0).clamp(0.0, 1.0);
        ctx.fill_round(r, 13.0, c(palette.accent_light));
        let knob = R {
            x: r.x + ((r.w - 22) as f32 * t) as i32,
            y: r.y,
            w: 22,
            h: r.h,
        };
        ctx.fill_round(knob, 11.0, c(palette.accent));
        ctx.text_center(
            fonts,
            R {
                x: r.x,
                y: r.y - 2,
                w: r.w,
                h: r.h,
            },
            &format!("点大小 {}", app.whiteboard_dot_size),
            15.0,
            c(palette.text),
        );
    }
}

fn draw_process_block(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let palette = ctx.palette;
    let p = process_block_rect(app);
    panel(ctx, p, 18.0, 1.0);
    ctx.text(
        fonts,
        "进程屏蔽",
        (p.x + 22) as f32,
        (p.y + 38) as f32,
        26.0,
        c(palette.text),
    );
    ctx.text(
        fonts,
        "启动 Sidera 后结束匹配的当前用户进程",
        (p.x + 150) as f32,
        (p.y + 37) as f32,
        15.0,
        c(palette.muted),
    );
    let enable = process_block_controls(app)
        .into_iter()
        .find(|(hit, _)| *hit == ProcessHit::Enable)
        .unwrap()
        .1;
    ctx.text(
        fonts,
        "启用进程屏蔽",
        (enable.x + 4) as f32,
        (enable.y + 25) as f32,
        19.0,
        c(palette.text),
    );
    let sw = R {
        x: enable.x + enable.w - 52,
        y: enable.y + 7,
        w: 44,
        h: 22,
    };
    ctx.fill_round(
        sw,
        11.0,
        c(if app.process_block_enabled {
            palette.accent
        } else {
            palette.muted
        }),
    );
    let cx = sw.x + if app.process_block_enabled { 33 } else { 11 };
    ctx.fill_circle(cx as f32, (sw.y + 11) as f32, 8.0, c(palette.on_accent));

    let input = process_block_controls(app)
        .into_iter()
        .find(|(hit, _)| *hit == ProcessHit::Input)
        .unwrap()
        .1;
    surface(
        ctx,
        input,
        8.0,
        c(palette.paper),
        c(if app.process_input_active {
            palette.accent
        } else {
            palette.border
        }),
    );
    let shown: String = app.process_input.chars().take(42).collect();
    let text = if shown.is_empty() && !app.process_input_active {
        "输入进程名或可执行文件路径".to_string()
    } else {
        shown
    };
    ctx.text(
        fonts,
        &text,
        (input.x + 10) as f32,
        (input.y + 26) as f32,
        18.0,
        c(
            if app.process_input.is_empty() && !app.process_input_active {
                palette.muted
            } else {
                palette.text
            },
        ),
    );

    for (hit, r) in process_block_controls(app) {
        match hit {
            ProcessHit::Add
            | ProcessHit::Paste
            | ProcessHit::Scan
            | ProcessHit::ScrollUp
            | ProcessHit::ScrollDown
            | ProcessHit::Done => {
                let (label, accent) = match hit {
                    ProcessHit::Add => ("添加", true),
                    ProcessHit::Paste => ("粘贴", false),
                    ProcessHit::Scan => ("扫描进程", false),
                    ProcessHit::ScrollUp => ("上翻", false),
                    ProcessHit::ScrollDown => ("下翻", false),
                    _ => ("完成", true),
                };
                surface(
                    ctx,
                    r,
                    8.0,
                    c(if accent {
                        palette.accent_light
                    } else {
                        palette.surface
                    }),
                    c(if accent {
                        palette.accent_line
                    } else {
                        palette.border
                    }),
                );
                ctx.text_center(
                    fonts,
                    r,
                    label,
                    18.0,
                    c(if accent { palette.accent } else { palette.text }),
                );
            }
            ProcessHit::Rule(_) | ProcessHit::Proc(_) => {
                let rows = process_rows(app);
                let index = match hit {
                    ProcessHit::Rule(i) => i,
                    ProcessHit::Proc(i) => app.process_rules.len() + i,
                    _ => continue,
                };
                let Some((_, label, detail)) = rows.get(index) else {
                    continue;
                };
                let active = matches!(hit, ProcessHit::Rule(_));
                surface(
                    ctx,
                    r,
                    8.0,
                    c(if active {
                        palette.danger_light
                    } else {
                        palette.surface
                    }),
                    if active {
                        alpha(palette.danger, 70)
                    } else {
                        c(palette.border)
                    },
                );
                ctx.text(
                    fonts,
                    label,
                    (r.x + 12) as f32,
                    (r.y + 21) as f32,
                    17.0,
                    c(palette.text),
                );
                ctx.text(
                    fonts,
                    detail,
                    (r.x + r.w - 220) as f32,
                    (r.y + 21) as f32,
                    14.0,
                    c(palette.muted),
                );
            }
            _ => {}
        }
    }
    if process_rows(app).is_empty() {
        ctx.text_center(
            fonts,
            R {
                x: p.x + 20,
                y: p.y + 250,
                w: p.w - 40,
                h: 60,
            },
            "暂无规则，输入进程名或点击“扫描进程”",
            17.0,
            c(palette.muted),
        );
    }
}

fn draw_settings(app: &App, fonts: &Fonts, ctx: &mut Ctx, icon: Option<&Pixmap>) {
    let palette = ctx.palette;
    let p = settings_rect(app);
    panel(ctx, p, 18.0, 1.0);
    if let Some(ico) = icon {
        let ix = p.x + 24;
        let iy = p.y + if p.w < 420 { 8 } else { 20 };
        let size = if p.w < 420 { 32.0 } else { 44.0 };
        ctx.pm.draw_pixmap(0, 0, ico.as_ref(), &tiny_skia::PixmapPaint {
            quality: tiny_skia::FilterQuality::Bicubic, ..Default::default()
        }, ctx.tr.pre_translate(ix as f32, iy as f32)
            .pre_scale(size / ico.width() as f32, size / ico.height() as f32), None);
    }
    ctx.text(fonts, "Sidera 设置", (p.x + 84) as f32, (p.y + 36) as f32, 27.0, c(palette.text));
    ctx.stroke_line(((p.x + 24) as f32, (p.y + 82) as f32),
        ((p.x + p.w - 24) as f32, (p.y + 82) as f32), c(palette.border), 1.0);

    // Paint only complete rows on this page, without scaling down text or
    // rasterizing a tall panel and stretching it into the available space.
    for (first, count) in [(2, 4), (settings_action_start(app), 2)] {
        for row in first..first + count {
            if !settings_row_visible(app, row) { continue; }
            let group = R { x: p.x + 18, y: settings_row_y(app, row), w: p.w - 36, h: 40 };
            surface(ctx, group, 10.0, c(palette.surface), c(palette.border));
        }
    }

    for s in &settings_sliders(app) {
        let ry = s.hit.y;
        let label = match s.id {
            SliderId::Alpha => "工具栏不透明度", SliderId::Scale => "工具栏大小", _ => s.label,
        };
        let baseline = ry + if p.w < 420 { -4 } else { 27 };
        ctx.text(fonts, label, (p.x + 26) as f32, baseline as f32, 22.0, c(palette.text));
        let track = s.track;
        let cy = track.y + track.h / 2;
        let rail = R { x: track.x, y: cy - 3, w: track.w, h: 6 };
        ctx.fill_round(rail, 3.0, c(palette.border));
        let fw = (track.w as f32 * s.t()) as i32;
        if fw > 1 { ctx.fill_round(R { w: fw, ..rail }, 3.0, c(palette.accent)); }
        let hx = track.x + fw;
        let pressed = app.press.is_some_and(|p| p.active && !p.expired() && p.control == Control::Slider(s.id));
        if pressed { ctx.fill_circle(hx as f32, cy as f32, 17.0, alpha(palette.accent, 45)); }
        ctx.fill_circle(hx as f32, cy as f32, 11.0, c(palette.paper));
        ctx.stroke_round(R { x: hx - 11, y: cy - 11, w: 22, h: 22 }, 11.0, c(palette.accent), 2.0);
        let value = match s.id {
            SliderId::Alpha => format!("{:.0}%", s.value / 255.0 * 100.0),
            SliderId::Scale => format!("{:.0}%", s.value * 100.0),
            SliderId::EraseScale | SliderId::Threshold => {
                let number = if s.signed { format!("{:.1}", s.value) } else { format!("{:.0}", s.value) };
                if s.id == SliderId::EraseScale { format!("{number}×") } else { number }
            }
        };
        ctx.fill_round(s.badge, 8.0, c(palette.surface));
        ctx.text_center(fonts, s.badge, &value, 20.0, c(palette.accent));
    }

    let controls = settings_controls(app);
    for (hit, r) in controls {
        match hit {
            SetHit::Theme(preference) => {
                let selected = app.theme.preference == preference;
                surface(ctx, r, 8.0, c(if selected { palette.accent } else { palette.surface }),
                    c(if selected { palette.accent } else { palette.border }));
                ctx.text_center(fonts, r, preference.label(), 17.0,
                    c(if selected { palette.on_accent } else { palette.muted }));
            }
            SetHit::ResetSlider(_) => {
                surface(ctx, r, 8.0, c(palette.paper), c(palette.accent_line));
                ctx.text_center(fonts, r, "默认", 18.0, c(palette.accent));
            }
            SetHit::Autostart | SetHit::WpsDebug | SetHit::RightClick => {
                let (label, on) = match hit {
                    SetHit::Autostart => ("开机自启动", is_autostart()),
                    SetHit::WpsDebug => ("WPS 联动", app.wps_debug),
                    _ => ("右键切回光标", app.right_click_cursor_on),
                };
                ctx.text(fonts, label, (r.x + 14) as f32, (r.y + 27) as f32, 22.0, c(palette.text));
                let toggle = R { x: r.x + r.w - 58, y: r.y + 8, w: 44, h: 22 };
                ctx.fill_round(toggle, 11.0, c(if on { palette.accent } else { palette.muted }));
                let cx = toggle.x + if on { 33 } else { 11 };
                ctx.fill_circle(cx as f32, (toggle.y + 11) as f32, 8.0, c(palette.on_accent));
                ctx.text_center(fonts, R { x: toggle.x - 50, y: r.y, w: 42, h: r.h },
                    if on { "开启" } else { "关闭" }, 17.0, c(palette.muted));
            }
            SetHit::WpsRemove => {
                surface(ctx, r, 10.0, c(palette.danger_light), alpha(palette.danger, 65));
                ctx.text(fonts, "清除 WPS 加载项", (r.x + 14) as f32, (r.y + 27) as f32,
                    22.0, c(palette.danger));
            }
            SetHit::WpsRegister => {
                surface(ctx, r, 10.0, c(palette.accent_light), c(palette.accent_line));
                ctx.text(fonts, "重新登记 WPS 加载项", (r.x + 14) as f32, (r.y + 27) as f32,
                    22.0, c(palette.accent));
            }
            SetHit::ProcessBlock => {
                ctx.text(fonts, "进程屏蔽", (r.x + 14) as f32, (r.y + 27) as f32, 22.0, c(palette.text));
                let badge = R { x: r.x + r.w - 88, y: r.y + 5, w: 74, h: 28 };
                ctx.fill_round(badge, 8.0, c(if app.process_block_enabled { palette.accent_light } else { palette.surface }));
                ctx.text_center(fonts, badge, if app.process_block_enabled { "已启用" } else { "未启用" }, 17.0,
                    c(if app.process_block_enabled { palette.accent } else { palette.muted }));
            }
            SetHit::ThemeAccent => {
                ctx.text(fonts, "主题强调色", (r.x + 14) as f32, (r.y + 27) as f32,
                    22.0, c(palette.text));
                let badge = R { x: r.x + r.w - 130, y: r.y + 5, w: 116, h: 28 };
                ctx.fill_round(badge, 8.0, c(palette.accent_light));
                ctx.fill_circle((badge.x + 16) as f32, (badge.y + 14) as f32, 8.0, c(palette.accent));
                ctx.text(fonts, app.theme.accent.label(), (badge.x + 30) as f32,
                    (badge.y + 20) as f32, 17.0, c(palette.accent));
            }
            SetHit::ClearLog | SetHit::SysInfo => {
                let label = if hit == SetHit::SysInfo { "诊断信息 / 日志" }
                    else if crate::app::now_ms() < app.log_cleared_until { "日志已清空" } else { "清空调试日志" };
                let glyph = if hit == SetHit::SysInfo { Glyph::Info } else { Glyph::Document };
                icons::draw(ctx, glyph, (r.x + 13) as f32, (r.y + 8) as f32, 22.0, c(palette.muted), c(palette.muted));
                ctx.text(fonts, label, (r.x + 47) as f32, (r.y + 27) as f32, 21.0, c(palette.text));
                if hit == SetHit::SysInfo {
                    icons::draw(ctx, Glyph::Right, (r.x + r.w - 32) as f32, (r.y + 10) as f32,
                        18.0, c(palette.muted), c(palette.muted));
                }
            }
            SetHit::Quit => {
                surface(ctx, r, 10.0, c(palette.danger_light), alpha(palette.danger, 65));
                ctx.text_center(fonts, r, "退出软件", 22.0, c(palette.danger));
            }
            SetHit::Done => {
                surface(ctx, r, 10.0, c(palette.accent), c(palette.accent));
                ctx.text_center(fonts, r, "完成", 24.0, c(palette.on_accent));
                icons::draw(ctx, Glyph::Check, r.x as f32 + r.w as f32 / 2.0 - 44.0,
                    (r.y + 10) as f32, 18.0, c(palette.on_accent), c(palette.on_accent));
            }
            SetHit::SettingsPage(delta) => {
                let enabled = control_enabled(app, Control::Setting(hit));
                surface(ctx, r, 9.0, c(palette.surface), c(palette.border));
                let ink = if enabled { c(palette.accent) } else { alpha(palette.muted, 85) };
                icons::draw(ctx, if delta < 0 { Glyph::Up } else { Glyph::Down },
                    (r.x + 12) as f32, (r.y + 9) as f32, 20.0, ink, ink);
            }
        }
    }
    let l = settings_layout(app);
    if l.pages > 1 {
        ctx.text_center(fonts, R { x: p.x + 116, y: p.y + p.h - 68, w: p.w - 240, h: 38 },
            &format!("{} / {}", l.page + 1, l.pages), 18.0, c(palette.muted));
    }
    ctx.text_center(fonts, R { x: p.x, y: p.y + p.h - 30, w: p.w, h: 16 },
        &format!("Sidera {}  © 2026 Carl_Jin  GNU GPL v3", crate::app::VERSION), 11.0, c(palette.muted));
    ctx.text_center(fonts, R { x: p.x, y: p.y + p.h - 16, w: p.w, h: 14 },
        "定制与改进：shizhelei · Gary_6Stones", 10.0, alpha(palette.muted, 200));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut pm = Pixmap::new(2560, 1600).unwrap();
        pm.fill(Color::TRANSPARENT);
        App::new(pm, 2560, 1600)
    }

    #[test]
    fn small_settings_reach_every_control_without_overlap_or_offscreen_targets() {
        for (w, h) in [(320, 240), (360, 640), (480, 270), (640, 360), (980, 512),
            (1024, 600), (1280, 720), (1920, 1080)] {
            {
                let mut a = App::new(Pixmap::new(w, h).unwrap(), w as i32, h as i32);
                a.settings_open = true;
                let mut reached = Vec::new();
                let mut sliders = Vec::new();
                let panel = settings_rect(&a);
                assert!(panel.x >= 0 && panel.y >= 0 && panel.x + panel.w <= w as i32 && panel.y + panel.h <= h as i32);
                let pages = settings_layout(&a).pages;
                for page in 0..pages {
                    a.settings_page = page;
                    let controls = settings_controls(&a);
                    for (index, (hit, r)) in controls.iter().enumerate() {
                        assert!(panel.contains(r.x, r.y) && panel.contains(r.x + r.w - 1, r.y + r.h - 1), "{w}x{h} {hit:?} {r:?}");
                        assert_eq!(control_at(&a, r.x + r.w / 2, r.y + r.h / 2), Some(Control::Setting(*hit)));
                        for (_, other) in &controls[index + 1..] {
                            assert!(r.x + r.w <= other.x || other.x + other.w <= r.x || r.y + r.h <= other.y || other.y + other.h <= r.y,
                                "overlapping controls: {w}x{h} {r:?} {other:?}");
                        }
                        if !reached.contains(hit) { reached.push(*hit); }
                    }
                    assert!(controls.iter().any(|(hit, _)| *hit == SetHit::Done));
                    for s in settings_sliders(&a) {
                        assert!(s.track.w >= 60, "unusable slider at {w}x{h}");
                        assert!(panel.contains(s.hit.x, s.hit.y) && panel.contains(s.hit.x + s.hit.w - 1, s.hit.y + s.hit.h - 1));
                        assert_eq!(control_at(&a, s.track.x, s.track.y + 9), Some(Control::Slider(s.id)));
                        if !sliders.contains(&s.id) { sliders.push(s.id); }
                    }
                }
                for expected in [SetHit::Autostart, SetHit::WpsDebug,
                    SetHit::WpsRemove, SetHit::WpsRegister, SetHit::ProcessBlock, SetHit::ThemeAccent,
                    SetHit::RightClick,
                    SetHit::ClearLog, SetHit::SysInfo, SetHit::Quit, SetHit::Done] {
                    assert!(reached.contains(&expected), "unreachable {expected:?} at {w}x{h}");
                }
                assert_eq!(sliders.len(), 4);
                for id in sliders { assert!(reached.contains(&SetHit::ResetSlider(id))); }
                assert!(!settings_change_page(&mut a, i32::MAX));
                settings_change_page(&mut a, i32::MIN);
                assert_eq!(settings_layout(&a).page, 0);
                assert!(!control_enabled(&a, Control::Setting(SetHit::SettingsPage(-1))));
            }
        }
    }

    #[test]
    fn settings_page_clamps_after_resize() {
        let mut a = App::new(Pixmap::new(640, 360).unwrap(), 640, 360);
        a.settings_page = usize::MAX;
        assert!(settings_layout(&a).page < settings_layout(&a).pages);
        assert!(settings_controls(&a).iter().any(|(hit, _)| *hit == SetHit::Quit));
        a.screen_h = 1080;
        assert_eq!(settings_layout(&a).page, 0);
        assert_eq!(settings_layout(&a).pages, 1);
    }

    #[test]
    fn small_settings_render_at_native_pixel_density_in_both_themes() {
        let fonts = Fonts::load();
        for scale in [1.0, 1.5, 2.0] {
            let mut a = App::new(Pixmap::new(640, 360).unwrap(), 640, 360);
            a.set_scale(scale);
            a.settings_open = true;
            for preference in [theme::Preference::Light, theme::Preference::Dark] {
                a.apply_theme(theme::Theme { preference, system_dark: false, ..Default::default() });
                for page in 0..settings_layout(&a).pages {
                    a.settings_page = page;
                    let r = settings_rect(&a);
                    let pm = render_region(&a, &fonts, None, r.x, r.y, r.w, r.h).unwrap();
                    assert_eq!(pm.width(), (r.w as f64 * scale).round() as u32);
                    assert_eq!(pm.height(), (r.h as f64 * scale).round() as u32);
                    let done = settings_controls(&a).into_iter().find(|(h, _)| *h == SetHit::Done).unwrap().1;
                    assert_eq!(pm.pixel(((done.x - r.x + 6) as f64 * scale) as u32,
                        ((done.y - r.y + 19) as f64 * scale) as u32).unwrap(),
                        c(a.theme.palette().accent).to_color_u8().premultiply());
                    assert!(a.canvas.data().iter().all(|&b| b == 0));
                }
            }
        }
    }

    #[test]
    fn settings_controls_are_hittable() {
        let mut app = app();
        app.settings_open = true;
        {
            for (hit, r) in settings_controls(&app) {
                if matches!(hit, SetHit::Theme(_)) {
                    assert!(r.h >= 32 && r.w >= 80);
                    assert!(settings_rect(&app).contains(r.x + r.w - 1, r.y + r.h - 1));
                }
                let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
                assert_eq!(settings_hit(&app, x, y), Some(hit), "控件 {:?} 矩形 {:?} 命中失败", hit, r);
                assert_eq!(control_at(&app, x, y), Some(Control::Setting(hit)),
                    "setting buttons must not be captured by slider drag regions");
            }
        }
    }

    #[test]
    fn settings_sliders_hittable_and_map() {
        let app = app();
        for s in settings_sliders(&app) {
            let mid = s.track.x + s.track.w / 2;
            let got = settings_slider_hit(&app, mid, s.track.y + s.track.h / 2);
            assert_eq!(got.map(|d| d.id), Some(s.id), "滑条 {:?} 命中失败", s.id);
            let v = s.value_at(mid);
            let mid_v = (s.min + s.max) / 2.0;
            assert!(
                (v - mid_v).abs() <= s.step * 0.75 + 1.0,
                "中点取值异常 {:?}: {} vs {}",
                s.id,
                v,
                mid_v
            );
        }
    }

    #[test]
    fn sidebar_buttons_are_hittable() {
        let app = app();
        for right in [false, true] {
            for (k, r) in sidebar_buttons(&app, right) {
                assert!(r.contains(r.x + r.w / 2, r.y + r.h / 2), "{:?} 不在自身矩形内", k);
            }
        }
    }

    #[test]
    fn more_menu_items_hittable() {
        let mut app = app();
        app.more_menu_open = true;
        let m = more_menu_rect(&app);
        assert_eq!(
            more_menu_hit(&app, m.x + m.w / 2, m.y + 18),
            Some(MoreHit::Screenshot)
        );
        assert_eq!(more_menu_hit(&app, m.x + m.w / 2, m.y + 54), None);
        app.more_settings_entry = true;
        let m = more_menu_rect(&app);
        assert_eq!(
            more_menu_hit(&app, m.x + m.w / 2, m.y + 54),
            Some(MoreHit::Settings)
        );
    }

    #[test]
    fn themes_cover_every_panel_and_startup_cache_at_multiple_scales() {
        let fonts = Fonts::load();
        for scale in [1.0, 1.5, 2.0] {
            let mut a = App::new(Pixmap::new(1280, 900).unwrap(), 1280, 900);
            a.set_scale(scale);
            for preference in [theme::Preference::Light, theme::Preference::Dark] {
                a.apply_theme(theme::Theme { preference, system_dark: false, ..Default::default() });
                for panel in 0..7 {
                    a.settings_open = panel == 0;
                    a.pen_popup_visible = panel == 1;
                    a.eraser_popup_visible = panel == 2;
                    a.more_menu_open = panel == 3;
                    a.confirm_quit = panel == 4;
                    a.show_diag = panel == 5;
                    a.page_picker = (panel == 6).then(|| crate::page_picker::PagePicker::new(
                        6, 24, crate::page_picker::Source::Wps(1)));
                    let r = match panel {
                        0 => settings_rect(&a), 1 => pen_popup_rect(&a), 2 => eraser_popup_rect(&a),
                        3 => more_menu_rect(&a), 4 => confirm_rect(&a), 5 => diag_rect(&a),
                        _ => page_picker_rect(&a),
                    };
                    let pixels = render_region(&a, &fonts, None, r.x, r.y, r.w, r.h).unwrap();
                    assert_eq!(pixels.pixel((4.0 * scale) as u32, pixels.height() / 2).unwrap(),
                        c(a.theme.palette().paper).to_color_u8().premultiply(),
                        "panel {panel} / {preference:?} / {scale}");
                    assert!(a.canvas.data().iter().all(|&b| b == 0));
                }
            }
            // Render the real cached startup, then change theme while it is visible.
            a.launch = crate::launch::Launch::begin("isolated".into(), true, true, std::time::Instant::now());
            for preference in [theme::Preference::Light, theme::Preference::Dark, theme::Preference::Light] {
                a.apply_theme(theme::Theme { preference, system_dark: false, ..Default::default() });
                let r = launch_rect(&a);
                let pixels = render_region(&a, &fonts, None, r.x, r.y, r.w, r.h).unwrap();
                assert_eq!(pixels.pixel((4.0 * scale) as u32, pixels.height() / 2).unwrap(),
                    c(a.theme.palette().paper).to_color_u8().premultiply());
                assert!(a.launch.card.borrow().is_some());
            }
        }
    }
}


#[cfg(test)]
mod whiteboard_dot_tests {
    use super::*;
    use crate::text::Fonts;
    use tiny_skia::{Color, Pixmap};

    #[test]
    fn dots_render_behind_ink_only_when_enabled() {
        let fonts = Fonts::load();
        let mut app = App::new(Pixmap::new(128, 128).unwrap(), 128, 128);
        app.whiteboard = true;
        app.whiteboard_bg_index = 1;
        app.whiteboard_dots = false;
        let plain = render_region(&app, &fonts, None, 0, 0, 64, 64).unwrap();
        app.whiteboard_dots = true;
        app.whiteboard_dot_size = 4;
        let dotted = render_region(&app, &fonts, None, 0, 0, 64, 64).unwrap();
        assert!(plain.data() != dotted.data());
        assert!(app.canvas.data().iter().all(|byte| *byte == 0));
        let _ = Color::WHITE;
    }
}
