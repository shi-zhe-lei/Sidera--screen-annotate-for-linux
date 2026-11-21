// ============================================================
// Sidera - 手绘 UI（侧边栏 / 弹窗 / 设置面板）与命中测试
// 不再使用 QWidget：所有控件都是几何矩形 + tiny-skia 绘制
// ============================================================
use tiny_skia::{
    BlendMode, Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Shader, Stroke,
    StrokeDash, Transform,
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
    Screenshot,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PopupHit {
    Color(usize),
    PenSize(usize),
    EraserSize(usize),
    ClearAll,
}


#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SetHit {
    EditSlider(SliderId),
    Tab(crate::settings_state::Tab),
    Logs,
    Repository,
    License,
    InspectorPage(i32),
    InspectorClose,
    Theme(theme::Preference),
    ResetSlider(SliderId),
    Autostart,
    WpsRemove,
    WpsRegister,
    ProcessBlock,
    ThemeAccent,
    ClearLog,
    SysInfo,
    Quit,
    Done,
    SettingsPage(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliderId {
    Position,
    AutoExpand,
    Alpha,
    Scale,
    StartupDelay,
    Threshold,
    EraseScale,
}

impl SliderId {
    pub fn default_value(self) -> f32 {
        match self {
            Self::Position => crate::app::DEFAULT_SIDEBAR_POSITION,
            Self::AutoExpand => crate::app::DEFAULT_SIDEBAR_EXPAND_MINUTES as f32,
            Self::Alpha => crate::app::DEFAULT_SIDEBAR_ALPHA as f32,
            Self::Scale => crate::app::DEFAULT_SB_SCALE as f32,
            Self::StartupDelay => crate::app::DEFAULT_STARTUP_DELAY as f32,
            Self::Threshold => crate::app::DEFAULT_LARGE_TOUCH_THRESHOLD as f32,
            Self::EraseScale => crate::app::DEFAULT_LARGE_ERASE_SCALE10 as f32 / 10.0,
        }
    }
}

pub use crate::toolbar::PresentationAction;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PagePickerHit { Wheel, Input, Cancel, Jump }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WhiteboardControl {
    Empty,
    Ellipse,
    Hyperbola,
    Parabola,
    Select,
    Direction(crate::app::ConicKind, crate::app::ConicDirection),
    ZoomSlider,
    SizeSlider,
    ShapeSlider,
    Done,
    Delete,
    SelDone,
    SelCancel,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Editor(crate::value_input::Hit),
    Prompt(PromptHit),
    Process(ProcessHit),
    Dialog(crate::settings_state::DialogHit),
    Picker(crate::settings_state::PickerHit),
    Sidebar(Btn, bool), Popup(PopupHit), Setting(SetHit),
    Slider(SliderId), Confirm(ConfirmHit), DiagClose,
    PageCount(bool), PagePicker(PagePickerHit), Whiteboard(WhiteboardControl),
}

pub fn control_enabled(app: &App, control: Control) -> bool {
    if let Control::Process(hit)=control {
        let max=process_rows(app).len().saturating_sub(process_visible_rows(app));
        return match hit {
            ProcessHit::ScrollUp=>app.process_scroll.min(max)>0,
            ProcessHit::ScrollDown=>app.process_scroll.min(max)<max,
            ProcessHit::Add=>!app.process_input.trim().is_empty()&&app.process_rules.len()<64,
            _=>true,
        };
    }
    if let Control::Setting(SetHit::SettingsPage(delta)) = control {
        let l = settings_layout(app);
        return (delta < 0 && l.page > 0) || (delta > 0 && l.page + 1 < l.pages);
    }
    if matches!(control, Control::PageCount(_)) { return app.page_picker_context().is_some(); }
    if matches!(control, Control::Sidebar(Btn::Restore, _)) { return app.can_restore_cleared(); }
    if matches!(control, Control::Whiteboard(
        WhiteboardControl::ZoomSlider | WhiteboardControl::SizeSlider
        | WhiteboardControl::ShapeSlider | WhiteboardControl::Done | WhiteboardControl::Delete
    )) {
        let editing = app.whiteboard && app.coord.is_some_and(|c| c.editing);
        return editing && match control {
            Control::Whiteboard(WhiteboardControl::ShapeSlider) =>
                app.coord.is_some_and(|c| c.conic != crate::app::ConicKind::None),
            _ => true,
        };
    }
    if matches!(control, Control::Whiteboard(WhiteboardControl::SelDone | WhiteboardControl::SelCancel)) {
        return app.whiteboard && app.floating.is_some();
    }
    true
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
    if app.show_input_group_prompt { return input_group_prompt_hit(app,x,y).map(Control::Prompt); }
    if app.settings_ui.editor.is_some() {
        return crate::value_input::controls(app).into_iter().find_map(|(hit,r)|r.contains(x,y).then_some(Control::Editor(hit)));
    }
    if app.settings_ui.dialog.is_some() {
        return crate::settings_panel::dialog_controls(app).into_iter().find_map(|(hit, r)| r.contains(x,y).then_some(Control::Dialog(hit)));
    }
    if app.settings_ui.picker.is_some() {
        return crate::settings_panel::picker_controls(app).into_iter().find_map(|(hit, r)| r.contains(x,y).then_some(Control::Picker(hit)));
    }
    if app.confirm_quit { return confirm_hit(app, x, y).map(Control::Confirm); }
    if app.show_diag {
        return crate::settings_panel::inspector_controls(app).into_iter().find_map(|(hit, r)| r.contains(x,y).then_some(Control::Setting(hit)));
    }
    if app.process_block_open { return process_block_hit(app,x,y).map(Control::Process); }
    if app.settings_open {
        return settings_slider_hit(app, x, y).map(|s| Control::Slider(s.id))
            .or_else(|| settings_hit(app, x, y).map(Control::Setting));
    }
    if app.whiteboard {
        if let Some(hit) = whiteboard_hit(app, x, y) { return Some(Control::Whiteboard(hit)); }
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
    if app.settings_ui.editor.is_some() && !app.show_input_group_prompt {
        return match control {
            Control::Editor(hit)=>crate::value_input::controls(app).into_iter().find(|(h,_)|*h==hit).map(|(_,r)|r),
            _=>None,
        };
    }
    if app.show_input_group_prompt {
        return match control {
            Control::Prompt(hit) => input_group_prompt_controls(app).into_iter().find(|(h,_)|*h==hit).map(|(_,r)|r),
            _ => None,
        };
    }
    if app.settings_ui.modal() {
        return match control {
            Control::Dialog(hit) => crate::settings_panel::dialog_controls(app).into_iter().find(|(h,_)| *h==hit).map(|(_,r)|r),
            Control::Picker(hit) => crate::settings_panel::picker_controls(app).into_iter().find(|(h,_)| *h==hit).map(|(_,r)|r),
            _ => None,
        };
    }
    if app.show_diag {
        return match control {
            Control::Setting(hit) => crate::settings_panel::inspector_controls(app).into_iter().find(|(h,_)|*h==hit).map(|(_,r)|r),
            Control::DiagClose => crate::settings_panel::inspector_controls(app).into_iter().find(|(h,_)|*h==SetHit::InspectorClose).map(|(_,r)|r),
            _ => None,
        };
    }
    if app.confirm_quit && !matches!(control, Control::Confirm(_)) { return None; }
    if app.settings_open && !matches!(control, Control::Setting(_) | Control::Slider(_) | Control::Confirm(_) | Control::DiagClose) { return None; }
    match control {
        Control::Process(hit) if app.process_block_open => process_block_controls(app).into_iter().find(|(h,_)|*h==hit).map(|(_,r)|r),
        Control::PageCount(right) if app.sidebar_page_visible() && !app.collapsed => Some(sidebar_page_rect(app, right)),
        Control::PagePicker(hit) if app.page_picker.is_some() => Some(page_picker_control_rect(app, hit)),
        Control::Sidebar(b, right) => sidebar_buttons(app, right).into_iter().find(|(k, _)| *k == b).map(|(_, r)| r),
        Control::Setting(hit) if app.settings_open => settings_controls(app).into_iter().find(|(k, _)| *k == hit).map(|(_, r)| r),
        Control::Slider(id) if app.settings_open => settings_sliders(app).into_iter().find(|s| s.id == id).map(|s| s.hit),
        Control::Whiteboard(WhiteboardControl::Empty) if app.whiteboard => Some(wb_button_rect(app, 0)),
        Control::Whiteboard(WhiteboardControl::Ellipse) if app.whiteboard => Some(wb_button_rect(app, 1)),
        Control::Whiteboard(WhiteboardControl::Hyperbola) if app.whiteboard => Some(wb_button_rect(app, 2)),
        Control::Whiteboard(WhiteboardControl::Parabola) if app.whiteboard => Some(wb_button_rect(app, 3)),
        Control::Whiteboard(WhiteboardControl::Select) if app.whiteboard => Some(wb_button_rect(app, 4)),
        Control::Whiteboard(WhiteboardControl::Direction(kind, direction)) if app.whiteboard => {
            kind.directions().iter().position(|(d, _)| *d == direction)
                .map(|index| direction_option_rect(app, kind, index))
        }
        Control::Whiteboard(WhiteboardControl::ZoomSlider) if app.whiteboard => Some(wb_zoom_slider_rect(app)),
        Control::Whiteboard(WhiteboardControl::SizeSlider) if app.whiteboard => Some(wb_size_slider_rect(app)),
        Control::Whiteboard(WhiteboardControl::ShapeSlider) if app.whiteboard => Some(wb_shape_slider_rect(app)),
        Control::Whiteboard(WhiteboardControl::Done) if app.whiteboard => Some(wb_done_rect(app)),
        Control::Whiteboard(WhiteboardControl::Delete) if app.whiteboard => Some(wb_delete_rect(app)),
        Control::Whiteboard(WhiteboardControl::SelDone) if app.whiteboard => Some(selection_done_rect(app)),
        Control::Whiteboard(WhiteboardControl::SelCancel) if app.whiteboard => Some(selection_cancel_rect(app)),
        Control::Popup(PopupHit::Color(i)) if app.pen_popup_visible && i < 10 => {
            let p = pen_popup_rect(app);
            Some(R { x: p.x + 16 + (i % 4) as i32 * 48, y: p.y + 46 + (i / 4) as i32 * 48, w: 38, h: 38 })
        }
        Control::Popup(PopupHit::PenSize(i)) if app.pen_popup_visible && i < crate::app::PEN_SIZES.len() => {
            let p = pen_popup_rect(app); let bw = tier_button_width(p.w, crate::app::PEN_SIZES.len());
            Some(R { x: p.x + 16 + i as i32 * (bw + 10), y: p.y + 214, w: bw, h: 48 })
        }
        Control::Popup(PopupHit::EraserSize(i)) if app.eraser_popup_visible && i < crate::app::ERASER_SIZES.len() => {
            let p = eraser_popup_rect(app); let bw = tier_button_width(p.w, crate::app::ERASER_SIZES.len());
            Some(R { x: p.x + 16 + i as i32 * (bw + 10), y: p.y + 46, w: bw, h: 48 })
        }
        Control::Popup(PopupHit::ClearAll) if app.eraser_popup_visible => {
            let p = eraser_popup_rect(app);
            Some(R { x: p.x + 16, y: p.y + 110, w: p.w - 32, h: 48 })
        }
        Control::Confirm(hit) if app.confirm_quit => {
            confirm_controls(app).into_iter().find(|(h,_)|*h==hit).map(|(_,r)|r)
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
    if matches!(press.control, Control::Slider(_) | Control::PagePicker(PagePickerHit::Wheel)
        | Control::Picker(crate::settings_state::PickerHit::Field | crate::settings_state::PickerHit::Hue | crate::settings_state::PickerHit::Edit(_))
        | Control::Editor(_) | Control::Setting(SetHit::EditSlider(_) | SetHit::InspectorPage(_))
        | Control::Process(ProcessHit::ScrollUp | ProcessHit::ScrollDown)
        | Control::PagePicker(PagePickerHit::Input)) { return; }
    let radius = match press.control {
        Control::Popup(PopupHit::Color(_)) => 19.0,
        Control::Sidebar(_, _) => (r.w as f32 * 0.24).min(10.0),
        _ => 9.0,
    };
    let inset = R { x: r.x + 2, y: r.y + 2, w: r.w - 4, h: r.h - 4 };
    let filled_action = matches!(press.control, Control::Setting(SetHit::Done)
        | Control::DiagClose | Control::PagePicker(PagePickerHit::Jump))
        || matches!(press.control, Control::Setting(SetHit::Theme(p)) if p == app.theme.preference);
    let feedback = if matches!(press.control, Control::Setting(SetHit::Quit)) {
        palette.danger
    } else if filled_action || matches!(press.control, Control::Sidebar(button, _) if sidebar_button_selected(app, button)) {
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

fn rounded_path(x: f32, y: f32, w: f32, h: f32, left: f32, right: f32) -> Option<tiny_skia::Path> {
    let max_radius = (w.min(h) / 2.0).max(0.0);
    let left = left.clamp(0.0, max_radius);
    let right = right.clamp(0.0, max_radius);
    let mut pb = PathBuilder::new();
    pb.move_to(x + left, y);
    pb.line_to(x + w - right, y);
    pb.quad_to(x + w, y, x + w, y + right);
    pb.line_to(x + w, y + h - right);
    pb.quad_to(x + w, y + h, x + w - right, y + h);
    pb.line_to(x + left, y + h);
    pb.quad_to(x, y + h, x, y + h - left);
    pb.line_to(x, y + left);
    pb.quad_to(x, y, x + left, y);
    pb.close();
    pb.finish()
}

/// At fractional desktop scales a 1-logical-pixel line becomes 1.25/1.5
/// device pixels and looks soft. Use exactly one device pixel for those thin
/// strokes; integer scales keep their intended 1/2/3 device-pixel widths.
fn crisp_line_width(tr: &Transform, w: f32) -> f32 {
    let scale = (tr.sx.abs() + tr.sy.abs()) * 0.5;
    if scale <= 0.0 || (scale - scale.round()).abs() < 0.01 {
        return w;
    }
    let dev = w * scale;
    if (0.6..1.8).contains(&dev) { 1.0 / scale } else { w }
}

/// Snap a thin rounded rectangle's edges to half device pixels. Only affects
/// uniform, axis-aligned transforms and ~1-device-pixel strokes.
fn snap_stroke_rect(tr: &Transform, r: R, width: f32) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
    if tr.kx.abs() > 0.01 || tr.ky.abs() > 0.01 || tr.sx == 0.0 || tr.sy == 0.0
        || (tr.sx - tr.sy).abs() > 0.01 {
        return (x, y, w, h);
    }
    let dev_w = width * tr.sx.abs();
    if (dev_w - 1.0).abs() > 0.2 {
        return (x, y, w, h);
    }
    let snap = |v: f32| (v - 0.5).round() + 0.5;
    let x0 = snap(x * tr.sx + tr.tx);
    let y0 = snap(y * tr.sy + tr.ty);
    let x1 = snap((x + w) * tr.sx + tr.tx);
    let y1 = snap((y + h) * tr.sy + tr.ty);
    let nx = (x0 - tr.tx) / tr.sx;
    let ny = (y0 - tr.ty) / tr.sy;
    let nw = ((x1 - tr.tx) / tr.sx - nx).max(1.0);
    let nh = ((y1 - tr.ty) / tr.sy - ny).max(1.0);
    (nx, ny, nw, nh)
}

/// Align 1-device-pixel horizontal/vertical lines to pixel centers. A stroke at
/// an integer coordinate covers two rows/columns at 50% each and looks fuzzy;
/// adding half a device pixel makes separators crisp without changing geometry.
fn snap_axis_line(tr: &Transform, mut a: (f32, f32), mut b: (f32, f32), w: f32) -> ((f32, f32), (f32, f32)) {
    if tr.kx.abs() > 0.01 || tr.ky.abs() > 0.01 || tr.sx == 0.0 || tr.sy == 0.0 {
        return (a, b);
    }
    let dev_w = w * ((tr.sx.abs() + tr.sy.abs()) * 0.5);
    if (dev_w - 1.0).abs() > 0.2 {
        return (a, b);
    }
    if (a.1 - b.1).abs() < 0.01 {
        let snapped = (a.1 * tr.sy + tr.ty).floor() + 0.5;
        let y = (snapped - tr.ty) / tr.sy;
        a.1 = y; b.1 = y;
    } else if (a.0 - b.0).abs() < 0.01 {
        let snapped = (a.0 * tr.sx + tr.tx).floor() + 0.5;
        let x = (snapped - tr.tx) / tr.sx;
        a.0 = x; b.0 = x;
    }
    (a, b)
}

pub struct Ctx<'a> {
    pub pm: &'a mut Pixmap,
    pub tr: Transform,
    pub palette: theme::Palette,
}

impl<'a> Ctx<'a> {
    pub fn fill_round(&mut self, r: R, radius: f32, c: Color) {
        self.fill_round_sides(r, radius, radius, c);
    }
    fn fill_round_sides(&mut self, r: R, left: f32, right: f32, c: Color) {
        if let Some(p) = rounded_path(r.x as f32, r.y as f32, r.w as f32, r.h as f32, left, right) {
            self.pm
                .fill_path(&p, &solid(c), FillRule::Winding, self.tr, None);
        }
    }
    pub fn stroke_round(&mut self, r: R, radius: f32, c: Color, width: f32) {
        // A 1-device-pixel rounded border looks soft at fractional scales
        // (125%/150%) when its edges land between pixels. Snap the path centre
        // to half-pixel coordinates so the stroke covers exactly one pixel.
        let width = crisp_line_width(&self.tr, width);
        let (x, y, w, h) = snap_stroke_rect(&self.tr, r, width);
        if let Some(p) = rounded_path(x, y, w, h, radius, radius) {
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
        let w = crisp_line_width(&self.tr, w);
        let (a, b) = snap_axis_line(&self.tr, a, b, w);
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
    pub fn stroke_polyline(&mut self, points: &[(f32, f32)], c: Color, width: f32, closed: bool) {
        if points.len() < 2 { return; }
        let mut pb = PathBuilder::new();
        pb.move_to(points[0].0, points[0].1);
        for &(x, y) in &points[1..] { pb.line_to(x, y); }
        if closed { pb.close(); }
        if let Some(p) = pb.finish() {
            let st = Stroke { width, line_cap: tiny_skia::LineCap::Round,
                line_join: tiny_skia::LineJoin::Round, ..Default::default() };
            self.pm.stroke_path(&p, &solid(c), &st, self.tr, None);
        }
    }
    /// Yellow/black hazard dashes for the rectangular selection outline.
    pub fn stroke_hazard(&mut self, points: &[(f32, f32)], closed: bool) {
        if points.len() < 2 { return; }
        let mut pb = PathBuilder::new();
        pb.move_to(points[0].0, points[0].1);
        for &(x, y) in &points[1..] { pb.line_to(x, y); }
        if closed { pb.close(); }
        let Some(p) = pb.finish() else { return };
        for (color, offset) in [(Color::from_rgba8(255, 193, 7, 255), 0.0f32),
                                (Color::from_rgba8(15, 15, 15, 255), 8.0f32)] {
            let st = Stroke {
                width: 2.0,
                dash: StrokeDash::new(vec![8.0, 8.0], offset),
                line_cap: tiny_skia::LineCap::Butt,
                ..Default::default()
            };
            self.pm.stroke_path(&p, &solid(color), &st, self.tr, None);
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
pub(crate) fn surface(ctx: &mut Ctx, r: R, radius: f32, fill: Color, border: Color) {
    ctx.fill_round(r, radius, fill);
    ctx.stroke_round(surface_outline(r), (radius - 1.0).max(1.0), border, 1.0);
}

fn surface_outline(r: R) -> R {
    R { x: r.x + 1, y: r.y + 1, w: r.w - 2, h: r.h - 2 }
}

pub(crate) fn panel_with_sidebar(ctx: &mut Ctx, r: R, radius: f32, sidebar_width: i32) {
    let palette = ctx.palette;
    let frame = surface_outline(r);
    let inner_radius = (radius - 1.0).max(1.0);
    let seam = r.x + sidebar_width;
    ctx.fill_round(r, radius, c(palette.paper));
    let sidebar = R { w: seam - frame.x, ..frame };
    ctx.fill_round_sides(sidebar, inner_radius, 0.0, c(theme::mix(palette.paper, palette.surface, 0.65)));

    // End the divider on the same device-pixel centers as the frame. Painting
    // the shared outline last keeps both junctions flush at fractional scales.
    let width = crisp_line_width(&ctx.tr, 1.0);
    let (_, top, _, height) = snap_stroke_rect(&ctx.tr, frame, width);
    let (start, _) = snap_axis_line(&ctx.tr, (seam as f32, top), (seam as f32, top + height), width);
    if let Some(divider) = tiny_skia::Rect::from_xywh(start.0 - width / 2.0, top, width, height) {
        ctx.pm.fill_rect(divider, &solid(alpha(palette.border, 150)), ctx.tr, None);
    }
    ctx.stroke_round(frame, inner_radius, c(palette.border), 1.0);
}

pub(crate) fn panel(ctx: &mut Ctx, r: R, radius: f32, opacity: f32) {
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

#[cfg(unix)]
pub(crate) fn sidebar_preview_strip(app: &App, right: bool) -> R {
    // Include the widest supported sidebar, so reducing the scale clears its
    // previous footprint. The full height covers every position of the slider.
    let width = ((crate::app::SB_BASE_W * 1.4).ceil() as i32 + 6).min(app.screen_w);
    R { x: if right { app.screen_w - width } else { 0 }, y: 0, w: width, h: app.screen_h }
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
        return vec![(Btn::Collapse, sb)];
    }
    expanded_sidebar_buttons(app, right)
}

fn expanded_sidebar_buttons(app: &App, right: bool) -> Vec<(Btn, R)> {
    let sb = expanded_sidebar_rect(app, right);
    let btn = app.sb_btn();
    let bx = sb.x + (sb.w - btn) / 2;
    let mut y = sb.y + 12;
    let mut v = Vec::new();
    // Whiteboard has no collapse button; the remaining buttons move up.
    let top: &[Btn] = if app.whiteboard { &[Btn::Cursor, Btn::Pen] }
        else { &[Btn::Collapse, Btn::Cursor, Btn::Pen] };
    for &k in top {
        v.push((k, R { x: bx, y, w: btn, h: btn }));
        y += btn + 6;
    }
    if app.drawing_tools.fraction() >= 1.0 { v.extend(drawing_tool_buttons(app, right)); }
    y += app.drawing_tools_height();
    for k in [Btn::Prev, Btn::Next, Btn::Exit, Btn::Whiteboard, Btn::Screenshot] {
        v.push((k, R { x: bx, y, w: btn, h: btn }));
        y += btn + 6;
        if k == Btn::Prev { y += app.sidebar_page_space(); }
    }
    v
}

fn drawing_tool_buttons(app: &App, right: bool) -> Vec<(Btn, R)> {
    let sb = expanded_sidebar_rect(app, right);
    let btn = app.sb_btn();
    let clear_or_restore = if app.can_restore_cleared() { Btn::Restore } else { Btn::Clear };
    let top = if app.whiteboard { 2 } else { 3 };
    [Btn::Eraser, clear_or_restore, Btn::Undo].into_iter().enumerate().map(|(i, k)|
        (k, R { x: sb.x + (sb.w - btn) / 2, y: sb.y + 12 + (top + i as i32) * (btn + 6),
            w: btn, h: btn })).collect()
}

pub fn sidebar_page_rect(app: &App, right: bool) -> R {
    let sb = expanded_sidebar_rect(app, right);
    let top = if app.whiteboard { 3 } else { 4 };
    R { x: sb.x + 4, y: sb.y + 12 + top * (app.sb_btn() + 6) + app.drawing_tools_height(),
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

fn page_picker_has_thumbnail(app: &App) -> bool {
    app.page_picker.as_ref().is_some_and(|p| matches!(p.source, crate::page_picker::Source::Wps(_)))
}

pub fn page_picker_thumb_rect(app: &App) -> R {
    let p = page_picker_rect(app);
    R { x: p.x + 16, y: p.y + 76, w: p.w - 32, h: 128 }
}

pub fn page_picker_rect(app: &App) -> R {
    let w = 260;
    let h = if page_picker_has_thumbnail(app) { 512 } else { 376 };
    let sb = sidebar_rect(app, app.popup_on_right);
    let anchor = sidebar_page_rect(app, app.popup_on_right);
    R { x: (if app.popup_on_right { sb.x - w - 8 } else { sb.x + sb.w + 8 })
            .clamp(0, (app.screen_w - w).max(0)),
        y: (anchor.y + anchor.h / 2 - h / 2).clamp(0, (app.screen_h - h).max(0)), w, h }
}

pub fn page_picker_control_rect(app: &App, hit: PagePickerHit) -> R {
    let p = page_picker_rect(app);
    let (wheel_y, footer_y) = if page_picker_has_thumbnail(app) { (212, 452) } else { (74, 314) };
    match hit {
        PagePickerHit::Wheel => R { x: p.x + 16, y: p.y + wheel_y, w: p.w - 32, h: 220 },
        PagePickerHit::Input => R {x:p.x+p.w-104,y:p.y+14,w:88,h:40},
        PagePickerHit::Cancel => R { x: p.x + 16, y: p.y + footer_y, w: 92, h: 44 },
        PagePickerHit::Jump => R { x: p.x + 118, y: p.y + footer_y, w: 126, h: 44 },
    }
}

pub fn page_picker_hit(app: &App, x: i32, y: i32) -> Option<PagePickerHit> {
    [PagePickerHit::Wheel, PagePickerHit::Input, PagePickerHit::Cancel, PagePickerHit::Jump].into_iter()
        .find(|hit| page_picker_control_rect(app, *hit).contains(x, y))
}

fn tier_button_width(popup_w: i32, tiers: usize) -> i32 {
    let gaps = 10 * (tiers.max(1) as i32 - 1);
    (popup_w - 32 - gaps) / tiers.max(1) as i32
}

pub fn pen_popup_rect(app: &App) -> R {
    let w = 296;
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
    let w = 308;
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
    let tiers = crate::app::PEN_SIZES.len();
    let bw = tier_button_width(p.w, tiers);
    for i in 0..tiers {
        let bx = p.x + 16 + i as i32 * (bw + 10);
        let rr = R { x: bx, y: sy, w: bw, h: 48 };
        if rr.contains(x, y) {
            return Some(PopupHit::PenSize(i));
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
    let tiers = crate::app::ERASER_SIZES.len();
    let bw = tier_button_width(p.w, tiers);
    for i in 0..tiers {
        let bx = p.x + 16 + i as i32 * (bw + 10);
        let rr = R { x: bx, y: sy, w: bw, h: 48 };
        if rr.contains(x, y) {
            return Some(PopupHit::EraserSize(i));
        }
    }
    let cy = p.y + 110;
    let rr = R { x: p.x + 16, y: cy, w: p.w - 32, h: 48 };
    if rr.contains(x, y) {
        return Some(PopupHit::ClearAll);
    }
    None
}


pub fn whiteboard_bar_rect(app: &App) -> R {
    let w = 560.min((app.screen_w - 32).max(240));
    R { x: (app.screen_w - w) / 2, y: app.screen_h - 74, w, h: 58 }
}

const WB_BUTTONS: [(crate::app::ConicKind, &str); 4] = [
    (crate::app::ConicKind::None, "空坐标系"),
    (crate::app::ConicKind::Ellipse, "椭圆"),
    (crate::app::ConicKind::Hyperbola, "双曲线"),
    (crate::app::ConicKind::Parabola, "抛物线"),
];

fn wb_button_rect(app: &App, index: usize) -> R {
    let bar = whiteboard_bar_rect(app);
    let pad = 12;
    let gap = 6;
    const BUTTONS: i32 = 5;
    let bw = ((bar.w - pad * 2 - gap * (BUTTONS - 1)) / BUTTONS).max(1);
    R { x: bar.x + pad + index as i32 * (bw + gap), y: bar.y + 8, w: bw, h: 42 }
}

fn wb_shape_index(kind: crate::app::ConicKind) -> Option<usize> {
    match kind {
        crate::app::ConicKind::Ellipse => Some(1),
        crate::app::ConicKind::Hyperbola => Some(2),
        crate::app::ConicKind::Parabola => Some(3),
        crate::app::ConicKind::None => None,
    }
}

pub fn direction_menu_rect(app: &App, kind: crate::app::ConicKind) -> R {
    let Some(index) = wb_shape_index(kind) else { return R { x: 0, y: 0, w: 1, h: 1 } };
    let button = wb_button_rect(app, index);
    let count = kind.directions().len() as i32;
    let w = 136;
    let h = count * 42 + 12;
    let x = (button.x + button.w / 2 - w / 2).clamp(0, (app.screen_w - w).max(0));
    let y = (button.y - h - 8).clamp(8, (app.screen_h - h).max(8));
    R { x, y, w, h }
}

fn direction_option_rect(app: &App, kind: crate::app::ConicKind, index: usize) -> R {
    let menu = direction_menu_rect(app, kind);
    R { x: menu.x + 6, y: menu.y + 6 + index as i32 * 42, w: menu.w - 12, h: 38 }
}

pub fn selection_bar_rect(app: &App) -> R {
    let bar = whiteboard_bar_rect(app);
    R { x: (app.screen_w - 220) / 2, y: (bar.y - 66).max(8), w: 220, h: 52 }
}

fn selection_cancel_rect(app: &App) -> R {
    let bar = selection_bar_rect(app);
    R { x: bar.x + 8, y: bar.y + 7, w: 96, h: 38 }
}

fn selection_done_rect(app: &App) -> R {
    let bar = selection_bar_rect(app);
    R { x: bar.x + bar.w - 104, y: bar.y + 7, w: 96, h: 38 }
}

fn wb_has_conic(app: &App) -> bool {
    app.coord.is_some_and(|c| c.conic != crate::app::ConicKind::None)
}

pub fn whiteboard_edit_bar_rect(app: &App) -> R {
    let bar = whiteboard_bar_rect(app);
    let has_conic = wb_has_conic(app);
    let w = if has_conic { 760 } else { 600 };
    let w = w.min((app.screen_w - 32).max(420));
    R { x: (app.screen_w - w) / 2, y: (bar.y - 86).max(8), w, h: 68 }
}

fn wb_done_rect(app: &App) -> R {
    let bar = whiteboard_edit_bar_rect(app);
    R { x: bar.x + bar.w - 84, y: bar.y + 15, w: 72, h: 38 }
}

fn wb_delete_rect(app: &App) -> R {
    let done = wb_done_rect(app);
    R { x: done.x - 82, y: done.y, w: 72, h: 38 }
}

fn wb_slider_count(app: &App) -> i32 {
    if wb_has_conic(app) { 3 } else { 2 }
}

fn wb_slider_slot(app: &App, index: i32) -> R {
    let bar = whiteboard_edit_bar_rect(app);
    let left = bar.x + 16;
    let right = (wb_delete_rect(app).x - 16).max(left + 60);
    let n = wb_slider_count(app);
    let gap = 16;
    let w = ((right - left - gap * (n - 1)) / n).max(40);
    R { x: left + index * (w + gap), y: bar.y + 6, w, h: 54 }
}

fn wb_slider_track(slot: R) -> R {
    R { x: slot.x, y: slot.y + 30, w: slot.w, h: 18 }
}

fn wb_zoom_slider_rect(app: &App) -> R {
    wb_slider_track(wb_slider_slot(app, 0))
}

fn wb_shape_slider_rect(app: &App) -> R {
    wb_slider_track(wb_slider_slot(app, 1))
}

fn wb_size_slider_rect(app: &App) -> R {
    let index = if wb_has_conic(app) { 2 } else { 1 };
    wb_slider_track(wb_slider_slot(app, index))
}

pub fn coord_origin(app: &App) -> Option<(f32, f32)> {
    app.coord.as_ref().map(|c| (c.origin_x, c.origin_y))
}

pub fn coord_rotate_handle(app: &App) -> Option<(f32, f32)> {
    // Fixed screen-space handle above the origin: the conic foci can sit on
    // either axis, so an axis-aligned handle would often overlap them.
    app.coord.as_ref().map(|c| (c.origin_x, c.origin_y - 76.0))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CoordHandle { Origin, Rotate }

pub fn coord_handle_at(app: &App, x: i32, y: i32) -> Option<CoordHandle> {
    let c = app.coord.as_ref()?;
    if !c.editing || app.conic_menu.is_some() { return None; }
    let near = |p: (f32, f32), radius: f32| {
        let dx = x as f32 - p.0;
        let dy = y as f32 - p.1;
        dx * dx + dy * dy <= radius * radius
    };
    if let Some(p) = coord_rotate_handle(app) {
        if near(p, 18.0) { return Some(CoordHandle::Rotate); }
    }
    if near((c.origin_x, c.origin_y), 20.0) { return Some(CoordHandle::Origin); }
    None
}

pub fn whiteboard_hit(app: &App, x: i32, y: i32) -> Option<WhiteboardControl> {
    if !app.whiteboard { return None; }
    if let Some(kind) = app.conic_menu {
        for (i, (_, _)) in kind.directions().iter().enumerate() {
            if direction_option_rect(app, kind, i).contains(x, y) {
                return Some(WhiteboardControl::Direction(kind, kind.directions()[i].0));
            }
        }
    }
    if app.coord.as_ref().is_some_and(|c| c.editing) && app.conic_menu.is_none() {
        if wb_done_rect(app).contains(x, y) { return Some(WhiteboardControl::Done); }
        if wb_delete_rect(app).contains(x, y) { return Some(WhiteboardControl::Delete); }
        if wb_zoom_slider_rect(app).contains(x, y) { return Some(WhiteboardControl::ZoomSlider); }
        if wb_size_slider_rect(app).contains(x, y) { return Some(WhiteboardControl::SizeSlider); }
        if wb_has_conic(app) && wb_shape_slider_rect(app).contains(x, y) {
            return Some(WhiteboardControl::ShapeSlider);
        }
    }
    if app.floating.is_some() {
        if selection_cancel_rect(app).contains(x, y) { return Some(WhiteboardControl::SelCancel); }
        if selection_done_rect(app).contains(x, y) { return Some(WhiteboardControl::SelDone); }
    }
    for (i, (kind, _)) in WB_BUTTONS.iter().enumerate() {
        if wb_button_rect(app, i).contains(x, y) {
            return Some(match kind {
                crate::app::ConicKind::None => WhiteboardControl::Empty,
                crate::app::ConicKind::Ellipse => WhiteboardControl::Ellipse,
                crate::app::ConicKind::Hyperbola => WhiteboardControl::Hyperbola,
                crate::app::ConicKind::Parabola => WhiteboardControl::Parabola,
            });
        }
    }
    if wb_button_rect(app, 4).contains(x, y) { return Some(WhiteboardControl::Select); }
    None
}

pub fn whiteboard_ui_contains(app: &App, x: i32, y: i32) -> bool {
    if !app.whiteboard { return false; }
    if whiteboard_bar_rect(app).contains(x, y) { return true; }
    if let Some(kind) = app.conic_menu {
        if direction_menu_rect(app, kind).contains(x, y) { return true; }
    }
    if app.coord.as_ref().is_some_and(|c| c.editing) && app.conic_menu.is_none()
        && whiteboard_edit_bar_rect(app).contains(x, y) { return true; }
    if app.floating.is_some() && selection_bar_rect(app).contains(x, y) { return true; }
    if let Some(c) = app.coord.as_ref().filter(|c| c.editing && app.conic_menu.is_none()) {
        let near = |p: (f32, f32), radius: f32| {
            let dx = x as f32 - p.0; let dy = y as f32 - p.1;
            dx * dx + dy * dy <= radius * radius
        };
        if near((c.origin_x, c.origin_y), 20.0) { return true; }
        if let Some(p) = coord_rotate_handle(app) { if near(p, 18.0) { return true; } }
    }
    false
}

fn slider_value(rect: R, x: i32) -> f32 {
    ((x - rect.x) as f32 / rect.w.max(1) as f32).clamp(0.0, 1.0)
}

pub fn coord_zoom_at(app: &App, x: i32) -> f32 {
    12.0 + slider_value(wb_zoom_slider_rect(app), x) * 108.0
}
pub fn coord_size_at(app: &App, x: i32) -> f32 {
    slider_value(wb_size_slider_rect(app), x)
}
pub fn coord_shape_at(app: &App, x: i32) -> f32 {
    slider_value(wb_shape_slider_rect(app), x)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProcessHit {
    Enable,
    View(crate::settings_state::ProcessView),
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


pub fn process_block_rect(app: &App) -> R { crate::settings_panel::process_rect(app) }
pub fn process_visible_rows(app: &App) -> usize { crate::settings_panel::process_visible_rows(app) }

pub fn process_rows(app: &App) -> Vec<(ProcessRow, String, String)> {
    let mut rows = Vec::new();
    if app.settings_ui.process_view==crate::settings_state::ProcessView::Rules {
        for (index,rule) in app.process_rules.iter().enumerate() {
            rows.push((ProcessRow::Rule(index),rule.clone(),if rule.contains('/') {"按完整路径匹配"}else{"按进程名称匹配"}.into()));
        }
        return rows;
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

pub fn process_block_controls(app: &App) -> Vec<(ProcessHit, R)> { crate::settings_panel::process_controls(app) }

pub fn process_block_hit(app: &App, x: i32, y: i32) -> Option<ProcessHit> {
    if !app.process_block_open || !process_block_rect(app).contains(x, y) {
        return None;
    }
    process_block_controls(app)
        .into_iter()
        .find_map(|(hit, rect)| rect.contains(x, y).then_some(hit))
}

pub fn settings_rect(app: &App) -> R { crate::settings_panel::rect(app) }
#[derive(Clone, Copy, Debug)]
pub struct SettingsLayout { pub rows: usize, pub page: usize, pub pages: usize, pub stride: i32 }
pub fn settings_layout(app: &App) -> SettingsLayout { crate::settings_panel::layout(app) }
pub fn settings_change_page(app: &mut App, delta: i32) -> bool { crate::settings_panel::change_page(app, delta) }
pub fn settings_sliders(app: &App) -> Vec<SliderDef> { crate::settings_panel::sliders(app) }
pub fn settings_slider_hit(app: &App, x: i32, y: i32) -> Option<SliderDef> {
    settings_sliders(app).into_iter().find(|s| s.hit.contains(x,y))
}
pub fn settings_controls(app: &App) -> Vec<(SetHit, R)> { crate::settings_panel::controls(app) }
pub fn settings_hit(app: &App, x: i32, y: i32) -> Option<SetHit> {
    settings_controls(app).into_iter().find_map(|(hit,r)|r.contains(x,y).then_some(hit))
}

// ---------------- 退出确认 / 系统诊断 覆盖层 ----------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConfirmHit {
    Yes,
    No,
}

pub fn confirm_rect(app: &App) -> R {
    let (width, height) = crate::settings_panel::available_size(app);
    let w=440.min((width-24).max(1)); let h=220.min((height-24).max(1));
    R {x:(width-w)/2,y:(height-h)/2,w,h}
}
pub fn confirm_controls(app: &App) -> Vec<(ConfirmHit,R)> {
    let p=confirm_rect(app); let w=(p.w-52)/2;
    vec![(ConfirmHit::No,R {x:p.x+20,y:p.y+p.h-64,w,h:44}),
        (ConfirmHit::Yes,R {x:p.x+32+w,y:p.y+p.h-64,w,h:44})]
}
pub fn confirm_hit(app: &App,x:i32,y:i32)->Option<ConfirmHit> {
    confirm_controls(app).into_iter().find_map(|(hit,r)|r.contains(x,y).then_some(hit))
}
pub fn draw_confirm(app:&App,fonts:&Fonts,ctx:&mut Ctx) {
    crate::settings_panel::layer(app,ctx,crate::settings_state::ease(app.settings_ui.overlay_at,160),|ctx| {
        let p=confirm_rect(app); let palette=ctx.palette;
        panel(ctx,p,12.0,1.0);
        ctx.text(fonts,"退出 Sidera？",(p.x+24) as f32,(p.y+42) as f32,19.0,c(palette.text));
        let lines=crate::settings_panel::wrapped(fonts,"退出后当前批注不会保留。\n请先截图保存需要的内容。",14.0,p.w-48);
        for (i,line) in lines.iter().enumerate() {
            if 72+i as i32*24>p.h-72 {break;}
            ctx.text(fonts,line,(p.x+24) as f32,(p.y+72+i as i32*24) as f32,14.0,c(palette.muted));
        }
        for (hit,r) in confirm_controls(app) {
            if hit==ConfirmHit::Yes {
                let r=R {y:r.y+6,h:r.h-12,..r};
                surface(ctx,r,6.0,c(palette.danger_light),c(palette.danger));
                ctx.text_center(fonts,r,"确定退出",14.0,c(palette.text));
            } else {crate::settings_panel::button(ctx,fonts,r,"取消",false,14.0);}
        }
    });
}

pub fn diag_rect(app: &App) -> R {
    let (width, height) = crate::settings_panel::available_size(app);
    let w = 620.min((width - 16).max(1));
    let h = 620.min((height - 16).max(1));
    R {
        x: (width - w) / 2,
        y: ((height - h) / 2).max(8),
        w,
        h,
    }
}

pub fn diag_close_hit(app: &App, x: i32, y: i32) -> bool {
    crate::settings_panel::inspector_controls(app).into_iter().any(|(h,r)|h==SetHit::InspectorClose && r.contains(x,y))
}
pub fn draw_diag(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    crate::settings_panel::draw_inspector(app, fonts, ctx);
}

// ---------------- input 组授权弹窗 ----------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PromptHit {
    Authorize,
    Close,
}

pub fn input_group_prompt_rect(app:&App)->R {
    let w=540.min((app.screen_w-24).max(1)); let h=290.min((app.screen_h-24).max(1));
    R {x:(app.screen_w-w)/2,y:(app.screen_h-h)/2,w,h}
}
pub fn input_group_prompt_controls(app:&App)->Vec<(PromptHit,R)> {
    let p=input_group_prompt_rect(app); let w=(p.w-52)/2;
    if app.input_group_state==0 {
        vec![(PromptHit::Close,R {x:p.x+20,y:p.y+p.h-64,w,h:44}),
            (PromptHit::Authorize,R {x:p.x+32+w,y:p.y+p.h-64,w,h:44})]
    } else {vec![(PromptHit::Close,R {x:p.x+20,y:p.y+p.h-64,w:p.w-40,h:44})]}
}
pub fn input_group_prompt_hit(app:&App,x:i32,y:i32)->Option<PromptHit> {
    input_group_prompt_controls(app).into_iter().find_map(|(hit,r)|r.contains(x,y).then_some(hit))
}
pub fn draw_input_group_prompt(app:&App,fonts:&Fonts,ctx:&mut Ctx) {
    let p=input_group_prompt_rect(app); let palette=ctx.palette;
    panel(ctx,p,12.0,1.0);
    ctx.text(fonts,"触屏输入权限",(p.x+24) as f32,(p.y+42) as f32,19.0,c(palette.text));
    let body=match app.input_group_state {
        0 if p.h<250 => "触屏需要访问输入设备。\n授权后需重新登录。",
        0 => "触屏批注需要读取输入设备。\n授权会允许当前用户读取输入设备事件。\n添加后请注销并重新登录。",
        1 => "请在系统授权窗口中完成授权。",
        2 => "权限已添加。\n注销并重新登录后生效。",
        _ => "授权未完成。\n可以稍后重新尝试。",
    };
    let lines=crate::settings_panel::wrapped(fonts,body,14.0,p.w-48);
    for (i,line) in lines.iter().enumerate() {
        if 78+i as i32*25>p.h-72 {break;}
        ctx.text(fonts,line,(p.x+24) as f32,(p.y+78+i as i32*25) as f32,14.0,c(palette.muted));
    }
    for (hit,r) in input_group_prompt_controls(app) {
        crate::settings_panel::button(ctx,fonts,r,match hit {PromptHit::Authorize=>"授权",PromptHit::Close if app.input_group_state==0=>"稍后",_=>"关闭"},hit==PromptHit::Authorize,15.0);
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
            "[Desktop Entry]\nType=Application\nName=Sidera\nComment=Sidera软件\nExec=sidera --autostart\nIcon={}\nTerminal=false\n",
            autostart_icon());
        let _ = std::fs::write(p, entry);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(unix)]
pub fn ensure_autostart_flag() {
    let path = autostart_path();
    let Ok(mut content) = std::fs::read_to_string(&path) else { return; };
    if content.contains("--autostart") { return; }
    if content.lines().any(|line| line.trim() == "Exec=sidera") {
        content = content.replace("Exec=sidera", "Exec=sidera --autostart");
        if let Err(error) = crate::persistence::atomic_write(&path, content.as_bytes()) {
            log::warn!("更新自启动参数失败：{error}");
        }
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
            let data = wide_autostart(&format!("\"{}\" --autostart", exe.display()));
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordLayerKey {
    w: i32,
    h: i32,
    scale: f64,
    bg: i32,
    palette: theme::Palette,
    origin_x: f32,
    origin_y: f32,
    coord_scale: f32,
    rotation: f32,
    /// Plain coordinate guides use `size` for their finite grid extent.
    extent: Option<f32>,
    show_labels: bool,
}

/// Full-screen background + grid cache. The conic curve itself is drawn live,
/// so changing shape/size during editing can reuse this layer.
pub struct CoordLayer {
    key: CoordLayerKey,
    pixmap: Pixmap,
}

fn coord_layer_key(app: &App, coord: crate::app::CoordGuide) -> CoordLayerKey {
    CoordLayerKey {
        w: app.screen_w,
        h: app.screen_h,
        scale: app.scale,
        bg: app.whiteboard_bg_index,
        palette: app.palette(),
        origin_x: coord.origin_x,
        origin_y: coord.origin_y,
        coord_scale: coord.scale,
        rotation: coord.rotation,
        extent: (coord.conic == crate::app::ConicKind::None).then_some(coord.size),
        show_labels: !coord.editing,
    }
}

fn build_coord_layer(app: &App, fonts: &Fonts, key: CoordLayerKey, coord: crate::app::CoordGuide) -> Option<Pixmap> {
    let sc = app.scale as f32;
    let pw = ((app.screen_w as f32) * sc).round().max(1.0) as u32;
    let ph = ((app.screen_h as f32) * sc).round().max(1.0) as u32;
    let mut pm = Pixmap::new(pw, ph)?;
    pm.fill(app.whiteboard_bg_color());
    let mut ctx = Ctx { palette: key.palette, pm: &mut pm, tr: Transform::from_scale(sc, sc) };
    if let Some(bounds) = coord_bounds(&coord, 0, 0, app.screen_w, app.screen_h) {
        draw_coord_grid(fonts, &mut ctx, &coord, bounds, key.show_labels);
    }
    Some(pm)
}

/// 区域渲染的设备像素原点与变换。
///
/// 三个后端都按 `round(x * scale)` 放置上屏的 pixmap。渲染时必须使用同一个
/// 设备像素原点，否则内容会带上 0~1 像素的亚像素偏移：局部重绘与整屏重绘
/// 对不齐，表现为点击按钮时整个界面轻微跳动（125%/150% 缩放下最明显）。
fn region_origin(app: &App, x: i32, y: i32, x2: i32, y2: i32) -> (Transform, u32, u32) {
    let sc = app.scale as f32;
    let px0 = (x as f32 * sc).round();
    let py0 = (y as f32 * sc).round();
    let px1 = (x2 as f32 * sc).round();
    let py1 = (y2 as f32 * sc).round();
    let pw = (px1 - px0).max(1.0) as u32;
    let ph = (py1 - py0).max(1.0) as u32;
    // world(逻辑) -> region(物理)：平移量直接写在设备像素空间，避免
    // pre_translate 再乘一次缩放带来的浮点误差。
    (Transform { sx: sc, sy: sc, kx: 0.0, ky: 0.0, tx: -px0, ty: -py0 }, pw, ph)
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
    let palette = app.palette();
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
    let (tr, pw, ph) = region_origin(app, x, y, x2, y2);
    let mut pm = Pixmap::new(pw, ph)?;
    if app.settings_ui.window_size().is_some() {
        if app.previewing_sidebars() {
            let mut ctx = Ctx { palette, pm: &mut pm, tr };
            draw_sidebar(app, fonts, &mut ctx, false, 0.0);
            draw_sidebar(app, fonts, &mut ctx, true, 0.0);
        }
        return Some(pm);
    }

    // 背景 + 网格缓存；圆锥曲线每帧实时绘制，便于编辑时只更新曲线。
    if app.whiteboard {
        let mut cached = false;
        if let Some(coord) = app.coord {
            let key = coord_layer_key(app, coord);
            let mut cache = app.coord_layer.borrow_mut();
            if cache.as_ref().is_none_or(|layer| layer.key != key) {
                *cache = build_coord_layer(app, fonts, key, coord)
                    .map(|pixmap| CoordLayer { key, pixmap });
            }
            if let Some(layer) = cache.as_ref() {
                let dx = -((x as f32) * sc).round() as i32;
                let dy = -((y as f32) * sc).round() as i32;
                pm.draw_pixmap(dx, dy, layer.pixmap.as_ref(),
                    &tiny_skia::PixmapPaint::default(), Transform::identity(), None);
                cached = true;
            }
        }
        if !cached { pm.fill(app.whiteboard_bg_color()); }
        if let Some(coord) = &app.coord {
            if let Some(bounds) = coord_bounds(coord, x, y, x2, y2) {
                let mut ctx = Ctx { palette, pm: &mut pm, tr };
                draw_coord_curve(&mut ctx, coord, bounds);
            }
        }
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
    if app.whiteboard {
        if let Some(sel) = &app.floating {
            let dx = sel.x - ((x as f32) * sc).round() as i32;
            let dy = sel.y - ((y as f32) * sc).round() as i32;
            pm.draw_pixmap(dx, dy, sel.pixmap.as_ref(),
                &tiny_skia::PixmapPaint { blend_mode: BlendMode::SourceOver, ..Default::default() },
                Transform::identity(), None);
        }
        let mut ctx = Ctx { palette, pm: &mut pm, tr };
        draw_coord_foci(app, fonts, &mut ctx);
        draw_selection_outline(app, &mut ctx);
        draw_coord_handles(app, fonts, &mut ctx);
    }

    if app.launch.active() {
        let mut ctx = Ctx { palette: app.palette(), pm: &mut pm, tr };
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

        let sidebar_progress = if app.previewing_sidebars() { 0.0 } else { app.sidebar_retract() };
        draw_sidebar(app, fonts, &mut ctx, false, sidebar_progress);
        draw_sidebar(app, fonts, &mut ctx, true, sidebar_progress);

        if app.pen_popup_visible {
            draw_pen_popup(app, fonts, &mut ctx);
        }
        if app.eraser_popup_visible {
            draw_eraser_popup(app, fonts, &mut ctx);
        }
        if app.page_picker.is_some() { draw_page_picker(app, fonts, &mut ctx); }
        draw_preferences(app, fonts, &mut ctx, icon);
        if app.show_input_group_prompt {
            draw_input_group_prompt(app, fonts, &mut ctx);
        }
        draw_press_feedback(app, &mut ctx);
    }

    Some(pm)
}

/// Draw into a caller-owned buffer so the settings windows can be repainted
/// without allocating a full-surface Pixmap for every frame.
pub fn draw_settings_window(
    app: &App, fonts: &Fonts, icon: Option<&Pixmap>, pixmap: &mut Pixmap, scale: f32,
) {
    draw_settings_surface(app, fonts, icon, pixmap, scale, c(app.palette().paper));
}

pub(crate) fn draw_settings_surface(
    app: &App, fonts: &Fonts, icon: Option<&Pixmap>, pixmap: &mut Pixmap, scale: f32, background: Color,
) {
    let palette = app.palette();
    pixmap.fill(background);
    let mut ctx = Ctx { palette, pm: pixmap, tr: Transform::from_scale(scale, scale) };
    draw_preferences(app, fonts, &mut ctx, icon);
    draw_press_feedback(app, &mut ctx);
}

fn draw_preferences(app: &App, fonts: &Fonts, ctx: &mut Ctx, icon: Option<&Pixmap>) {
    if !app.settings_open && !app.process_block_open && !app.show_diag && !app.confirm_quit
        && app.settings_ui.dialog.is_none() && app.settings_ui.picker.is_none() {
        return;
    }
    let fonts = &fonts.for_settings();
    if app.settings_open {
        draw_settings(app, fonts, ctx, icon);
    }
    if app.process_block_open {
        draw_process_block(app, fonts, ctx);
    }
    if app.show_diag {
        draw_diag(app, fonts, ctx);
    }
    if app.confirm_quit {
        draw_confirm(app, fonts, ctx);
    }
    crate::settings_panel::draw_overlay(app, fonts, ctx);
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
    draw(&mut Ctx { palette: app.palette(), pm: &mut tile,
        tr: Transform::from_scale(scale, scale).pre_translate(-r.x as f32, -r.y as f32) });
    ctx.pm.draw_pixmap(0, 0, tile.as_ref(), &tiny_skia::PixmapPaint { opacity, ..Default::default() },
        ctx.tr.pre_translate(r.x as f32, r.y as f32).pre_scale(1.0 / scale, 1.0 / scale), None);
}

fn draw_launch_sidebar(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool) {
    if app.avoid_active { return; }
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
    if app.avoid_active && !app.settings_open { return; }
    let palette = ctx.palette;
    let drag = app.sidebar_drag.filter(|drag| drag.right == right);
    let held = drag.is_some();
    let stroke_width = 2.0;
    if progress < 1.0 {
        let sb = expanded_sidebar_rect(app, right);
        let dx = (sb.w + 4) as f32 * progress * if right { 1.0 } else { -1.0 };
        let mut moving = Ctx { palette: app.palette(), pm: &mut *ctx.pm, tr: ctx.tr.pre_translate(dx, 0.0) };
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
        let mut moving = Ctx { palette: app.palette(), pm: &mut *ctx.pm, tr: ctx.tr.pre_translate(dx, 0.0) };
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
        let mut local = Ctx { palette: app.palette(), pm: &mut tile,
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

fn draw_button_label(fonts: &Fonts, ctx: &mut Ctx, r: R, label: &str, ink: Color) {
    // Fit visible outlines, including overhangs and descenders, inside the padding.
    let Some((left, top, right, bottom)) = fonts.text_bounds(label, 1.0) else { return };
    let width = (right - left).max(0.01);
    let height = (bottom - top).max(0.01);
    let padding = (r.w.min(r.h) as f32 * 0.10).max(2.0);
    let size = (r.w as f32 * 0.49).max(11.0)
        .min((r.w as f32 - 2.0 * padding).max(1.0) / width)
        .min((r.h as f32 - 2.0 * padding).max(1.0) / height);
    let x = r.x as f32 + (r.w as f32 - width * size) / 2.0 - left * size;
    let baseline = r.y as f32 + (r.h as f32 - height * size) / 2.0 - top * size;
    ctx.text(fonts, label, x, baseline, size, ink);
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
    if k == Btn::Whiteboard && app.whiteboard {
        fill = c(palette.danger_light);
        edge = c(palette.danger);
        ink = c(palette.danger);
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
        Btn::Cursor | Btn::Pen | Btn::Eraser | Btn::Prev | Btn::Next => {
            let glyph = match k {
                Btn::Cursor => Glyph::Cursor, Btn::Pen => Glyph::Pen, Btn::Eraser => Glyph::Eraser,
                Btn::Prev => Glyph::Up, Btn::Next => Glyph::Down, _ => Glyph::More,
            };
            // 21 / 24 / 27 logical pixels at default scale: visible size cues,
            // with a fixed centre and fixed hit target at every toolbar scale.
            let factor = if k == Btn::Eraser {
                let last = (crate::app::ERASER_SIZES.len() - 1).max(1) as f32;
                0.86 + 0.26 * app.cur_eraser.min(crate::app::ERASER_SIZES.len() - 1) as f32 / last
            } else { 1.0 };
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
        Btn::Clear => draw_button_label(fonts, ctx, r, "清除", c(palette.danger)),
        Btn::Restore => draw_button_label(fonts, ctx, r, "恢复", ink),
        Btn::Screenshot => draw_button_label(fonts, ctx, r, "截图", ink),
        Btn::Undo => ctx.text_center(fonts, r, "撤回", (btn * 0.49).max(11.0), ink),
        Btn::Whiteboard => {
            if app.whiteboard {
                let top = R { h: r.h / 2, ..r };
                let bottom = R { y: r.y + top.h, h: r.h - top.h, ..r };
                draw_button_label(fonts, ctx, top, "关闭", ink);
                draw_button_label(fonts, ctx, bottom, "白板", ink);
            } else {
                draw_button_label(fonts, ctx, r, "白板", ink);
            }
        }
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
    let mut local = Ctx { palette: app.palette(), pm: &mut tile,
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
    ctx.text(fonts, "跳转页面", (p.x + 18) as f32, (p.y + 35) as f32, 18.0, c(palette.text));
    crate::value_input::draw_value(app,fonts,ctx,crate::value_input::Target::Page,
        page_picker_control_rect(app,PagePickerHit::Input),16.0,""," 页");
    let hint = crate::value_input::message(app,crate::value_input::Target::Page)
        .unwrap_or_else(||format!("共 {} 页 · 拨动选页或点击页码输入",picker.total));
    let hint_size = 12.0 * ((p.w - 36) as f32 / fonts.text_width(&hint, 12.0).max(1.0)).min(1.0);
    let error=app.settings_ui.editor.as_ref().is_some_and(|e|e.target==crate::value_input::Target::Page&&!e.error.is_empty());
    ctx.text(fonts, &hint, (p.x + 18) as f32, (p.y + 56) as f32, hint_size, c(if error{palette.danger}else{palette.muted}));
    if page_picker_has_thumbnail(app) {
        let thumb = page_picker_thumb_rect(app);
        surface(ctx, thumb, 8.0, c(palette.surface), c(palette.border));
        let (epoch, page) = match &picker.source {
            crate::page_picker::Source::Wps(epoch) => (*epoch, picker.selected()),
            _ => (0, picker.selected()),
        };
        if let Some(pm) = app.thumbs.get(&(epoch, page)) {
            let scale = (thumb.w as f32 / pm.width() as f32)
                .min(thumb.h as f32 / pm.height() as f32);
            let dx = thumb.x as f32 + (thumb.w as f32 - pm.width() as f32 * scale) / 2.0;
            let dy = thumb.y as f32 + (thumb.h as f32 - pm.height() as f32 * scale) / 2.0;
            ctx.pm.draw_pixmap(0, 0, pm.as_ref(),
                &tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() },
                ctx.tr.pre_translate(dx, dy).pre_scale(scale, scale), None);
        } else {
            ctx.text_center(fonts, thumb, "缩略图加载中…", 14.0, c(palette.muted));
        }
    }
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
        let ink = palette.text;
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
    let names = ["红色", "蓝色", "绿色", "黄色", "白色", "青色", "紫色", "黄绿", "橙色", "黑色"];
    ctx.text(fonts, "当前颜色", (p.x + 124) as f32, (p.y + 156) as f32, 15.0, c(palette.muted));
    ctx.text(fonts, names[app.cur_color.min(9)], (p.x + 124) as f32, (p.y + 177) as f32, 20.0, c(palette.text));
    let sy = p.y + 214;
    ctx.text(fonts, "画笔粗细", (p.x + 16) as f32, (sy - 14) as f32, 19.0, c(palette.muted));
    let tiers = crate::app::PEN_SIZES.len();
    let bw = tier_button_width(p.w, tiers);
    for i in 0..tiers {
        let r = R { x: p.x + 16 + i as i32 * (bw + 10), y: sy, w: bw, h: 48 };
        choice(ctx, r, i == app.cur_pen);
        let label = crate::app::PEN_SIZES[i].to_string();
        ctx.text_center(fonts, R { h: 28, ..r }, &label, 18.0, c(palette.text));
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
    let size = 20.0 + crate::app::ERASER_SIZES[app.cur_eraser.min(crate::app::ERASER_SIZES.len() - 1)] as f32 * 0.15;
    icons::draw(ctx, Glyph::Eraser, p.x as f32 + p.w as f32 - 30.0 - size / 2.0,
        p.y as f32 + 24.0 - size / 2.0, size, c(palette.text), c(palette.eraser));
    let tiers = crate::app::ERASER_SIZES.len();
    let bw = tier_button_width(p.w, tiers);
    for i in 0..tiers {
        let r = R { x: p.x + 16 + i as i32 * (bw + 10), y: p.y + 46, w: bw, h: 48 };
        choice(ctx, r, i == app.cur_eraser);
        let size = 16.0 + crate::app::ERASER_SIZES[i] as f32 * 0.16;
        icons::draw(ctx, Glyph::Eraser, r.x as f32 + 19.0 - size / 2.0, r.y as f32 + 24.0 - size / 2.0,
            size, c(palette.text), c(palette.eraser));
        let label = crate::app::ERASER_SIZES[i].to_string();
        ctx.text_center(fonts, R { x: r.x + 33, w: r.w - 37, ..r }, &label, 18.0, c(palette.text));
    }
    let r = R { x: p.x + 16, y: p.y + 110, w: p.w - 32, h: 48 };
    surface(ctx, r, 10.0, c(palette.danger_light), alpha(palette.danger, 75));
    ctx.text_center(fonts, r, "清空当前页", 20.0, c(palette.danger));
}


fn wb_label(ctx: &mut Ctx, fonts: &Fonts, r: R, text: &str, px: f32, color: Color) {
    let fit = px * ((r.w - 6).max(1) as f32 / fonts.text_width(text, px).max(1.0)).min(1.0);
    ctx.text_center(fonts, r, text, fit, color);
}

fn coord_bounds(coord: &crate::app::CoordGuide,
    x: i32, y: i32, x2: i32, y2: i32) -> Option<(f32, f32, f32, f32)> {
    let (mut u0, mut u1, mut v0, mut v1) = crate::conic::visible_bounds(coord,
        x as f32, y as f32, x2 as f32, y2 as f32);
    if coord.conic == crate::app::ConicKind::None {
        let extent = 20.0 * crate::conic::size_scale(coord.size);
        u0 = u0.max(-extent); u1 = u1.min(extent);
        v0 = v0.max(-extent); v1 = v1.min(extent);
        if u0 >= u1 || v0 >= v1 { return None; }
    }
    Some((u0, u1, v0, v1))
}

fn draw_coord_grid(fonts: &Fonts, ctx: &mut Ctx, coord: &crate::app::CoordGuide,
    bounds: (f32, f32, f32, f32), show_labels: bool) {
    let (u0, u1, v0, v1) = bounds;
    let palette = ctx.palette;
    let mut step = if coord.scale >= 28.0 { 1.0 }
        else if coord.scale >= 14.0 { 2.0 }
        else if coord.scale >= 7.0 { 5.0 } else { 10.0 };
    let max_lines = if show_labels { 110.0 } else { 80.0 };
    while (u1 - u0) / step > max_lines || (v1 - v0) / step > max_lines { step *= 2.0; }
    let grid = alpha(palette.muted, 70);
    let axis = alpha(palette.text, 190);
    let mut u = (u0 / step).floor() * step;
    while u <= u1 {
        let p0 = crate::conic::to_screen(coord, u, v0);
        let p1 = crate::conic::to_screen(coord, u, v1);
        let (ink, width) = if u.abs() < 1e-4 { (axis, 1.8) } else { (grid, 1.0) };
        ctx.stroke_line(p0, p1, ink, width);
        u += step;
    }
    let mut v = (v0 / step).floor() * step;
    while v <= v1 {
        let p0 = crate::conic::to_screen(coord, u0, v);
        let p1 = crate::conic::to_screen(coord, u1, v);
        let (ink, width) = if v.abs() < 1e-4 { (axis, 1.8) } else { (grid, 1.0) };
        ctx.stroke_line(p0, p1, ink, width);
        v += step;
    }
    if !show_labels { return; }
    let label_step = {
        let count = ((u1 - u0) / step).max(1.0);
        step * (count / 10.0).ceil().max(1.0)
    };
    let mut u = (u0 / label_step).ceil() * label_step;
    while u <= u1 {
        if u.abs() > 1e-4 {
            let p = crate::conic::to_screen(coord, u, 0.0);
            ctx.text(fonts, &format!("{u:.0}"), p.0 + 3.0, p.1 - 4.0, 11.0, c(palette.muted));
        }
        u += label_step;
    }
    let label_step = {
        let count = ((v1 - v0) / step).max(1.0);
        step * (count / 10.0).ceil().max(1.0)
    };
    let mut v = (v0 / label_step).ceil() * label_step;
    while v <= v1 {
        if v.abs() > 1e-4 {
            let p = crate::conic::to_screen(coord, 0.0, v);
            ctx.text(fonts, &format!("{v:.0}"), p.0 + 4.0, p.1 + 4.0, 11.0, c(palette.muted));
        }
        v += label_step;
    }
}

fn draw_coord_curve(ctx: &mut Ctx, coord: &crate::app::CoordGuide,
    bounds: (f32, f32, f32, f32)) {
    let (u0, u1, v0, v1) = bounds;
    if coord.conic == crate::app::ConicKind::None { return; }
    let palette = ctx.palette;
    for branch in crate::conic::branches(coord, u0, u1, v0, v1) {
        let points: Vec<(f32, f32)> = branch.iter()
            .map(|&(u, v)| crate::conic::to_screen(coord, u, v)).collect();
        ctx.stroke_polyline(&points, c(palette.accent), 2.0,
            coord.conic == crate::app::ConicKind::Ellipse);
    }
}

fn draw_coord_foci(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let Some(coord) = app.coord.as_ref() else { return };
    if coord.conic == crate::app::ConicKind::None { return; }
    let palette = ctx.palette;
    let foci = crate::conic::foci(coord);
    let count = foci.len();
    for (i, &(fu, fv)) in foci.iter().enumerate() {
        let p = crate::conic::to_screen(coord, fu, fv);
        ctx.fill_circle(p.0, p.1, 4.5, c(palette.accent));
        ctx.fill_circle(p.0, p.1, 1.7, c(palette.on_accent));
        let label = if count == 1 { "F".to_string() }
            else if i == 0 { "F1".to_string() } else { "F2".to_string() };
        ctx.text(fonts, &label, p.0 + 7.0, p.1 - 7.0, 12.0, c(palette.accent));
    }
}

fn draw_selection_outline(app: &App, ctx: &mut Ctx) {
    if let Some(sel) = &app.floating {
        let sc = app.scale.max(0.001) as f32;
        let dx = (sel.x - sel.origin_x) as f32 / sc;
        let dy = (sel.y - sel.origin_y) as f32 / sc;
        let points: Vec<(f32, f32)> = sel.outline.iter()
            .map(|&(x, y)| (x + dx, y + dy)).collect();
        ctx.stroke_hazard(&points, true);
    } else if !app.lasso.is_empty() {
        ctx.stroke_hazard(&app.lasso, app.lasso.len() > 2);
    }
}

fn draw_coord_handles(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let Some(coord) = app.coord.as_ref() else { return };
    if !coord.editing || app.conic_menu.is_some() { return; }
    let palette = ctx.palette;
    if let Some((hx, hy)) = coord_rotate_handle(app) {
        ctx.stroke_line((coord.origin_x, coord.origin_y), (hx, hy), alpha(palette.accent, 140), 1.5);
        ctx.fill_circle(hx, hy, 10.0, c(palette.accent));
        ctx.fill_circle(hx, hy, 6.0, c(palette.paper));
        ctx.text(fonts, "旋转", hx + 13.0, hy - 10.0, 12.0, c(palette.muted));
    }
    ctx.fill_circle(coord.origin_x, coord.origin_y, 8.0, c(palette.accent));
    ctx.fill_circle(coord.origin_x, coord.origin_y, 3.5, c(palette.paper));
}

fn draw_slider(ctx: &mut Ctx, r: R, value: f32, accent: Color, track: Color) {
    let value = value.clamp(0.0, 1.0);
    ctx.fill_round(R { y: r.y + r.h / 2 - 2, h: 4, ..r }, 2.0, track);
    let thumb_x = r.x + ((r.w - 16) as f32 * value) as i32;
    ctx.fill_round(R { x: thumb_x, y: r.y + 1, w: 16, h: r.h - 2 }, 8.0, accent);
}

fn draw_whiteboard_controls(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let palette = ctx.palette;
    let bar = whiteboard_bar_rect(app);
    panel(ctx, bar, 12.0, 1.0);
    let conic = app.coord.map(|c| c.conic);
    let labels = ["空坐标系", "椭圆", "双曲线", "抛物线", "选择"];
    for (i, label) in labels.iter().enumerate() {
        let r = wb_button_rect(app, i);
        let selected = match i {
            0 => conic == Some(crate::app::ConicKind::None),
            1..=3 => conic == Some(WB_BUTTONS[i].0) || app.conic_menu == Some(WB_BUTTONS[i].0),
            _ => app.select_mode || app.floating.is_some(),
        };
        let fill = if selected { c(palette.accent_light) } else { c(palette.surface) };
        let border = if selected { c(palette.accent) } else { c(palette.border) };
        surface(ctx, r, 8.0, fill, border);
        wb_label(ctx, fonts, r, label, 14.0, c(palette.text));
    }
    if let Some(kind) = app.conic_menu {
        let menu = direction_menu_rect(app, kind);
        panel(ctx, menu, 12.0, 1.0);
        for (i, (direction, label)) in kind.directions().iter().enumerate() {
            let r = direction_option_rect(app, kind, i);
            let selected = app.coord.is_some_and(|c| c.conic == kind && c.direction == *direction);
            surface(ctx, r, 7.0,
                c(if selected { palette.accent_light } else { palette.paper }),
                c(if selected { palette.accent } else { palette.border }));
            wb_label(ctx, fonts, r, label, 15.0, c(palette.text));
        }
    }
    if app.floating.is_some() {
        let bar = selection_bar_rect(app);
        panel(ctx, bar, 12.0, 1.0);
        let cancel = selection_cancel_rect(app);
        surface(ctx, cancel, 7.0, c(palette.surface), c(palette.border));
        wb_label(ctx, fonts, cancel, "取消", 14.0, c(palette.text));
        let done = selection_done_rect(app);
        surface(ctx, done, 7.0, c(palette.accent), c(palette.accent));
        wb_label(ctx, fonts, done, "完成", 14.0, c(palette.on_accent));
    }
    if let Some(coord) = app.coord.filter(|c| c.editing && app.conic_menu.is_none()) {
        let edit = whiteboard_edit_bar_rect(app);
        panel(ctx, edit, 12.0, 1.0);
        let zoom_slot = wb_slider_slot(app, 0);
        ctx.text(fonts, "缩放", zoom_slot.x as f32, (zoom_slot.y + 18) as f32, 13.0, c(palette.muted));
        draw_slider(ctx, wb_zoom_slider_rect(app), (coord.scale - 12.0) / 108.0,
            c(palette.accent), c(palette.border));
        if coord.conic != crate::app::ConicKind::None {
            let shape_slot = wb_slider_slot(app, 1);
            ctx.text(fonts, "形状", shape_slot.x as f32, (shape_slot.y + 18) as f32, 13.0, c(palette.muted));
            draw_slider(ctx, wb_shape_slider_rect(app), coord.shape,
                c(palette.accent), c(palette.border));
        }
        let size_index = if coord.conic != crate::app::ConicKind::None { 2 } else { 1 };
        let size_slot = wb_slider_slot(app, size_index);
        ctx.text(fonts, "大小", size_slot.x as f32, (size_slot.y + 18) as f32, 13.0, c(palette.muted));
        draw_slider(ctx, wb_size_slider_rect(app), coord.size,
            c(palette.accent), c(palette.border));
        let delete = wb_delete_rect(app);
        surface(ctx, delete, 7.0, c(palette.danger_light), c(palette.danger));
        wb_label(ctx, fonts, delete, "删除", 14.0, c(palette.danger));
        let done = wb_done_rect(app);
        surface(ctx, done, 7.0, c(palette.accent), c(palette.accent));
        wb_label(ctx, fonts, done, "完成", 14.0, c(palette.on_accent));
        ctx.text(fonts, "拖动中心移动 · 拖动圆点旋转", (edit.x + 12) as f32,
            (edit.y - 6) as f32, 12.0, c(palette.muted));
    }
}

fn draw_process_block(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    crate::settings_panel::draw_process(app, fonts, ctx);
}

fn draw_settings(app: &App, fonts: &Fonts, ctx: &mut Ctx, icon: Option<&Pixmap>) {
    crate::settings_panel::draw(app, fonts, ctx, icon);
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_pixel_axis_lines_snap_to_half_pixels() {
        let identity = Transform::from_scale(1.0, 1.0);
        let ((x1, y1), (x2, y2)) = snap_axis_line(&identity, (0.0, 10.0), (20.0, 10.0), 1.0);
        assert_eq!((x1, y1), (0.0, 10.5));
        assert_eq!((x2, y2), (20.0, 10.5));
        let ((x1, y1), (x2, y2)) = snap_axis_line(&identity, (10.0, 0.0), (10.0, 20.0), 1.0);
        assert_eq!((x1, y1), (10.5, 0.0));
        assert_eq!((x2, y2), (10.5, 20.0));
        // Two device pixels already land on the grid; do not shift them.
        let double = Transform::from_scale(2.0, 2.0);
        let (_, (_, y2)) = snap_axis_line(&double, (0.0, 10.0), (20.0, 10.0), 1.0);
        assert_eq!(y2, 10.0);
    }

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
                for tab in crate::settings_state::Tab::ALL {
                a.settings_ui.tab = tab;
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
                }
                for expected in [SetHit::Autostart,
                    SetHit::WpsRemove, SetHit::WpsRegister, SetHit::ProcessBlock, SetHit::ThemeAccent,
                    SetHit::Logs, SetHit::SysInfo, SetHit::License, SetHit::Repository, SetHit::Quit, SetHit::Done] {
                    assert!(reached.contains(&expected), "unreachable {expected:?} at {w}x{h}");
                }
                assert_eq!(sliders.len(), 7);
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
        assert!(settings_controls(&a).iter().any(|(hit, _)| *hit == SetHit::ResetSlider(SliderId::Position)));
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
                a.settings_ui.theme_from = None;
                for page in 0..settings_layout(&a).pages {
                    a.settings_page = page;
                    let r = settings_rect(&a);
                    let pm = render_region(&a, &fonts, None, r.x, r.y, r.w, r.h).unwrap();
                    let px0 = (r.x as f64 * scale).round() as u32;
                    let py0 = (r.y as f64 * scale).round() as u32;
                    let px1 = ((r.x + r.w) as f64 * scale).round() as u32;
                    let py1 = ((r.y + r.h) as f64 * scale).round() as u32;
                    assert_eq!(pm.width(), px1 - px0);
                    assert_eq!(pm.height(), py1 - py0);
                    let done = settings_controls(&a).into_iter().find(|(h, _)| *h == SetHit::Done).unwrap().1;
                    let center_x = ((done.x as f64 + done.w as f64 / 2.0) * scale).round() as u32 - px0;
                    let center_y = ((done.y as f64 + done.h as f64 / 2.0) * scale).round() as u32 - py0;
                    assert_eq!(pm.pixel(center_x, center_y).unwrap(),
                        c(a.theme.palette().accent).to_color_u8().premultiply());
                    assert!(a.canvas.data().iter().all(|&b| b == 0));
                }
            }
        }
    }

    #[test]
    fn region_repaint_origin_snaps_to_device_pixels() {
        // 点击控件只重绘一小块区域。三个后端都按 round(x*scale) 放置 pixmap，
        // 渲染原点若不对齐到同一个设备像素，局部重绘就会带上亚像素偏移，
        // 表现为界面轻微跳动。
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            let mut a = App::new(Pixmap::new(640, 360).unwrap(), 640, 360);
            a.set_scale(scale);
            for (x, y) in [(0, 0), (1, 3), (7, 57), (101, 57), (233, 199)] {
                let (tr, pw, ph) = region_origin(&a, x, y, x + 120, y + 90);
                let px0 = (x as f64 * scale).round() as f32;
                let py0 = (y as f64 * scale).round() as f32;
                let px1 = ((x + 120) as f64 * scale).round() as f32;
                let py1 = ((y + 90) as f64 * scale).round() as f32;
                assert_eq!((tr.sx, tr.sy), (scale as f32, scale as f32));
                assert_eq!(tr.tx, -px0, "scale={scale} x={x}");
                assert_eq!(tr.ty, -py0, "scale={scale} y={y}");
                assert_eq!(pw as f32, px1 - px0, "scale={scale} x={x}");
                assert_eq!(ph as f32, py1 - py0, "scale={scale} y={y}");
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
    fn themes_cover_every_panel_and_startup_cache_at_multiple_scales() {
        let fonts = Fonts::load();
        for scale in [1.0, 1.5, 2.0] {
            let mut a = App::new(Pixmap::new(1280, 900).unwrap(), 1280, 900);
            a.set_scale(scale);
            for preference in [theme::Preference::Light, theme::Preference::Dark] {
                a.apply_theme(theme::Theme { preference, system_dark: false, ..Default::default() });
                a.settings_ui.theme_from = None;
                for panel in 0..6 {
                    a.settings_open = panel == 0;
                    a.pen_popup_visible = panel == 1;
                    a.eraser_popup_visible = panel == 2;
                    a.confirm_quit = panel == 3;
                    a.show_diag = panel == 4;
                    a.page_picker = (panel == 5).then(|| crate::page_picker::PagePicker::new(
                        6, 24, crate::page_picker::Source::Wps(1)));
                    let r = match panel {
                        0 => settings_rect(&a), 1 => pen_popup_rect(&a), 2 => eraser_popup_rect(&a),
                        3 => confirm_rect(&a), 4 => diag_rect(&a),
                        _ => page_picker_rect(&a),
                    };
                    let pixels = render_region(&a, &fonts, None, r.x, r.y, r.w, r.h).unwrap();
                    let palette = a.theme.palette();
                    let edge_fill = if panel == 0 {
                        theme::mix(palette.paper, palette.surface, 0.65)
                    } else {
                        palette.paper
                    };
                    assert_eq!(pixels.pixel((4.0 * scale) as u32, pixels.height() / 2).unwrap(),
                        c(edge_fill).to_color_u8().premultiply(),
                        "panel {panel} / {preference:?} / {scale}");
                    assert!(a.canvas.data().iter().all(|&b| b == 0));
                }
            }
            // Render the real cached startup, then change theme while it is visible.
            a.launch = crate::launch::Launch::begin("isolated".into(), true, true, std::time::Instant::now());
            for preference in [theme::Preference::Light, theme::Preference::Dark, theme::Preference::Light] {
                a.apply_theme(theme::Theme { preference, system_dark: false, ..Default::default() });
                a.settings_ui.theme_from = None;
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
mod whiteboard_guide_tests {
    use super::*;
    use tiny_skia::Pixmap;

    #[test]
    fn coordinate_conic_renders_with_edit_controls() {
        let fonts = Fonts::load();
        let mut app = App::new(Pixmap::new(320, 240).unwrap(), 320, 240);
        app.whiteboard = true;
        app.coord = Some(crate::app::CoordGuide::new(320, 240,
            crate::app::ConicKind::Ellipse, crate::app::ConicDirection::Horizontal));
        let pm = render_region(&app, &fonts, None, 0, 0, 320, 240).unwrap();
        assert!(pm.data().iter().any(|value| *value != 0));
        assert!(control_rect(&app, Control::Whiteboard(WhiteboardControl::ZoomSlider)).is_some());
        assert!(control_rect(&app, Control::Whiteboard(WhiteboardControl::SizeSlider)).is_some());
        assert!(control_rect(&app, Control::Whiteboard(WhiteboardControl::ShapeSlider)).is_some());
        assert!(control_rect(&app, Control::Whiteboard(WhiteboardControl::Done)).is_some());
        assert!(control_rect(&app, Control::Whiteboard(WhiteboardControl::Delete)).is_some());
    }

    fn overlaps(a: R, b: R) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn edit_controls_fit_without_overlapping() {
        for (w, h) in [(640i32, 360i32), (1280, 720), (1920, 1080)] {
            let mut app = App::new(Pixmap::new(w as u32, h as u32).unwrap(), w, h);
            app.whiteboard = true;
            app.coord = Some(crate::app::CoordGuide::new(w, h,
                crate::app::ConicKind::Ellipse, crate::app::ConicDirection::Horizontal));
            let bar = whiteboard_edit_bar_rect(&app);
            let rects = [
                wb_zoom_slider_rect(&app), wb_shape_slider_rect(&app), wb_size_slider_rect(&app),
                wb_done_rect(&app), wb_delete_rect(&app),
            ];
            for r in rects {
                assert!(r.x >= bar.x && r.x + r.w <= bar.x + bar.w
                    && r.y >= bar.y && r.y + r.h <= bar.y + bar.h, "{w}x{h}: {r:?} outside {bar:?}");
            }
            for i in 0..rects.len() {
                for j in i + 1..rects.len() {
                    assert!(!overlaps(rects[i], rects[j]), "{w}x{h}: {i} overlaps {j}");
                }
            }
            for i in 0..5 {
                assert!(!overlaps(wb_button_rect(&app, i), wb_button_rect(&app, if i == 4 { 0 } else { i + 1 })));
            }
        }
    }

    #[test]
    fn x11_settings_keep_the_large_tabbed_window() {
        let fonts = Fonts::load();
        let mut app = App::new(Pixmap::new(1280, 900).unwrap(), 1280, 900);
        app.settings_open = true;
        let p = settings_rect(&app);
        assert_eq!((p.w, p.h), (860, 620));
        assert!(render_region(&app, &fonts, None, p.x, p.y, p.w, p.h).is_some());
        for (_, r) in settings_controls(&app) {
            assert!(r.x >= p.x && r.y >= p.y && r.x + r.w <= p.x + p.w && r.y + r.h <= p.y + p.h,
                "control {r:?} escapes settings {p:?}");
        }
    }

    #[test]
    fn static_guide_reuses_the_cached_layer() {
        let fonts = Fonts::load();
        let mut app = App::new(Pixmap::new(640, 360).unwrap(), 640, 360);
        app.whiteboard = true;
        app.coord = Some(crate::app::CoordGuide::new(640, 360,
            crate::app::ConicKind::Ellipse, crate::app::ConicDirection::Horizontal));
        app.coord.as_mut().unwrap().editing = false;
        let first = render_region(&app, &fonts, None, 0, 0, 640, 360).unwrap();
        assert!(app.coord_layer.borrow().is_some(), "static guide must be cached");
        let second = render_region(&app, &fonts, None, 0, 0, 640, 360).unwrap();
        assert_eq!(first.data(), second.data(), "cache must reproduce the same pixels");
        let before = app.coord_layer.borrow().as_ref().unwrap().key;
        app.coord.as_mut().unwrap().shape = 0.9;
        let changed = render_region(&app, &fonts, None, 0, 0, 640, 360).unwrap();
        assert_ne!(second.data(), changed.data(), "the live conic must repaint");
        let after = app.coord_layer.borrow().as_ref().unwrap().key;
        assert_eq!(before, after, "shape edits must reuse the grid cache");
    }

    #[test]
    fn plain_coordinate_hides_the_shape_control() {
        let mut app = App::new(Pixmap::new(640, 360).unwrap(), 640, 360);
        app.whiteboard = true;
        app.coord = Some(crate::app::CoordGuide::new(640, 360,
            crate::app::ConicKind::None, crate::app::ConicDirection::Horizontal));
        assert!(!control_enabled(&app, Control::Whiteboard(WhiteboardControl::ShapeSlider)));
        assert!(control_enabled(&app, Control::Whiteboard(WhiteboardControl::ZoomSlider)));
        assert!(control_enabled(&app, Control::Whiteboard(WhiteboardControl::SizeSlider)));
    }
}
