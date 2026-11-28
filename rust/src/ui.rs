// ============================================================
// Sidera - 手绘 UI（侧边栏 / 弹窗 / 设置面板）与命中测试
// 不再使用 QWidget：所有控件都是几何矩形 + tiny-skia 绘制
// ============================================================
use tiny_skia::{
    BlendMode, Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Shader, Stroke,
    StrokeDash, Transform,
};

use crate::app::App;
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

    // Rasterize curved edges inside the tile. Clipping exactly at a control's
    // bounds can change the antialiased corner pixels between partial frames.
    pub(crate) fn with_paint_margin(self) -> Self {
        Self { x: self.x - 2, y: self.y - 2, w: self.w + 4, h: self.h + 4 }
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

impl Btn {
    pub(crate) fn tool_mode(self) -> Option<i32> {
        match self {
            Self::Cursor => Some(0),
            Self::Pen => Some(1),
            Self::Eraser => Some(2),
            _ => None,
        }
    }
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
    FadeToolbarDuringShow,
    ToolbarIconsOnly,
    DocumentScroll,
    #[cfg(unix)]
    EnableScrollAccess,
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
    Value(crate::conic::Parameter),
    ParameterTab(crate::conic::Parameter),
    ResetView,
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
    PageCount(bool), DocumentScroll(bool), PagePicker(PagePickerHit), Whiteboard(WhiteboardControl),
}

impl Control {
    fn pressed(self, app: &App) -> bool {
        app.press.is_some_and(|press| press.control == self && press.active && !press.expired())
    }

    pub(crate) fn tool_segment(self) -> Option<(Btn, bool)> {
        match self {
            Self::Sidebar(button, right) if button.tool_mode().is_some() => Some((button, right)),
            _ => None,
        }
    }
}

pub fn control_enabled(app: &App, control: Control) -> bool {
    if matches!(control,Control::DocumentScroll(_)) {
        return app.wants_scroll_rail() && app.scroll_rail.mix >= 0.75;
    }
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
    if let Control::Whiteboard(hit) = control {
        return whiteboard::control_bounds(app, hit).is_some();
    }
    true
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
    if app.page_picker.is_some() && page_picker_rect(app).contains(x, y) {
        return page_picker_hit(app, x, y).map(Control::PagePicker);
    }
    if app.pen_popup_visible && pen_popup_rect(app).contains(x, y) {
        return pen_popup_hit(app, x, y).map(Control::Popup);
    }
    if app.eraser_popup_visible && eraser_popup_rect(app).contains(x, y) {
        return eraser_popup_hit(app, x, y).map(Control::Popup);
    }
    sidebar_control_at(app, x, y)
        .or_else(|| whiteboard_hit(app, x, y).map(Control::Whiteboard))
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
        Control::DocumentScroll(right) if app.wants_scroll_rail() && !app.collapsed => Some(scroll_track_rect(app, right)),
        Control::PagePicker(hit) if app.page_picker.is_some() => Some(page_picker_control_rect(app, hit)),
        Control::Sidebar(b, right) => sidebar_buttons(app, right).into_iter().find(|(k, _)| *k == b).map(|(_, r)| r),
        Control::Setting(hit) if app.settings_open => settings_controls(app).into_iter().find(|(k, _)| *k == hit).map(|(_, r)| r),
        Control::Slider(id) if app.settings_open => settings_sliders(app).into_iter().find(|s| s.id == id).map(|s| s.hit),
        Control::Whiteboard(hit) => whiteboard::control_bounds(app, hit),
        Control::Popup(hit @ (PopupHit::Color(_) | PopupHit::PenSize(_))) if app.pen_popup_visible => {
            pen_controls(pen_popup_rect(app)).find(|(key, _)| *key == hit).map(|(_, r)| r)
        }
        Control::Popup(hit @ (PopupHit::EraserSize(_) | PopupHit::ClearAll)) if app.eraser_popup_visible => {
            eraser_controls(eraser_popup_rect(app)).find(|(key, _)| *key == hit).map(|(_, r)| r)
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
    // Toolbar controls paint their pressed face underneath their own glyph.
    if matches!(press.control, Control::Sidebar(..) | Control::PageCount(_) | Control::Popup(_)
        | Control::PagePicker(_) | Control::Whiteboard(_)) { return; }
    if !control_enabled(app, press.control) { return; }
    let Some(r) = control_rect(app, press.control) else { return };
    if matches!(press.control, Control::Slider(_)
        | Control::Picker(crate::settings_state::PickerHit::Field | crate::settings_state::PickerHit::Hue | crate::settings_state::PickerHit::Edit(_))
        | Control::Editor(_) | Control::Setting(SetHit::EditSlider(_) | SetHit::InspectorPage(_))
        | Control::Process(ProcessHit::ScrollUp | ProcessHit::ScrollDown)) { return; }
    let inset = R { x: r.x + 2, y: r.y + 2, w: r.w - 4, h: r.h - 4 };
    let filled_action = matches!(press.control, Control::Setting(SetHit::Done) | Control::DiagClose)
        || matches!(press.control, Control::Setting(SetHit::Theme(p)) if p == app.theme.preference);
    let feedback = if matches!(press.control, Control::Setting(SetHit::Quit)) {
        palette.danger
    } else if filled_action {
        palette.on_accent
    } else {
        palette.accent
    };
    ctx.fill_round(inset, 9.0, alpha(feedback, 32));
    ctx.stroke_round(inset, 9.0, c(feedback), 2.0);
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

pub(crate) const POPUP_FADE_MS: i64 = 160;

fn anim_t(app: &App) -> f32 {
    let el = (crate::app::now_ms() - app.open_anim) as f32;
    (el / POPUP_FADE_MS as f32).clamp(0.0, 1.0)
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
        if let Some(p) = rounded_path(r.x as f32, r.y as f32, r.w as f32, r.h as f32, radius) {
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
        if let Some(p) = rounded_path(x, y, w, h, radius) {
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
    /// Alternate contrasting strokes so the selection remains visible over handwriting.
    pub fn stroke_selection(&mut self, points: &[(f32, f32)], closed: bool) {
        if points.len() < 2 { return; }
        let mut pb = PathBuilder::new();
        pb.move_to(points[0].0, points[0].1);
        for &(x, y) in &points[1..] { pb.line_to(x, y); }
        if closed { pb.close(); }
        let Some(p) = pb.finish() else { return };
        for (color, offset) in [(c(self.palette.accent), 0.0f32), (c(self.palette.paper), 6.0f32)] {
            let st = Stroke {
                width: 2.0,
                dash: StrokeDash::new(vec![6.0, 6.0], offset),
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
    ctx.stroke_round(R { x: r.x + 1, y: r.y + 1, w: r.w - 2, h: r.h - 2 },
        (radius - 1.0).max(1.0), border, 1.0);
}

// The touch target can be larger than its visible face. Use the face's own
// fill and stroke paths in both states, including at fractional display scales.
fn button_surface(app: &App, ctx: &mut Ctx, control: Control, r: R, radius: f32, fill: Color, border: Color) -> Color {
    let (fill, border) = if control.pressed(app) && control_enabled(app, control) {
        let p = ctx.palette;
        let ink = match control {
            Control::Sidebar(Btn::Whiteboard, _) if app.whiteboard => p.danger,
            Control::PagePicker(PagePickerHit::Jump) | Control::Whiteboard(WhiteboardControl::Done | WhiteboardControl::SelDone) => p.on_accent,
            Control::Whiteboard(WhiteboardControl::Delete) => p.danger,
            _ => p.accent,
        };
        let pixel = fill.to_color_u8();
        let rgb = u32::from_be_bytes([0, pixel.red(), pixel.green(), pixel.blue()]);
        (c(theme::mix(rgb, ink, 0.14)), c(ink))
    } else {
        (fill, border)
    };
    surface(ctx, r, radius, fill, border);
    fill
}

pub(crate) fn panel(ctx: &mut Ctx, r: R, radius: f32, opacity: f32) {
    let palette = ctx.palette;
    let a = (255.0 * opacity.clamp(0.0, 1.0)) as u8;
    surface(ctx, r, radius, alpha(palette.paper, a), alpha(palette.border, a));
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

pub(crate) fn sidebar_tab_face(app: &App, right: bool) -> R {
    let target = sidebar_tab_rect(app, right);
    let w = (28.0 * app.sb_scale).round().clamp(24.0, 34.0) as i32;
    R { x: if right { target.x + target.w - w - 2 } else { target.x + 2 },
        y: target.y + 2, w, h: target.h - 4 }
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
    let y = app.sidebar_y(full).max(0);
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

fn sidebar_control_at(app: &App, x: i32, y: i32) -> Option<Control> {
    if app.avoid_active || app.sidebar_anim_start.is_some() || y < 0 || y >= app.screen_h { return None; }
    for right in [false, true] {
        let sb = sidebar_rect(app, right);
        if !sb.contains(x, y) { continue; }
        if app.collapsed { return Some(Control::Sidebar(Btn::Collapse, right)); }
        // Keep the side gutters and both end grips available for dragging.
        if x < sb.x + 4 || x >= sb.x + sb.w - 4 { return None; }
        if let Some(button) = app.tool_selector.anchored_button(app.mode, right, x, y) {
            return Some(Control::Sidebar(button, right));
        }
        let opening = app.drawing_tools.animating();
        if !opening && app.document_scroll_enabled {
            if scroll_hit_rect(app, right).contains(x, y)
                && (app.wants_scroll_rail() || app.scroll_rail.mix > 0.25) {
                return Some(Control::DocumentScroll(right));
            }
        }
        let buttons = expanded_sidebar_buttons(app, right).into_iter()
            .filter(|(button, _)| !app.wants_scroll_rail() || !matches!(button, Btn::Prev | Btn::Next))
            .filter(|(button, _)| !opening || button.tool_mode().is_some())
            .map(|(button, rect)| (Control::Sidebar(button, right), rect));
        let page = (!opening && app.sidebar_page_visible()).then(|| (Control::PageCount(right), sidebar_page_rect(app, right)));
        // Split each gap at its midpoint. A release near the neighbour must
        // resolve to exactly one target, including alongside the page count.
        return buttons.chain(page).filter_map(|(control, rect)| {
            let distance = (rect.y - y).max(y - (rect.y + rect.h - 1)).max(0);
            let reach = if control.tool_segment().is_some() { 3 } else { 2 };
            (distance <= reach).then_some((distance, control))
        }).min_by_key(|(distance, _)| *distance).map(|(_, control)| control);
    }
    None
}

pub fn sidebar_button_at(app: &App, x: i32, y: i32) -> Option<(Btn, bool)> {
    match sidebar_control_at(app, x, y) {
        Some(Control::Sidebar(button, right)) => Some((button, right)),
        _ => None,
    }
}

pub fn sidebar_buttons(app: &App, right: bool) -> Vec<(Btn, R)> {
    let sb = sidebar_rect(app, right);
    if app.collapsed {
        return vec![(Btn::Collapse, sb)];
    }
    expanded_sidebar_buttons(app, right)
}

fn sidebar_pen_rect(app: &App, right: bool) -> R {
    let sb = expanded_sidebar_rect(app, right);
    let btn = app.sb_btn();
    let slot = if app.whiteboard { 1 } else { 2 };
    R { x: sb.x + (sb.w - btn) / 2, y: sb.y + 12 + slot * (btn + 6), w: btn, h: btn }
}

fn tool_segments(app: &App, right: bool) -> [(Btn, R); 3] {
    let pen = sidebar_pen_rect(app, right);
    let step = pen.h + 6;
    [(Btn::Cursor, R { y: pen.y - step, ..pen }), (Btn::Pen, pen),
        (Btn::Eraser, R { y: pen.y + step, ..pen })]
}

const TOOL_TRACK_INSET: i32 = 3;

pub(crate) fn tool_track_rect(app: &App, right: bool) -> R {
    let [(_, first), (_, pen), _] = tool_segments(app, right);
    // The visual well sits just inside the key faces. The full touch slots and
    // the drag gutters still use the original sidebar geometry.
    R { x: first.x + 1, y: first.y + 1, w: first.w - 2,
        h: pen.y + pen.h - first.y - 2 + app.eraser_row_height() }
}

pub(crate) fn tool_track_hit(app: &App, right: bool, x: i32, y: i32) -> Option<(Btn, f32)> {
    let (button, side) = control_at(app, x, y)?.tool_segment()?;
    if side != right { return None; }
    let pen = sidebar_pen_rect(app, right);
    let position = 1.0 + (y - (pen.y + pen.h / 2)) as f32 / (pen.h + 6) as f32;
    let last = if app.eraser_row_height() == pen.h + 6 { 2.0 } else { 1.0 };
    Some((button, position.clamp(0.0, last)))
}

fn expanded_sidebar_buttons(app: &App, right: bool) -> Vec<(Btn, R)> {
    let pen = sidebar_pen_rect(app, right);
    let step = pen.h + 6;
    let mut v = Vec::with_capacity(11);
    if !app.whiteboard { v.push((Btn::Collapse, R { y: pen.y - 2 * step, ..pen })); }
    let [cursor, pen_button, eraser] = tool_segments(app, right);
    v.extend([cursor, pen_button]);
    if app.eraser_row_height() == step { v.push(eraser); }
    v.extend(drawing_tool_buttons(app, right).filter(|(_, _, height)| *height == step)
        .map(|(button, rect, _)| (button, rect)));
    // Borrow four pixels from within the navigation group for the gap after
    // the drawing tools. The rail footprint and everything below Next stay put.
    let group_gap = if app.sidebar_page_space() > 0 { 4 } else { 0 };
    let mut y = navigation_top(app, pen);
    for k in [Btn::Prev, Btn::Next, Btn::Exit, Btn::Whiteboard, Btn::Screenshot] {
        v.push((k, R { y, ..pen }));
        y += step;
        if k == Btn::Prev { y += app.sidebar_page_space() - group_gap; }
    }
    v
}

fn navigation_top(app: &App, pen: R) -> i32 {
    pen.y + pen.h + 6 + app.drawing_tools_height() + if app.sidebar_page_space() > 0 { 4 } else { 0 }
}

pub(crate) fn scroll_track_rect(app: &App, right: bool) -> R {
    let pen = sidebar_pen_rect(app, right);
    let gap = if app.sidebar_page_space() > 0 { 4 } else { 0 };
    R { y: navigation_top(app, pen), h: 2 * pen.h + 6 + app.sidebar_page_space() - gap, ..pen }
}

pub(crate) fn scroll_hit_rect(app: &App, right: bool) -> R {
    let bar = sidebar_rect(app, right);
    let track = scroll_track_rect(app, right);
    R { x: bar.x + 4, w: bar.w - 8, y: track.y - 2, h: track.h + 4 }
}

pub(crate) fn scroll_thumb_geometry(app: &App, right: bool) -> (i32, f32, i32) {
    let r = scroll_track_rect(app, right);
    let badge = page_counter_face(app, sidebar_page_rect(app, right));
    let middle = badge.y + badge.h / 2;
    let height = badge.h.clamp(14, 26);
    let travel = (middle - r.y).min(r.y + r.h - middle) - height / 2 - 4;
    (middle, travel.max(1) as f32, height)
}

pub(crate) fn scroll_notice_rect(app: &App, right: bool) -> R {
    let track = scroll_track_rect(app, right);
    let width = 180.min(app.screen_w.max(1));
    let x = if right { track.x - width - 12 } else { track.x + track.w + 12 };
    R { x: x.clamp(0,app.screen_w-width), y: (track.y+track.h/2-16).clamp(0,(app.screen_h-32).max(0)), w:width,h:32 }
}

fn draw_scroll_navigation(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool) {
    let r = scroll_track_rect(app, right);
    let badge = page_counter_face(app, sidebar_page_rect(app, right));
    let (middle, travel, height) = scroll_thumb_geometry(app, right);
    let blend = app.scroll_rail.mix;
    let area = R { x: r.x.min(badge.x), w: r.w.max(badge.w), ..r };
    let palette = ctx.palette;
    let cx = (r.x + r.w / 2) as f32;
    let radius = ((r.w as f32) * 0.30).min(12.0);
    draw_with_opacity(app, ctx, area, blend, |ctx| {
        surface(ctx, r, radius, c(palette.surface), c(palette.control_border()));
        ctx.stroke_line((cx, (r.y + 11) as f32),(cx, (r.y + r.h - 11) as f32),alpha(palette.muted,84),1.4);
        for (y, sign) in [(r.y + 10, -1.0), (r.y + r.h - 10, 1.0)] {
            let y = y as f32;
            ctx.stroke_polyline(&[(cx-3.5,y),(cx,y+sign*3.0),(cx+3.5,y)],c(palette.muted),1.5,false);
        }
    });
    let interpolate = |a: i32,b: i32| (a as f32 + (b-a) as f32 * blend).round() as i32;
    let width = interpolate(badge.w, r.w - 8);
    let h = interpolate(badge.h, height);
    let thumb = R { x: r.x + (r.w-width)/2, y: middle - h/2 + (app.scroll_rail.thumb * travel * blend).round() as i32, w: width, h };
    draw_with_opacity(app, ctx, area, 1.0 - blend, |ctx| {
        let btn = app.sb_btn();
        let shift = (blend * btn as f32 * 0.18).round() as i32;
        draw_btn(app,fonts,ctx,Btn::Prev,right,R {y:r.y+shift,h:btn,..r});
        draw_btn(app,fonts,ctx,Btn::Next,right,R {y:r.y+r.h-btn-shift,h:btn,..r});
        let numbers = if app.sidebar_page_visible() { sidebar_page_numbers(app) } else { app.scroll_rail.last_page };
        draw_page_face(app,fonts,ctx,right,thumb,numbers);
    });
    draw_with_opacity(app, ctx, area, blend, |ctx| {
        let held = app.scroll_rail.held == Some(right);
        let fill = if held { palette.accent_light } else { palette.paper };
        let border = if held { palette.accent } else { palette.control_border() };
        surface(ctx,thumb,(h as f32*0.36).min(8.0),c(fill),c(border));
        let cy = (thumb.y + thumb.h/2) as f32;
        if held && app.scroll_rail.unavailable {
            icons::draw(ctx,Glyph::Info,cx-7.0,cy-7.0,14.0,c(palette.muted),c(palette.muted));
        } else {
            let handle = (thumb.w as f32 * 0.23).clamp(3.0,7.0);
            for dy in [-1.5, 1.5] { ctx.stroke_line((cx-handle,cy+dy),(cx+handle,cy+dy),c(palette.muted),1.5); }
        }
    });
}

fn drawing_tool_buttons(app: &App, right: bool) -> impl Iterator<Item = (Btn, R, i32)> {
    let pen = sidebar_pen_rect(app, right);
    let tools = app.eraser_row_height();
    let action = app.clear_row_height();
    let y = pen.y + pen.h + 6 + tools;
    // A page switch can leave the old slot on screen until the finger lifts.
    // Only the page owning the snapshot gets a Restore control.
    let button = if app.can_restore_cleared() { Some(Btn::Restore) }
        else if app.drawing_tools.expanded() || tools > 0 { Some(Btn::Clear) } else { None };
    [button.map(|b| (b, R { y, ..pen }, action)),
        Some((Btn::Undo, R { y: y + action, ..pen }, tools))].into_iter().flatten()
}

pub fn sidebar_page_rect(app: &App, right: bool) -> R {
    let sb = expanded_sidebar_rect(app, right);
    let pen = sidebar_pen_rect(app, right);
    R { x: sb.x + 4, y: pen.y + 2 * (pen.h + 6) + app.drawing_tools_height() + 2,
        w: sb.w - 8, h: app.sb_page_h() }
}

pub fn sidebar_page_values(app: &App) -> (String, String) {
    page_labels(sidebar_page_numbers(app))
}

pub(crate) fn sidebar_page_numbers(app: &App) -> (i32, i32) {
    if app.whiteboard {
        (app.current_slide, crate::app::CACHE_BOARD)
    } else if app.wps_debug && app.wps_connected {
        (app.wps_page_index, app.wps_page_total)
    } else { (-1, -1) }
}

fn page_labels((page, total): (i32, i32)) -> (String, String) {
    let value = |n: i32| if n > 0 { n.to_string() } else { "—".to_string() };
    (value(page), value(total))
}

fn page_counter_face(app: &App, hit: R) -> R {
    let h = (28.0 * app.sb_scale).round().clamp(24.0, 36.0) as i32;
    R { y: hit.y + (hit.h - h) / 2, h, ..hit }
}

pub fn page_count_at(app: &App, x: i32, y: i32) -> Option<bool> {
    if !app.sidebar_page_visible() { return None; }
    match sidebar_control_at(app, x, y) {
        Some(Control::PageCount(right)) => Some(right),
        _ => None,
    }
}

pub fn page_picker_rect(app: &App) -> R {
    let sb = sidebar_rect(app, app.popup_on_right);
    let available = if app.popup_on_right { sb.x } else { app.screen_w - sb.x - sb.w };
    let max_w = if available >= 200 { available - 12 } else { app.screen_w };
    let w = (if app.page_preview_enabled() { 378 } else { 212 }).min(max_w);
    let h = (if app.page_preview_enabled() { 254 } else { 242 }).min(app.screen_h);
    let anchor = sidebar_page_rect(app, app.popup_on_right);
    R { x: (if app.popup_on_right { sb.x - w - 8 } else { sb.x + sb.w + 8 })
            .clamp(0, (app.screen_w - w).max(0)),
        y: (anchor.y + anchor.h / 2 - h / 2).clamp(0, (app.screen_h - h).max(0)), w, h }
}

fn page_picker_body(p: R) -> R {
    let top = 52.min(p.h / 4);
    let footer = 68.min(p.h / 3);
    R { x: p.x + 12, y: p.y + top, w: (p.w - 24).max(1), h: (p.h - top - footer).max(1) }
}

fn page_wheel_width(body: R) -> i32 { (body.w / 4).clamp(52, 76).min(body.w) }

pub fn page_picker_thumb_rect(app: &App) -> R {
    let body = page_picker_body(page_picker_rect(app));
    if !app.page_preview_enabled() { return R { w: 0, h: 0, ..body }; }
    R { w: (body.w - page_wheel_width(body) - 10).max(1), ..body }
}

pub fn page_picker_control_rect(app: &App, hit: PagePickerHit) -> R {
    let p = page_picker_rect(app);
    let body = page_picker_body(p);
    let width = ((body.w - 8) / 2).clamp(1, 100);
    let footer = R { x: body.x, y: body.y + body.h + 10, w: width, h: (p.y + p.h - body.y - body.h - 22).clamp(1, 48) };
    match hit {
        PagePickerHit::Wheel if app.page_preview_enabled() => {
            let w = page_wheel_width(body);
            R { x: body.x + body.w - w, w, ..body }
        }
        PagePickerHit::Wheel => body,
        PagePickerHit::Input => {
            let w = (p.w - 106).clamp(60, 150).min(body.w);
            R { x: body.x + body.w - w, y: p.y + 2, w, h: (body.y - p.y - 4).max(1) }
        }
        PagePickerHit::Cancel => footer,
        PagePickerHit::Jump => R { x: body.x + body.w - width, ..footer },
    }
}

pub fn page_picker_hit(app: &App, x: i32, y: i32) -> Option<PagePickerHit> {
    [PagePickerHit::Wheel, PagePickerHit::Input, PagePickerHit::Cancel, PagePickerHit::Jump].into_iter()
        .find(|hit| page_picker_control_rect(app, *hit).contains(x, y))
}

#[path = "ui/pen_palette.rs"]
mod pen_palette;
pub use pen_palette::{pen_popup_hit, pen_popup_rect};
use pen_palette::{draw_pen_popup, pen_controls};

#[path = "ui/eraser_palette.rs"]
mod eraser_palette;
pub use eraser_palette::{eraser_popup_hit, eraser_popup_rect};
use eraser_palette::{draw_eraser_popup, eraser_controls};


#[path = "ui/whiteboard.rs"]
mod whiteboard;
#[path = "ui/whiteboard_preview.rs"]
mod whiteboard_preview;
pub use whiteboard::{direction_menu_rect, selection_bar_rect, whiteboard_bar_rect,
    whiteboard_edit_bar_rect, whiteboard_hit, whiteboard_ui_contains};
use whiteboard::draw_whiteboard_controls;
pub(crate) fn whiteboard_fraction_at(app: &App, p: crate::conic::Parameter, x: i32) -> f32 {
    whiteboard::fraction_at(app, p, x)
}
pub(crate) fn whiteboard_panel_contains(app: &App, x: i32, y: i32) -> bool {
    whiteboard::panel_contains(app, x, y)
}
pub(crate) fn whiteboard_value_rect(app: &App, field: crate::conic::Parameter) -> Option<R> {
    whiteboard::control_bounds(app, WhiteboardControl::Value(field))
}

fn board_palette(app: &App) -> theme::Palette {
    let preference = if app.whiteboard_bg_index == 0 { theme::Preference::Dark } else { theme::Preference::Light };
    theme::Theme { preference, ..app.theme }.palette()
}
#[cfg(test)]
use whiteboard::{wb_button_rect, wb_slider_rect, wb_action_rect};

pub fn coord_origin(app: &App) -> Option<(f32, f32)> {
    app.coord.as_ref().map(|c| (c.origin_x, c.origin_y))
}

pub fn coord_rotate_handle(app: &App) -> Option<(f32, f32)> {
    app.coord.as_ref().map(|c| {
        let (sin, cos) = c.rotation.sin_cos();
        let dy = if whiteboard::top_editor(app) { 40.0 } else { -50.0 };
        let dx = if dy > 0.0 { 40.0 } else { 50.0 };
        (c.origin_x + dx * cos - dy * sin, c.origin_y + dx * sin + dy * cos)
    })
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
            rows.push((ProcessRow::Rule(index),rule.clone(),if rule.contains('/') {"完整路径"}else{"进程名"}.into()));
        }
        return rows;
    }
    for (index, proc) in app.process_scan.iter().enumerate() {
        let mut detail = format!("PID {}", proc.pid);
        if !proc.exe.is_empty() { detail.push_str("  "); detail.push_str(&proc.exe); }
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
        let lines=crate::settings_panel::wrapped(fonts,"退出会清除当前批注。需要保留的内容请先截图。",14.0,p.w-48);
        for (i,line) in lines.iter().enumerate() {
            if 72+i as i32*24>p.h-72 {break;}
            ctx.text(fonts,line,(p.x+24) as f32,(p.y+72+i as i32*24) as f32,14.0,c(palette.muted));
        }
        for (hit,r) in confirm_controls(app) {
            if hit==ConfirmHit::Yes {
                let r=R {y:r.y+6,h:r.h-12,..r};
                surface(ctx,r,6.0,c(palette.danger_light),c(palette.danger));
                ctx.text_center(fonts,r,"退出",14.0,c(palette.text));
            } else {crate::settings_panel::button(ctx,fonts,r,"取消",false);}
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
        0 => "授权后，当前用户可读取触屏和键盘输入。\n注销并重新登录后生效。",
        1 => "请在系统窗口中授权。",
        2 => "已授权，重新登录后生效。",
        _ => "未获得输入权限。",
    };
    let lines=crate::settings_panel::wrapped(fonts,body,14.0,p.w-48);
    for (i,line) in lines.iter().enumerate() {
        if 78+i as i32*25>p.h-72 {break;}
        ctx.text(fonts,line,(p.x+24) as f32,(p.y+78+i as i32*25) as f32,14.0,c(palette.muted));
    }
    for (hit,r) in input_group_prompt_controls(app) {
        crate::settings_panel::button(ctx,fonts,r,match hit {PromptHit::Authorize=>"授权",PromptHit::Close if app.input_group_state==0=>"稍后",_=>"关闭"},hit==PromptHit::Authorize);
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
            "[Desktop Entry]\nType=Application\nName=Sidera\nComment=屏幕批注与白板\nExec=sidera --autostart\nIcon={}\nTerminal=false\n",
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
        palette: board_palette(app),
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
        draw_axis_ends(&mut ctx, &fonts.for_settings(), &coord, app.screen_w, app.screen_h);
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
            if let Some(bounds) = coord_bounds(coord, 0, 0, app.screen_w, app.screen_h) {
                let mut ctx = Ctx { palette: board_palette(app), pm: &mut pm, tr };
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
        let mut ctx = Ctx { palette: board_palette(app), pm: &mut pm, tr };
        let board_fonts = fonts.for_settings();
        if let Some(coord) = &app.coord { draw_coord_foci(coord, &board_fonts, &mut ctx); }
        draw_selection_outline(app, &mut ctx);
        draw_coord_handles(app, &mut ctx);
    }

    if app.launch.active() {
        let mut ctx = Ctx { palette: app.palette(), pm: &mut pm, tr };
        for right in [false, true] { draw_launch_sidebar(app, fonts, &mut ctx, right); }
        let r = launch_rect(app);
        if app.launch.panel_alpha() > 0.0 && x < r.x + r.w && x2 > r.x && y < r.y + r.h && y2 > r.y {
            let (cw, ch) = (((r.w as f32) * sc).round().max(1.0) as u32,
                ((r.h as f32) * sc).round().max(1.0) as u32);
            let mut cache = app.launch.card.borrow_mut();
            if cache.as_ref().is_none_or(|card| card.pixels.width() != cw || card.pixels.height() != ch) {
                *cache = Some(crate::splash_paint::Card::new(fonts, icon, cw, ch, palette));
            }
            if let Some(card) = cache.as_mut() {
                card.update(fonts, &app.launch);
                ctx.pm.draw_pixmap(0, 0, card.pixels.as_ref(), &tiny_skia::PixmapPaint {
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
            if whiteboard_bar_rect(app).x as f32 > fonts.text_width(&s, 20.0) + 24.0 {
                ctx.text(fonts, &s, 16.0, app.screen_h as f32 - 22.0, 20.0, page_ink);
            }
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
                    "已翻回第 1 页",
                    34.0,
                    page_ink,
                );
            }
        }

        if app.whiteboard { draw_whiteboard_controls(app, fonts, &mut ctx); }

        let sidebar_progress = if app.previewing_sidebars() { 0.0 } else { app.sidebar_retract() };
        let opacity = if app.toolbar_can_fade() { app.presentation_fade.opacity() } else { 1.0 };
        for right in [false, true] {
            if opacity < 1.0 {
                draw_with_opacity(app, &mut ctx, sidebar_input_rect(app, right), opacity,
                    |ctx| draw_sidebar(app, fonts, ctx, right, sidebar_progress));
            } else {
                draw_sidebar(app, fonts, &mut ctx, right, sidebar_progress);
            }
        }

        if let Some(right) = app.scroll_rail.held.filter(|_| app.scroll_rail.unavailable && app.wants_scroll_rail() && !app.collapsed) {
            let r = scroll_notice_rect(app,right);
            surface(&mut ctx,r,7.0,c(palette.paper),c(palette.control_border()));
            let notice = "当前窗口无法接收滚动";
            ctx.text_center(fonts,r,notice,13.0,c(palette.text));
        }
        if app.pen_popup_visible {
            draw_pen_popup(app, fonts, &mut ctx);
        }
        if app.eraser_popup_visible {
            draw_eraser_popup(app, fonts, &mut ctx);
        }
        if app.page_picker.is_some() { draw_page_picker(app, fonts, &mut ctx); }
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
        crate::settings_panel::draw_overlay(app, fonts, &mut ctx);
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
    let palette = app.palette();
    pixmap.fill(c(palette.paper));
    let mut ctx = Ctx { palette, pm: pixmap, tr: Transform::from_scale(scale, scale) };
    if app.settings_open {
        draw_settings(app, fonts, &mut ctx, icon);
    }
    if app.process_block_open {
        draw_process_block(app, fonts, &mut ctx);
    }
    if app.show_diag {
        draw_diag(app, fonts, &mut ctx);
    }
    if app.confirm_quit {
        draw_confirm(app, fonts, &mut ctx);
    }
    crate::settings_panel::draw_overlay(app, fonts, &mut ctx);
    draw_press_feedback(app, &mut ctx);
}

fn launch_piece_opacity(target: R, visible: R, r: R) -> f32 {
    let space = (r.y - visible.y).min(visible.y + visible.h - r.y - r.h);
    let final_space = (r.y - target.y).min(target.y + target.h - r.y - r.h).clamp(1, 18);
    // A button fades in only after it fits completely: no squashed icons or
    // sliced labels along the moving top/bottom edges.
    (space as f32 / final_space as f32).clamp(0.0, 1.0)
}

fn draw_launch_piece(app: &App, ctx: &mut Ctx, target: R, visible: R, r: R,
    draw: impl FnOnce(&mut Ctx)) {
    if r.w <= 0 || r.h <= 0 { return; }
    let opacity = launch_piece_opacity(target, visible, r);
    if opacity <= 0.0 { return; }
    let scale = app.scale as f32;
    let Some(mut tile) = Pixmap::new((r.w as f32 * scale).ceil() as u32,
        (r.h as f32 * scale).ceil() as u32) else { return; };
    draw(&mut Ctx { palette: app.palette(), pm: &mut tile,
        tr: Transform::from_scale(scale, scale).pre_translate(-r.x as f32, -r.y as f32) });
    ctx.pm.draw_pixmap(0, 0, tile.as_ref(), &tiny_skia::PixmapPaint { opacity, ..Default::default() },
        ctx.tr.pre_translate(r.x as f32, r.y as f32).pre_scale(1.0 / scale, 1.0 / scale), None);
}

fn draw_sidebar_shell(app: &App, ctx: &mut Ctx, sb: R, held: bool) {
    let p = ctx.palette;
    let radius = (sb.w as f32 * 0.27).min(17.0).min(sb.h as f32 / 2.0);
    let rim = if held { p.accent } else { theme::mix(p.border, p.text, 0.22) };
    let shell = if held { p.accent_light } else { p.surface };
    surface(ctx, sb, radius, alpha(shell, app.sidebar_alpha), c(rim));
    // Keep the housing translucent. Each touch key has its own opaque face.
    if sb.h > app.screen_h && app.screen_h > 16 {
        let travel = (app.screen_h - 12) as f32;
        let fraction = app.screen_h as f32 / sb.h as f32;
        let top = 6.0 + (-sb.y as f32 / sb.h as f32) * travel;
        let edge = if sb.x < app.screen_w / 2 { sb.x + 2 } else { sb.x + sb.w - 3 };
        ctx.stroke_line((edge as f32, top), (edge as f32, top + travel * fraction), c(p.muted), 2.0);
    }
    let cx = (sb.x + sb.w / 2) as f32;
    let grip = (app.sb_dot() as f32 * 0.32).round();
    for y in [sb.y + 6, sb.y + sb.h - 5] {
        if held {
            let w = (app.sb_dot() + 6).clamp(20, sb.w - 6);
            ctx.fill_round(R { x: sb.x + (sb.w - w) / 2, y: y - 4, w, h: 8 }, 4.0, c(p.accent));
        }
        ctx.stroke_line((cx - grip, y as f32), (cx + grip, y as f32),
            c(if held { p.on_accent } else { p.muted }), 2.0);
    }
}

fn draw_launch_sidebar(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool) {
    if app.avoid_active { return; }
    let p = app.launch.reveal();
    if p <= 0.0 { return; }
    if p >= 1.0 { draw_sidebar(app, fonts, ctx, right, app.sidebar_retract()); return; }
    let target = sidebar_rect(app, right);
    let visible = sidebar_launch_rect(app, right);
    if app.collapsed {
        let face = sidebar_tab_face(app, right);
        draw_sidebar_tab(app, ctx, right, face, false, 1.0, launch_piece_opacity(target, visible, face));
        return;
    }
    draw_sidebar_shell(app, ctx, visible, false);
    draw_launch_piece(app, ctx, target, visible, tool_track_rect(app, right).with_paint_margin(),
        |local| draw_tool_track(app, fonts, local, right));
    for (button, r) in expanded_sidebar_buttons(app, right) {
        if button.tool_mode().is_some() || (app.document_scroll_enabled && matches!(button, Btn::Prev | Btn::Next)) { continue; }
        draw_launch_piece(app, ctx, target, visible, r, |local| draw_btn(app, fonts, local, button, right, r));
    }
    if app.document_scroll_enabled {
        draw_launch_piece(app, ctx, target, visible, scroll_track_rect(app, right).with_paint_margin(),
            |local| draw_scroll_navigation(app, fonts, local, right));
    } else if app.sidebar_page_visible() {
        draw_launch_piece(app, ctx, target, visible, sidebar_page_rect(app, right),
            |local| draw_sidebar_pages(app, fonts, local, right));
    }
}

fn draw_sidebar(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool, progress: f32) {
    if app.avoid_active && !app.settings_open { return; }
    let drag = app.sidebar_drag.filter(|drag| drag.right == right);
    let held = drag.is_some();
    if progress < 1.0 {
        let sb = expanded_sidebar_rect(app, right);
        let dx = (sb.w + 4) as f32 * progress * if right { 1.0 } else { -1.0 };
        let mut moving = Ctx { palette: app.palette(), pm: &mut *ctx.pm, tr: ctx.tr.pre_translate(dx, 0.0) };
        draw_sidebar_shell(app, &mut moving, sb, held);
        draw_tool_track(app, fonts, &mut moving, right);
        for (k, r) in expanded_sidebar_buttons(app, right) {
            if k.tool_mode().is_none() && !(app.document_scroll_enabled && matches!(k, Btn::Prev | Btn::Next)) {
                draw_btn(app, fonts, &mut moving, k, right, r);
            }
        }
        draw_opening_tools(app, fonts, &mut moving, right);
        if app.document_scroll_enabled { draw_scroll_navigation(app, fonts, &mut moving, right); }
        else { draw_sidebar_pages(app, fonts, &mut moving, right); }
    }
    if progress > 0.0 {
        let face = sidebar_tab_face(app, right);
        draw_sidebar_tab(app, ctx, right, face, held, progress, 1.0);
    }
}

fn draw_sidebar_tab(app: &App, ctx: &mut Ctx, right: bool, face: R, held: bool, progress: f32, opacity: f32) {
    if opacity <= 0.0 { return; }
    let scale = app.scale as f32;
    let edge_gap = if right { app.screen_w - face.x - face.w } else { face.x };
    let travel = (face.w + 2) as f32 * (1.0 - progress);
    let gap_px = ((edge_gap as f32 - travel) * scale).round();
    let (origin_px, direction) = if right { (app.canvas_w() as f32 - gap_px, -1.0) } else { (gap_px, 1.0) };
    let origin = ctx.tr.pre_translate(origin_px / scale, face.y as f32);
    let (x, y) = (origin.tx.round() as i32, origin.ty.round() as i32);
    let pixel_w = (face.w as f32 * scale).ceil() as i32;
    let pixel_h = (face.h as f32 * scale).ceil() as i32;
    let left = if right { x - pixel_w } else { x };
    if left >= ctx.pm.width() as i32 || left + pixel_w <= 0
        || y >= ctx.pm.height() as i32 || y + pixel_h <= 0 { return; }

    // Mirror the finished pixels so even the curved edges and dash caps have identical coverage.
    let Some(mut tile) = Pixmap::new(pixel_w as u32, pixel_h as u32) else { return };
    let tr = Transform::from_scale(scale, scale);
    let p = ctx.palette;
    let mut tab = Ctx { palette: p, pm: &mut tile, tr };
    let radius = (face.w as f32 * 0.22).round();
    tab.fill_round(R { x: 0, y: 0, ..face }, radius, c(if held { p.accent_light } else { p.paper }));

    let width = ((face.w as f32 * 0.09).clamp(2.0, 3.0) * scale).round() / scale;
    let inset = width * 0.5;
    let (w, h) = (face.w as f32 - width, face.h as f32 - width);
    let rim_radius = radius - inset;
    if let Some(path) = rounded_path(inset, inset, w, h, rim_radius) {
        let mut stroke = Stroke { width, line_cap: LineCap::Round, ..Default::default() };
        let level = app.sidebar_countdown;
        let track = theme::mix(p.paper, p.border, 0.8);
        tab.pm.stroke_path(&path, &solid(c(track)), &stroke, tr, None);
        if level > 0 {
            if level < u8::MAX {
                // Each quadratic corner in rounded_path has a length of 1.623225 × its radius.
                let straight = 2.0 * (w + h - 4.0 * rim_radius);
                let perimeter = straight + 4.0 * 1.623225 * rim_radius;
                let length = perimeter * f32::from(level) / f32::from(u8::MAX);
                stroke.dash = StrokeDash::new(vec![length, perimeter], 0.0);
            }
            tab.pm.stroke_path(&path, &solid(c(p.accent)), &stroke, tr, None);
        }
    }
    let cx = face.w as f32 / 2.0;
    let cy = face.h as f32 / 2.0;
    let wing = face.w as f32 * 0.12;
    let half_height = face.w as f32 * 0.22;
    tab.stroke_polyline(&[(cx - wing, cy - half_height), (cx + wing, cy), (cx - wing, cy + half_height)],
        c(if held { p.accent } else { theme::mix(p.muted, p.accent, 0.42) }), 2.0, false);
    let placement = Transform::from_translate(x as f32, y as f32).pre_scale(direction, 1.0);
    ctx.pm.draw_pixmap(0, 0, tile.as_ref(), &tiny_skia::PixmapPaint { opacity, ..Default::default() }, placement, None);
}

fn draw_tool_track(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool) {
    let p = ctx.palette;
    let track = tool_track_rect(app, right);
    let well = theme::mix(p.surface, p.border, 0.16);
    let segments = tool_segments(app, right);
    let first = segments[0].1;
    let step = first.h + 6;
    let eraser_height = app.eraser_row_height();
    let position = app.tool_selector.position(app.mode).min(1.0 + eraser_height as f32 / step as f32);
    let pad = TOOL_TRACK_INSET;
    let size = track.w - 2 * pad;
    let thumb = R { x: track.x + pad, y: track.y + pad + (position * (first.h + 6) as f32).round() as i32, w: size, h: size };
    let radius = (size as f32 * 0.42).round();
    surface(ctx, track, radius + pad as f32, c(well), c(p.control_border()));
    for (index, (_, slot)) in segments[1..].iter().enumerate() {
        if index == 1 && eraser_height < step { continue; }
        // Only separate the two idle tools. Fade the line as the thumb approaches.
        let opacity = ((position - (index as f32 + 0.5)).abs() - 0.5).clamp(0.0, 1.0);
        if opacity <= 0.0 { continue; }
        let y = (slot.y - 3) as f32;
        ctx.stroke_line(((slot.x + 7) as f32, y), ((slot.x + slot.w - 7) as f32, y),
            alpha(theme::mix(p.border, p.muted, 0.30), (255.0 * opacity).round() as u8), 1.0);
    }
    let rim = if theme::contrast(p.accent, well) < 3.0 { p.text } else { theme::mix(p.accent, p.text, 0.22) };
    let contact = app.press.filter(|press| press.active && !press.expired()
        && press.control.tool_segment().is_some_and(|(_, side)| side == right));
    let (fill, rim) = match contact {
        Some(press) if press.until.is_none() => (theme::mix(p.accent, p.on_accent, 0.18), theme::mix(rim, p.on_accent, 0.80)),
        Some(_) => (theme::mix(p.accent, p.on_accent, 0.09), theme::mix(rim, p.on_accent, 0.47)),
        None => (p.accent, rim),
    };
    surface(ctx, thumb, radius, c(fill), c(rim));
    for (tool, r) in segments {
        if tool != Btn::Eraser || eraser_height == step { draw_btn(app, fonts, ctx, tool, right, r); }
    }
}

fn draw_opening_tools(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool) {
    if !app.drawing_tools.animating() { return; }
    let (eraser, rect) = tool_segments(app, right)[2];
    let buttons = std::iter::once((eraser, rect, app.eraser_row_height())).chain(drawing_tool_buttons(app, right));
    let scale = app.scale as f32;
    for (button, r, height) in buttons {
        let step = r.h + 6;
        if height >= step || height * 2 <= step { continue; }
        let reveal = height as f32 / step as f32;
        let opacity = 2.0 * reveal - 1.0;
        let Some(mut tile) = Pixmap::new((r.w as f32 * scale).ceil() as u32,
            (r.h as f32 * scale).ceil() as u32) else { continue };
        let mut local = Ctx { palette: app.palette(), pm: &mut tile,
            tr: Transform::from_scale(scale, scale).pre_translate(-r.x as f32, -r.y as f32) };
        draw_btn(app, fonts, &mut local, button, right, r);
        ctx.pm.draw_pixmap(0, 0, tile.as_ref(), &tiny_skia::PixmapPaint {
            opacity, ..Default::default()
        }, ctx.tr.pre_translate(r.x as f32 + r.w as f32 * (1.0 - reveal) / 2.0,
            r.y as f32).pre_scale(reveal / scale, reveal / scale), None);
    }
}

fn draw_sidebar_pages(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool) {
    if !app.sidebar_page_visible() { return; }
    let r = page_counter_face(app, sidebar_page_rect(app, right));
    draw_page_face(app, fonts, ctx, right, r, sidebar_page_numbers(app));
}

fn draw_page_face(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool, r: R, numbers: (i32,i32)) {
    let palette = ctx.palette;
    let (page, total) = page_labels(numbers);
    let text = format!("{page} / {total}");
    let available_picker = app.page_picker_context().is_some();
    let opened = app.page_picker.is_some() && app.popup_on_right == right;
    let (fill, border) = if opened {
        (palette.accent_light, palette.accent)
    } else if available_picker {
        (palette.surface, palette.control_border())
    } else {
        (palette.surface, palette.border)
    };
    button_surface(app, ctx, Control::PageCount(right), r, 6.0, c(fill), c(border));
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
    let nominal = (r.w as f32 * 0.34).round().max(9.0).min((r.h - 2).max(1) as f32);
    let width = fonts.text_width(label, nominal).max(1.0);
    let size = nominal * ((r.w - 2) as f32 / width).min(1.0);
    ctx.text_center(fonts, r, label, size, ink);
}

fn centered_glyph(ctx: &mut Ctx, glyph: Glyph, r: R, size: f32, ink: Color, accent: Color) {
    let size = size.min(r.w.min(r.h) as f32 * 0.82);
    let x = r.x as f32 + (r.w as f32 - size) / 2.0;
    let y = r.y as f32 + (r.h as f32 - size) / 2.0;
    icons::draw(ctx, glyph, x, y, size, ink, accent);
}

fn draw_btn(app: &App, fonts: &Fonts, ctx: &mut Ctx, k: Btn, right: bool, r: R) {
    let palette = ctx.palette;
    let btn = app.sb_btn() as f32;
    let radius = (r.w as f32 * 0.17).min(9.0);
    if k == Btn::Exit && !app.whiteboard {
        draw_presentation_button(app, fonts, ctx, right, r, radius);
        return;
    }
    let control = Control::Sidebar(k, right);
    let ink = match k {
        Btn::Exit => {
            let bg = app.whiteboard_bg_color();
            button_surface(app, ctx, control, r, radius, bg, c(palette.control_border()));
            theme::on_color(bg)
        }
        Btn::Whiteboard if app.whiteboard => {
            button_surface(app, ctx, control, r, radius, c(palette.danger_light), c(palette.danger));
            c(palette.danger)
        }
        Btn::Cursor | Btn::Pen | Btn::Eraser => c(palette.text),
        _ => {
            button_surface(app, ctx, control, r, radius, c(palette.paper), c(palette.control_border()));
            c(if k == Btn::Clear { theme::mix(palette.danger, palette.text, 0.30) } else { palette.text })
        }
    };
    let saved = k == Btn::Screenshot && app.screenshot_feedback.visible();
    let (glyph, caption) = match k {
        Btn::Collapse if !app.toolbar_icons_only => return draw_button_label(fonts, ctx, r, "收起", ink),
        Btn::Exit if !app.toolbar_icons_only => {
            ctx.text_multiline_center(fonts, r, "背景\n颜色", (btn * 0.42).max(10.0), ink);
            return;
        }
        Btn::Collapse => (if right { Glyph::CollapseRight } else { Glyph::CollapseLeft }, None),
        Btn::Exit => (Glyph::BoardColor, None),
        Btn::Cursor => (Glyph::Cursor, None), Btn::Pen => (Glyph::Pen, None), Btn::Eraser => (Glyph::Eraser, None),
        Btn::Prev => (Glyph::Up, None), Btn::Next => (Glyph::Down, None),
        Btn::Screenshot if saved => (Glyph::Check, Some("已保存")),
        Btn::Screenshot => (Glyph::Screenshot, Some("截图")),
        Btn::Clear => (Glyph::Trash, Some("清空")), Btn::Restore => (Glyph::Restore, Some("恢复")),
        Btn::Undo => (Glyph::Undo, Some("撤销")),
        Btn::Whiteboard if app.whiteboard => (Glyph::CloseBoard, Some("退出")),
        Btn::Whiteboard => (Glyph::Board, Some("白板")),
    };
    if let Some(label) = caption.filter(|_| !app.toolbar_icons_only || saved) {
        let icon_height = if matches!(glyph, Glyph::CloseBoard) { r.h * 2 / 3 } else { r.h * 3 / 5 };
        let top = R { y: r.y + 1, h: icon_height, ..r };
        centered_glyph(ctx, glyph, top, app.sb_icon() as f32, ink, ink);
        let bottom = R { y: top.y + top.h, h: r.h - top.h - 3, ..r };
        ctx.text_center(fonts, bottom, label, (r.w as f32 * 0.27).min(16.0), ink);
        return;
    }
    let factor = if k == Btn::Eraser {
        let last = crate::app::ERASER_SIZES.len() - 1;
        0.86 + 0.26 * app.cur_eraser.min(last) as f32 / last as f32
    } else { 1.0 };
    centered_glyph(ctx, glyph, r, app.sb_icon() as f32 * factor, ink, app.pen_color());
}

fn draw_presentation_button(app: &App, fonts: &Fonts, ctx: &mut Ctx, right: bool, r: R, radius: f32) {
    let palette = ctx.palette;
    let weights = app.presentation_transition.weights();
    let blend = |colors: [u32; 3]| {
        let channel = |shift: u32| -> u8 {
            colors.iter().zip(weights).map(|(rgb, weight)| ((rgb >> shift) & 255) as f32 * weight)
                .sum::<f32>().round().clamp(0.0, 255.0) as u8
        };
        Color::from_rgba8(channel(16), channel(8), channel(0), 255)
    };
    button_surface(app, ctx, Control::Sidebar(Btn::Exit, right), r, radius,
        blend([palette.paper, palette.success, palette.danger_light]),
        blend([palette.control_border(), palette.success, palette.danger]));
    // Fade through: only one label is visible, so Esc and the two-line Chinese
    // captions never overlap. Deriving opacity from the same sampled weights
    // also keeps interrupted transitions continuous, on both sidebars.
    for (index, (label, glyph, ink, factor)) in [
        ("Esc", None, palette.text, 0.43),
        ("开始\n放映", Some(Glyph::StartShow), palette.on_success, 0.36),
        ("结束\n放映", Some(Glyph::EndShow), palette.danger, 0.36),
    ].into_iter().enumerate() {
        let opacity = (2.0 * weights[index] - 1.0).clamp(0.0, 1.0);
        if opacity <= 0.0 { continue; }
        let ink = alpha(ink, (255.0 * opacity).round() as u8);
        if let Some(glyph) = glyph.filter(|_| app.toolbar_icons_only) {
            centered_glyph(ctx, glyph, r, app.sb_icon() as f32, ink, ink);
        } else {
            ctx.text_multiline_center(fonts, r, label, (app.sb_btn() as f32 * factor).round().max(9.0), ink);
        }
    }
}

fn draw_fading_popup(app: &App, ctx: &mut Ctx, p: R, draw: impl FnOnce(&mut Ctx)) {
    draw_with_opacity(app, ctx, p, anim_t(app), draw);
}

fn draw_with_opacity(app: &App, ctx: &mut Ctx, p: R, opacity: f32, draw: impl FnOnce(&mut Ctx)) {
    let p = p.with_paint_margin();
    let (tr, w, h) = region_origin(app, p.x, p.y, p.x + p.w, p.y + p.h);
    let x = (ctx.tr.tx - tr.tx).round() as i32;
    let y = (ctx.tr.ty - tr.ty).round() as i32;
    if x >= ctx.pm.width() as i32 || y >= ctx.pm.height() as i32
        || x + w as i32 <= 0 || y + h as i32 <= 0 { return; }
    if opacity >= 1.0 { draw(ctx); return; }
    if opacity <= 0.0 { return; }
    let Some(mut tile) = Pixmap::new(w, h) else { return };
    draw(&mut Ctx { palette: ctx.palette, pm: &mut tile, tr });
    // Composite on the same device pixels as the settled frame, including
    // when this repaint covers only a dirty region at fractional scale.
    ctx.pm.draw_pixmap(x, y, tile.as_ref(), &tiny_skia::PixmapPaint { opacity, ..Default::default() },
        Transform::identity(), None);
}

fn draw_page_picker(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    draw_fading_popup(app, ctx, page_picker_rect(app), |ctx| draw_page_picker_content(app, fonts, ctx));
}

fn draw_page_picker_content(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let palette = ctx.palette;
    let Some(picker) = &app.page_picker else { return };
    let p = page_picker_rect(app);
    panel(ctx, p, 12.0, 1.0);
    let input = page_picker_control_rect(app, PagePickerHit::Input);
    let error = app.settings_ui.editor.as_ref()
        .filter(|e| e.target == crate::value_input::Target::Page && !e.error.is_empty());
    let heading = if let Some(editor) = error { editor.error.as_str() } else { "选页" };
    let room = (input.x - p.x - 22).max(1) as f32;
    let nominal = if error.is_some() { 13.0 } else { 17.0 };
    let size = nominal * (room / fonts.text_width(heading, nominal).max(1.0)).min(1.0);
    let baseline = input.y as f32 + (input.h as f32 + size * 0.72) / 2.0;
    ctx.text(fonts, heading, (p.x + 15) as f32, baseline, size, c(if error.is_some() { palette.danger } else { palette.text }));
    let suffix = format!(" / {}", picker.total);
    let size = 16.0 * ((input.w - 10).max(1) as f32 / fonts.text_width(&format!("{}{suffix}", picker.total), 16.0).max(1.0)).min(1.0);
    crate::value_input::draw_value(app, fonts, ctx, crate::value_input::Target::Page, input, size, "", &suffix);
    let thumb = page_picker_thumb_rect(app);
    if thumb.h > 0 {
        surface(ctx, thumb, 8.0, c(palette.surface), c(palette.border));
        let page = picker.selected();
        match picker.source {
            crate::page_picker::Source::Whiteboard => {
                let inset = R { x: thumb.x + 4, y: thumb.y + 4, w: thumb.w - 8, h: thumb.h - 8 };
                if let Some(image) = whiteboard_preview::render(app, fonts, picker, page, inset) {
                    draw_thumbnail(ctx, inset, &image);
                }
            }
            crate::page_picker::Source::Wps(epoch) => match app.thumbs.get(&(epoch, page)) {
                Some(image) => draw_thumbnail(ctx, R { x: thumb.x + 3, y: thumb.y + 3, w: thumb.w - 6, h: thumb.h - 6 }, image),
                None => centered_glyph(ctx, Glyph::Document, thumb, 28.0, c(palette.muted), c(palette.muted)),
            },
        }
    }
    let wheel = page_picker_control_rect(app, PagePickerHit::Wheel);
    surface(ctx, wheel, 8.0, c(palette.surface), c(palette.control_border()));
    let center = wheel.y as f64 + wheel.h as f64 / 2.0;
    let band = R { x: wheel.x + 4, y: center as i32 - 19, w: wheel.w - 8, h: 38 };
    ctx.fill_round(band, 5.0, c(palette.accent_light));
    if picker.dragging() { ctx.stroke_round(band, 5.0, c(palette.accent_line), 1.0); }
    let nearest = picker.position.round() as i32;
    // Only five nearby rows are painted, independent of the deck's page count.
    for page in nearest.saturating_sub(3).max(1)..=nearest.saturating_add(3).min(picker.total) {
        let distance = page as f64 - picker.position;
        if distance.abs() > 2.2 { continue; }
        let size = (24.0 - 7.0 * distance.abs()).max(15.0) as f32;
        if (distance * crate::page_picker::ROW).abs() + f64::from(size) / 2.0 > f64::from(wheel.h) / 2.0 { continue; }
        let ink = theme::mix(palette.text, palette.muted, distance.abs().min(1.0) as f32);
        let text = page.to_string();
        let fit = size * ((wheel.w - 12).max(1) as f32 / fonts.text_width(&text, size).max(1.0)).min(1.0);
        let x = wheel.x as f32 + (wheel.w as f32 - fonts.text_width(&text, fit)) / 2.0;
        ctx.text(fonts, &text, x, (center + distance * crate::page_picker::ROW) as f32 + fit * 0.35,
            fit, c(ink));
    }
    for (hit, title, selected) in [
        (PagePickerHit::Cancel, "取消", false),
        (PagePickerHit::Jump, "跳转", true),
    ] {
        let r = page_picker_control_rect(app, hit);
        button_surface(app, ctx, Control::PagePicker(hit), r, 7.0, c(if selected { palette.accent } else { palette.paper }),
            c(if selected { palette.accent } else { palette.control_border() }));
        ctx.text_center(fonts, r, title, 15.0, c(if selected { palette.on_accent } else { palette.text }));
    }
}

fn draw_thumbnail(ctx: &mut Ctx, bounds: R, pm: &Pixmap) {
    let scale = (bounds.w as f32 / pm.width() as f32).min(bounds.h as f32 / pm.height() as f32);
    let x = bounds.x as f32 + (bounds.w as f32 - pm.width() as f32 * scale) / 2.0;
    let y = bounds.y as f32 + (bounds.h as f32 - pm.height() as f32 * scale) / 2.0;
    ctx.pm.draw_pixmap(0, 0, pm.as_ref(),
        &tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() },
        ctx.tr.pre_translate(x, y).pre_scale(scale, scale), None);
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
            ctx.text(fonts, &format!("{u:.0}"), p.0 + 3.0, p.1 - 4.0, 13.0, c(palette.muted));
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
            ctx.text(fonts, &format!("{v:.0}"), p.0 + 4.0, p.1 + 4.0, 13.0, c(palette.muted));
        }
        v += label_step;
    }
}

/// Mark the positive ends even after rotating the guide. Screen edges are
/// inset so a label stays on the board at fractional display scales.
fn draw_axis_ends(ctx: &mut Ctx, fonts: &Fonts, g: &crate::app::CoordGuide, w: i32, h: i32) {
    let palette = ctx.palette;
    let (ox, oy) = (g.origin_x, g.origin_y);
    if ox < 0. || oy < 0. || ox >= w as f32 || oy >= h as f32 { return; }
    let (sin, cos) = g.rotation.sin_cos();
    let limit = |origin: f32, delta: f32, extent: i32| {
        if delta > 0.001 { (extent as f32 - 18. - origin) / delta }
        else if delta < -0.001 { (16. - origin) / delta } else { f32::INFINITY }
    };
    for (dx,dy,label) in [(cos,sin,"x"),(sin,-cos,"y")] {
        let mut len = limit(ox,dx,w).min(limit(oy,dy,h));
        if g.conic == crate::app::ConicKind::None { len = len.min(20. * crate::conic::size_scale(g.size) * g.scale); }
        if len < 26. { continue; }
        let (x,y) = (ox+len*dx, oy+len*dy);
        ctx.stroke_polyline(&[(x-7.*dx-3.5*dy,y-7.*dy+3.5*dx),(x,y),
            (x-7.*dx+3.5*dy,y-7.*dy-3.5*dx)],c(palette.text),1.6,false);
        ctx.text(fonts,label,x-12.*dx+5.*dy,y-12.*dy-5.*dx,12.,c(palette.text));
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

fn draw_coord_foci(coord: &crate::app::CoordGuide, fonts: &Fonts, ctx: &mut Ctx) {
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
        ctx.stroke_selection(&points, true);
    } else if !app.lasso.is_empty() {
        ctx.stroke_selection(&app.lasso, app.lasso.len() > 2);
    }
}

fn draw_coord_handles(app: &App, ctx: &mut Ctx) {
    let Some(coord) = app.coord.as_ref() else { return };
    if !coord.editing || app.conic_menu.is_some() { return; }
    let palette = ctx.palette;
    if let Some((hx, hy)) = coord_rotate_handle(app) {
        ctx.stroke_line((coord.origin_x, coord.origin_y), (hx, hy), alpha(palette.accent, 140), 1.5);
        ctx.fill_circle(hx, hy, 11.5, c(palette.accent));
        ctx.fill_circle(hx, hy, 9.5, c(palette.paper));
        icons::draw(ctx,Glyph::Reset,hx-7.5,hy-7.5,15.,c(palette.text),c(palette.accent));
    }
    let (x,y) = (coord.origin_x,coord.origin_y);
    ctx.fill_circle(x,y,9.,c(palette.accent));
    ctx.fill_circle(x,y,7.,c(palette.paper));
    ctx.stroke_line((x-3.5,y),(x+3.5,y),c(palette.text),1.6);
    ctx.stroke_line((x,y-3.5),(x,y+3.5),c(palette.text),1.6);
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

    fn assert_press_fits_face(app: &mut App, fonts: &Fonts, control: Control, face: R, radius: f32) {
        let tile = face.with_paint_margin();
        let render = |app: &App| render_region(app, fonts, None, tile.x, tile.y, tile.w, tile.h).unwrap();
        app.press = None;
        let idle = render(app);
        app.press = Some(PressFeedback::new(control));
        let down = render(app);
        let (tr, w, h) = region_origin(app, tile.x, tile.y, tile.x + tile.w, tile.y + tile.h);
        let mut mask = Pixmap::new(w, h).unwrap();
        surface(&mut Ctx { pm: &mut mask, tr, palette: app.palette() }, face, radius, Color::WHITE, Color::WHITE);
        for ((a, b), m) in idle.pixels().iter().zip(down.pixels()).zip(mask.pixels()) {
            if m.alpha() == 0 { assert_eq!(a, b, "{control:?} painted outside its face at {}×", app.scale); }
        }
        // A perpendicular cut through each straight side crosses the existing
        // rim and the fill. Neither may leave a strip of the idle face visible.
        let (x, y) = (face.x as f32, face.y as f32);
        let (w, h) = (face.w as f32, face.h as f32);
        for d in [0.5, 1.5, 2.5] {
            for (px, py) in [(x+d, y+h/2.0), (x+w-d, y+h/2.0), (x+w/2.0, y+d), (x+w/2.0, y+h-d)] {
                let (i, j) = ((px*tr.sx + tr.tx).floor() as u32, (py*tr.sy + tr.ty).floor() as u32);
                assert_ne!(idle.pixel(i, j), down.pixel(i, j), "{control:?} missed ({px}, {py}) at {}×", app.scale);
            }
        }
        app.press = None;
    }

    #[test]
    fn toolbar_press_fits_the_key_thumb_badge_and_menu_faces() {
        let fonts = Fonts::load();
        let mut app = App::new(Pixmap::new(900, 720).unwrap(), 900, 720);
        app.whiteboard = true;
        for preference in [theme::Preference::Light, theme::Preference::Dark] {
            app.theme.preference = preference;
            for scale in [1.0, 1.25, 1.5, 2.0] {
                app.set_scale(scale);
                for right in [false, true] {
                    app.popup_on_right = right;
                    let next = Control::Sidebar(Btn::Next, right);
                    let key = control_rect(&app, next).unwrap();
                    assert_press_fits_face(&mut app, &fonts, next, key, (key.w as f32 * 0.17).min(9.0));
                    let badge = page_counter_face(&app, sidebar_page_rect(&app, right));
                    assert_press_fits_face(&mut app, &fonts, Control::PageCount(right), badge, 6.0);
                    let track = tool_track_rect(&app, right);
                    let size = track.w - 2 * TOOL_TRACK_INSET;
                    let thumb = R { x: track.x + TOOL_TRACK_INSET, y: track.y + TOOL_TRACK_INSET, w: size, h: size };
                    assert_press_fits_face(&mut app, &fonts, Control::Sidebar(Btn::Cursor, right), thumb, (size as f32 * 0.42).round());
                    app.eraser_popup_visible = true;
                    let clear = Control::Popup(PopupHit::ClearAll);
                    let face = control_rect(&app, clear).unwrap();
                    assert_press_fits_face(&mut app, &fonts, clear, face, 9.0);
                    app.eraser_popup_visible = false;
                }
            }
        }
    }

    #[test]
    fn collapsed_tab_countdown_drains_one_continuous_rim_in_both_themes() {
        let fonts = Fonts::load();
        let mut app = App::new(Pixmap::new(640, 360).unwrap(), 640, 360);
        app.set_sidebar_collapsed(true);
        app.sidebar_anim_start = None;
        for (preference, accent) in [
            (theme::Preference::Light, theme::Accent::Teal),
            (theme::Preference::Dark, theme::Accent::Teal),
            (theme::Preference::Light, theme::Accent::Custom(0xDDD7EF)),
            (theme::Preference::Dark, theme::Accent::Custom(0x453B54)),
        ] {
            app.theme.preference = preference;
            app.theme.accent = accent;
            let accent_rgb = app.palette().accent;
            let ink = c(accent_rgb).to_color_u8().premultiply();
            for scale in [1.0, 1.25, 2.0] {
                app.set_scale(scale);
                for right in [false, true] {
                    let face = sidebar_tab_face(&app, right);
                    let mut frame = |level| {
                        app.sidebar_countdown = level;
                        render_region(&app, &fonts, None, face.x, face.y, face.w, face.h).unwrap()
                    };
                    let empty = frame(0);
                    let half = frame(128);
                    let full = frame(255);
                    let changed = |a: &Pixmap, b: &Pixmap| a.pixels().iter().zip(b.pixels()).filter(|(p, q)| p != q).count();
                    let total = changed(&full, &empty);
                    let remaining = changed(&half, &empty);
                    assert!(total > 80 && remaining > total * 4 / 10 && remaining < total * 6 / 10);
                    let y = full.height() / 2;
                    let solid_band = (0..full.width() / 4).filter(|&x| full.pixel(x, y) == Some(ink)).count();
                    assert!(solid_band >= (2.0 * scale).floor() as usize);
                    let track = empty.pixel(scale.round() as u32, y).unwrap();
                    assert!(theme::contrast(accent_rgb, u32::from_be_bytes([0, track.red(), track.green(), track.blue()])) >= 3.0);
                    if scale == 1.0 {
                        for edge in [0, full.height() - 1] {
                            let flat = (0..full.width()).filter(|&x| full.pixel(x, edge) == Some(ink)).count();
                            assert!(flat >= full.width() as usize / 2);
                            assert_eq!(full.pixel(0, edge).unwrap().alpha(), 0);
                        }
                    }
                    // Half the time left lights the edge facing the middle of the screen.
                    for x in 0..full.width() {
                        let inward = (x < full.width() / 2) == right;
                        let reference = if inward { &full } else { &empty };
                        assert_eq!(half.pixel(x, y), reference.pixel(x, y));
                    }
                }
            }
        }
    }

    #[test]
    fn collapsed_tabs_mirror_the_whole_countdown_at_fractional_scales() {
        let fonts = Fonts::load();
        let mut app = App::new(Pixmap::new(641, 360).unwrap(), 641, 360);
        app.set_sidebar_collapsed(true);
        app.sidebar_anim_start = None;
        for (scale, toolbar_scale) in [(1.0, 1.0), (1.25, 0.6), (1.25, 1.0), (1.5, 1.4), (2.0, 1.0)] {
            app.set_scale(scale);
            app.sb_scale = toolbar_scale;
            for level in [0, 32, 64, 128, 192, 224, 255] {
                app.sidebar_countdown = level;
                let [left, right] = [false, true].map(|side| {
                    let r = sidebar_tab_rect(&app, side);
                    let x = if side { app.screen_w - 64 } else { 0 };
                    render_region(&app, &fonts, None, x, r.y, 64, r.h).unwrap()
                });
                assert_eq!((left.width(), left.height()), (right.width(), right.height()));
                let stride = left.width() as usize * 4;
                for (l, r) in left.data().chunks_exact(stride).zip(right.data().chunks_exact(stride)) {
                    assert!(l.chunks_exact(4).eq(r.chunks_exact(4).rev()), "scale {scale}, countdown {level}");
                }
            }
        }
    }

    #[test]
    fn toolbar_thumb_only_divides_idle_tools_and_shows_the_touch_on_its_own_side() {
        let fonts = Fonts::load();
        let mut app = App::new(Pixmap::new(900, 720).unwrap(), 900, 720);
        app.drawing_tools.set_expanded(true);
        app.drawing_tools.settle();
        for side in [false, true] {
            let track = tool_track_rect(&app, side);
            let segments = tool_segments(&app, side);
            let rail = sidebar_rect(&app, side);
            assert!(track.x - rail.x >= 6 && rail.x + rail.w - track.x - track.w >= 6);
            let render = |app: &App| render_region(app, &fonts, None, track.x, track.y, track.w, track.h).unwrap();
            for (i, (button, _)) in segments.iter().enumerate() {
                app.mode = i as i32;
                app.press = None;
                let rest = render(&app);
                for (gap, (_, slot)) in segments[1..].iter().enumerate() {
                    let x = (track.w / 2) as u32;
                    let y = (slot.y - 3 - track.y) as u32;
                    assert_eq!(rest.pixel(x, y) != rest.pixel(x, y - 1), i != gap && i != gap + 1);
                }
                app.press = Some(PressFeedback::new(Control::Sidebar(*button, !side)));
                assert_eq!(render(&app).data(), rest.data());
                app.press = Some(PressFeedback::new(Control::Sidebar(*button, side)));
                assert_ne!(render(&app).data(), rest.data());
                app.press.as_mut().unwrap().release();
                assert_ne!(render(&app).data(), rest.data());
            }
            app.mode = 0;
            app.tool_selector.begin(Btn::Cursor, side, 0, 0);
            app.tool_selector.motion(70, Some((Btn::Pen, 1.4)));
            app.press = None;
            let moving = render(&app);
            app.press = Some(PressFeedback::new(Control::Sidebar(Btn::Cursor, side)));
            let pressed = render(&app);
            let cursor_end = (segments[0].1.y + segments[0].1.h - track.y) as usize * moving.width() as usize * 4;
            assert_eq!(&moving.data()[..cursor_end], &pressed.data()[..cursor_end]);
            assert!(moving.data()[cursor_end..] != pressed.data()[cursor_end..]);
            app.tool_selector.cancel_press();
            app.press = None;
            let hit = sidebar_page_rect(&app, side);
            let face = page_counter_face(&app, hit);
            assert_eq!(face.h, 28);
            assert!(hit.h > face.h && hit.y + hit.h / 2 == face.y + face.h / 2);
        }
    }

    #[test]
    fn toolbar_icon_mode_hides_the_screenshot_caption_and_keeps_esc() {
        let fonts = Fonts::load();
        let mut app = App::new(Pixmap::new(900, 720).unwrap(), 900, 720);
        let render = |app: &App, button| {
            let r = control_rect(app, Control::Sidebar(button, false)).unwrap();
            render_region(app, &fonts, None, r.x, r.y, r.w, r.h).unwrap()
        };
        let esc = render(&app, Btn::Exit);
        let screenshot = render(&app, Btn::Screenshot);
        app.toolbar_icons_only = true;
        assert!(esc.data() == render(&app, Btn::Exit).data());
        assert!(screenshot.data() != render(&app, Btn::Screenshot).data());
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
                for expected in [SetHit::Autostart, SetHit::ToolbarIconsOnly, SetHit::FadeToolbarDuringShow,
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
        a.settings_ui.tab = crate::settings_state::Tab::Appearance;
        a.settings_page = usize::MAX;
        let compact = settings_layout(&a);
        assert!(compact.pages > 1);
        assert_eq!(compact.page + 1, compact.pages);
        assert!(settings_controls(&a).iter().any(|(hit, _)| *hit == SetHit::ToolbarIconsOnly));
        a.screen_h = 1080;
        assert_eq!(settings_layout(&a).page, 0);
        assert_eq!(settings_layout(&a).pages, 1);
        assert!(settings_controls(&a).iter().any(|(hit, _)| *hit == SetHit::ResetSlider(SliderId::Position)));
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
                // The upper two thirds now contain the shared artwork.
                assert_eq!(pixels.pixel((4.0 * scale) as u32, pixels.height() * 4 / 5).unwrap(),
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
        for (field,slider) in [(crate::conic::Parameter::Size,WhiteboardControl::SizeSlider),
            (crate::conic::Parameter::Shape,WhiteboardControl::ShapeSlider)] {
            assert!(control_rect(&app,Control::Whiteboard(WhiteboardControl::ParameterTab(field))).is_some());
            app.guide_parameter = field;
            assert!(control_rect(&app,Control::Whiteboard(slider)).is_some());
        }
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
                wb_slider_rect(&app, crate::conic::Parameter::Zoom), wb_slider_rect(&app, crate::conic::Parameter::Shape),
                wb_slider_rect(&app, crate::conic::Parameter::Size), wb_action_rect(&app, WhiteboardControl::Done),
                wb_action_rect(&app, WhiteboardControl::Delete),
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
