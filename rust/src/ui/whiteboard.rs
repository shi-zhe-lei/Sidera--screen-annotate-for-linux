//! The dock, its contextual tray and their pointer/touch targets share one layout.
use super::*;
use crate::app::{ConicDirection as Direction, ConicKind as Kind};
use crate::conic::Parameter;
use crate::value_input::{self, Target};
use WhiteboardControl as W;

const TOOLS: [(W, &str); 5] = [
    (W::Empty, "坐标系"), (W::Ellipse, "椭圆"), (W::Hyperbola, "双曲线"),
    (W::Parabola, "抛物线"), (W::Select, "框选"),
];
const CURVE_PARAMS: &[Parameter] = &[Parameter::Zoom, Parameter::Shape, Parameter::Size];
const GRID_PARAMS: &[Parameter] = &[Parameter::Zoom, Parameter::Size];

fn parameters(app: &App) -> &'static [Parameter] {
    if app.coord.is_some_and(|g| g.conic != Kind::None) { CURVE_PARAMS } else { GRID_PARAMS }
}

fn kind_of(tool: W) -> Option<Kind> {
    match tool {
        W::Empty => Some(Kind::None), W::Ellipse => Some(Kind::Ellipse),
        W::Hyperbola => Some(Kind::Hyperbola), W::Parabola => Some(Kind::Parabola),
        _ => None,
    }
}

// Reserve room for each vertical toolbar only where it intersects this tray.
fn horizontal_span(app: &App, y: i32, h: i32) -> (i32, i32) {
    let mut left = 6;
    let mut right = app.screen_w - 6;
    for side in [false, true] {
        let rail = sidebar_input_rect(app, side);
        if rail.y < y + h && y < rail.y + rail.h {
            if side { right = right.min(rail.x - 4); }
            else { left = left.max(rail.x + rail.w + 4); }
        }
    }
    if right <= left { (0, app.screen_w.max(1)) } else { (left, right) }
}

fn tray(app: &App, y: i32, h: i32, width: i32, center: i32) -> R {
    let y = y.max(0);
    let (left, right) = horizontal_span(app, y, h);
    let w = width.min(right - left);
    R { x: (center - w / 2).clamp(left, right - w), y, w, h }
}

pub fn whiteboard_bar_rect(app: &App) -> R {
    let h = 60;
    tray(app, app.screen_h - h - 12, h, 514, app.screen_w / 2)
}

pub(super) fn wb_button_rect(app: &App, index: usize) -> R {
    let b = whiteboard_bar_rect(app);
    let usable = (b.w - 18).max(5);
    let i = index as i32;
    R { x: b.x + 5 + usable * i / 5 + if i == 4 { 8 } else { 0 }, y: b.y + 5,
        w: usable * (i + 1) / 5 - usable * i / 5, h: b.h - 10 }
}

pub fn direction_menu_rect(app: &App, kind: Kind) -> R {
    let index = TOOLS.iter().position(|(w, _)| kind_of(*w) == Some(kind)).unwrap_or(0);
    let b = wb_button_rect(app, index);
    let count = kind.directions().len() as i32;
    tray(app, whiteboard_bar_rect(app).y - 96, 90, 16 + count * 72, b.x + b.w / 2)
}

fn direction_option_rect(app: &App, kind: Kind, index: usize) -> R {
    let r = direction_menu_rect(app, kind);
    let n = kind.directions().len().max(1) as i32;
    let i = index as i32;
    let span = r.w - 12;
    R { x: r.x + 6 + i * span / n, y: r.y + 29, w: (i + 1) * span / n - i * span / n, h: 55 }
}

fn narrow_editor(app: &App) -> bool {
    let bottom = whiteboard_bar_rect(app).y - 6;
    let (left, right) = horizontal_span(app, bottom - 110, 110);
    right - left < 440
}

pub(super) fn top_editor(app: &App) -> bool {
    narrow_editor(app) && app.screen_h < 430
}

pub fn whiteboard_edit_bar_rect(app: &App) -> R {
    let bottom = whiteboard_bar_rect(app).y - 6;
    let h = if narrow_editor(app) { 105 } else if app.screen_h < 430 { 90 } else { 110 };
    let y = if top_editor(app) { 6 } else { bottom - h };
    tray(app, y, h, 620, app.screen_w / 2)
}

fn active_parameter(app: &App) -> Parameter {
    if parameters(app).contains(&app.guide_parameter) { app.guide_parameter } else { Parameter::Zoom }
}

fn tab_rect(app: &App, field: Parameter) -> R {
    let r = whiteboard_edit_bar_rect(app);
    let params = parameters(app);
    let i = params.iter().position(|&p| p == field).unwrap_or(0) as i32;
    let n = params.len() as i32;
    let span = r.w - 12;
    R {x:r.x + 6 + i * span / n, y:r.y + 35, w:(i + 1) * span / n - i * span / n, h:27}
}

fn parameter_visible(app: &App, field: Parameter) -> bool {
    parameters(app).contains(&field) && (!narrow_editor(app) || active_parameter(app) == field)
}

pub(super) fn wb_action_rect(app: &App, hit: W) -> R {
    let r = whiteboard_edit_bar_rect(app);
    let width = if r.w < 260 { 35 } else { 50 };
    let index = match hit { W::Done => 1, W::Delete => 2, _ => 3 };
    R { x: r.x + r.w - 5 - index * width, y: r.y + 3, w: width - 2, h: 32 }
}

fn parameter_slot(app: &App, p: Parameter) -> R {
    let bar = whiteboard_edit_bar_rect(app);
    if narrow_editor(app) {
        return R { x: bar.x + 8, y: bar.y + 63, w: bar.w - 16, h: 30 };
    }
    let params = parameters(app);
    let i = params.iter().position(|&field| field == p).unwrap_or(0) as i32;
    let span = bar.w - 16;
    let n = params.len() as i32;
    R { x: bar.x + 8 + i * span / n, y: bar.y + 34,
        w: (i + 1) * span / n - i * span / n, h: if bar.h == 90 { 39 } else { 51 } }
}

fn value_rect(app: &App, p: Parameter) -> R {
    let r = parameter_slot(app, p);
    let w = 66.min(r.w / 3 + 7);
    R { x: r.x + r.w - w, y: r.y, w, h: if narrow_editor(app) {30} else {22} }
}

pub(super) fn wb_slider_rect(app: &App, p: Parameter) -> R {
    let r = parameter_slot(app, p);
    if narrow_editor(app) {
        R { x: r.x, y: r.y, w: (value_rect(app,p).x - r.x).max(1), h: r.h }
    } else {
        R { x: r.x + 4, y: r.y + 22, w: (r.w - 8).max(1), h: r.h - 22 }
    }
}

fn rail(app: &App, p: Parameter) -> (f32, f32, f32) {
    let r = wb_slider_rect(app, p);
    let inset = 9.min((r.w - 1).max(0) / 2);
    ((r.x + inset) as f32, (r.x + r.w - inset - 1) as f32, (r.y + r.h / 2) as f32)
}

pub(super) fn fraction_at(app: &App, p: Parameter, x: i32) -> f32 {
    let (a, b, _) = rail(app, p);
    ((x as f32 - a) / (b - a).max(1.0)).clamp(0.0, 1.0)
}

pub fn selection_bar_rect(app: &App) -> R {
    let bottom = whiteboard_bar_rect(app).y - 6;
    let (l, r) = horizontal_span(app, bottom - 78, 78);
    let h = if r - l < 335 { 76 } else { 48 };
    tray(app, bottom - h, h, 390, app.screen_w / 2)
}

fn selection_action(app: &App, hit: W) -> R {
    let r = selection_bar_rect(app);
    let last = if app.floating.is_some() && hit == W::SelCancel { 2 } else { 1 };
    R { x: r.x + r.w - 6 - last * 56, y: r.y + r.h - 40, w: 53, h: 34 }
}

fn editing(app: &App) -> bool {
    app.coord.is_some_and(|g| g.editing) && app.conic_menu.is_none()
}

pub(super) fn control_bounds(app: &App, hit: W) -> Option<R> {
    if !app.whiteboard { return None; }
    if let Some(i) = TOOLS.iter().position(|&(tool, _)| tool == hit) {
        return Some(wb_button_rect(app, i));
    }
    match hit {
        W::Direction(k, d) if app.conic_menu == Some(k) => k.directions().iter()
            .position(|&(item, _)| item == d).map(|i| direction_option_rect(app, k, i)),
        W::SelCancel if app.select_mode || app.floating.is_some() => Some(selection_action(app, hit)),
        W::SelDone if app.floating.is_some() => Some(selection_action(app, hit)),
        _ if !editing(app) => None,
        W::Done | W::Delete | W::ResetView => Some(wb_action_rect(app, hit)),
        W::ParameterTab(p) if narrow_editor(app) && parameters(app).contains(&p) => Some(tab_rect(app,p)),
        W::Value(p) if parameter_visible(app,p) => Some(value_rect(app, p)),
        W::ZoomSlider if parameter_visible(app,Parameter::Zoom) => Some(wb_slider_rect(app, Parameter::Zoom)),
        W::ShapeSlider if parameter_visible(app,Parameter::Shape) => Some(wb_slider_rect(app, Parameter::Shape)),
        W::SizeSlider if parameter_visible(app,Parameter::Size) => Some(wb_slider_rect(app, Parameter::Size)),
        _ => None,
    }
}

pub fn whiteboard_hit(app: &App, x: i32, y: i32) -> Option<W> {
    if !app.whiteboard { return None; }
    let inside = |w: &W| control_bounds(app, *w).is_some_and(|r| r.contains(x, y));
    if whiteboard_bar_rect(app).contains(x, y) { return TOOLS.iter().map(|&(w, _)| w).find(inside); }
    if let Some(k) = app.conic_menu {
        return k.directions().iter().map(|&(d, _)| W::Direction(k, d)).find(inside);
    }
    if editing(app) {
        return [W::ResetView, W::Delete, W::Done, W::ZoomSlider, W::ShapeSlider, W::SizeSlider]
            .into_iter().chain(parameters(app).iter().copied().map(W::Value))
            .chain(parameters(app).iter().copied().map(W::ParameterTab)).find(inside);
    }
    [W::SelCancel, W::SelDone].into_iter().find(inside)
}

pub(super) fn panel_contains(app: &App, x: i32, y: i32) -> bool {
    app.whiteboard && (whiteboard_bar_rect(app).contains(x, y)
        || app.conic_menu.is_some_and(|k| direction_menu_rect(app, k).contains(x,y))
        || editing(app) && whiteboard_edit_bar_rect(app).contains(x, y)
        || (app.select_mode || app.floating.is_some()) && selection_bar_rect(app).contains(x, y))
}

pub fn whiteboard_ui_contains(app: &App, x: i32, y: i32) -> bool {
    panel_contains(app, x, y) || app.whiteboard && coord_handle_at(app, x, y).is_some()
}

fn card(ctx: &mut Ctx, r: R) {
    let p = ctx.palette;
    ctx.fill_round(R { x: r.x - 1, y: r.y + 2, w: r.w + 2, h: r.h + 1 }, 14.0, alpha(0x10131B, 24));
    surface(ctx, r, 13.0, c(p.paper), c(p.border));
}

fn centered(ctx: &mut Ctx, fonts: &Fonts, r: R, label: &str, px: f32, ink: u32) {
    wb_label(ctx, fonts, r, label, px, c(ink));
}

fn symbol(ctx: &mut Ctx, tool: W, dir: Direction, x: f32, y: f32, size: f32, ink: u32) {
    // Curves are drawn from the same orientation convention as the canvas.
    let angle = match dir { Direction::Vertical | Direction::Right => 90.0,
        Direction::Down => 180.0, Direction::Left => -90.0, _ => 0.0 };
    let tr = ctx.tr.pre_translate(x, y).pre_scale(size / 24.0, size / 24.0)
        .pre_concat(Transform::from_rotate_at(angle, 12.0, 12.0));
    let mut p = PathBuilder::new();
    match tool {
        W::Empty => {
            p.move_to(4.,17.); p.line_to(21.,17.); p.line_to(18.,14.);
            p.move_to(8.,21.); p.line_to(8.,3.); p.line_to(5.,6.);
            p.move_to(15.,15.); p.line_to(15.,19.);
        }
        W::Ellipse => p.push_oval(tiny_skia::Rect::from_xywh(2.5,6.,19.,12.).unwrap()),
        W::Hyperbola => {
            p.move_to(4.,3.); p.quad_to(14.,12.,4.,21.);
            p.move_to(20.,3.); p.quad_to(10.,12.,20.,21.);
        }
        W::Parabola => { p.move_to(3.,4.); p.cubic_to(8.,26.,16.,26.,21.,4.); }
        W::Select => {
            for (x,y,dx,dy) in [(4.,4.,1.,1.),(20.,4.,-1.,1.),(4.,20.,1.,-1.),(20.,20.,-1.,-1.)] {
                p.move_to(x + 5.*dx,y); p.line_to(x,y); p.line_to(x,y + 5.*dy);
            }
            p.move_to(9.,14.); p.line_to(14.,9.);
        }
        _ => return,
    }
    if let Some(path) = p.finish() {
        let stroke = Stroke { width: 1.7, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        ctx.pm.stroke_path(&path, &solid(c(ink)), &stroke, tr, None);
    }
}

fn draw_tools(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let p = ctx.palette;
    let b = whiteboard_bar_rect(app);
    card(ctx, b);
    let select = wb_button_rect(app, 4);
    ctx.stroke_line(((select.x - 4) as f32, (b.y + 13) as f32),
        ((select.x - 4) as f32, (b.y + b.h - 13) as f32), c(p.border), 1.);
    let selecting = app.select_mode || app.floating.is_some();
    let active_kind = app.conic_menu.or_else(|| app.coord.filter(|g| g.editing).map(|g| g.conic));
    for (i, &(w, label)) in TOOLS.iter().enumerate() {
        let r = wb_button_rect(app,i);
        let kind = kind_of(w);
        let active = if w == W::Select { selecting } else { !selecting && kind == active_kind };
        if active { surface(ctx, r, 9., c(p.accent_light), c(p.accent_line)); }
        let ink = if active { p.accent } else { p.text };
        let direction = app.coord.filter(|g| Some(g.conic) == kind).map_or(Direction::Up, |g| g.direction);
        if r.w >= 80 {
            let tw = fonts.text_width(label, 13.);
            let has_menu = matches!(w, W::Ellipse | W::Hyperbola | W::Parabola);
            let x = r.x as f32 + (r.w as f32 - 28. - tw - if has_menu {10.} else {0.}) * 0.5;
            symbol(ctx, w, direction, x, (r.y + 14) as f32, 22., ink);
            ctx.text(fonts, label, x + 28., (r.y + 30) as f32, 13., c(p.text));
            if has_menu {
                let (cx,cy) = (x + 34. + tw, (r.y + 25) as f32);
                ctx.stroke_polyline(&[(cx-2.5,cy+1.),(cx,cy-1.5),(cx+2.5,cy+1.)],c(p.muted),1.2,false);
            }
        } else {
            symbol(ctx, w, direction, (r.x + (r.w - 22) / 2) as f32, (r.y + 4) as f32, 22., ink);
            centered(ctx, fonts, R {y:r.y + 26, h:20, ..r}, label, 12., p.text);
        }
    }
}

fn draw_direction(app: &App, fonts: &Fonts, ctx: &mut Ctx, kind: Kind) {
    let palette = ctx.palette;
    let r = direction_menu_rect(app, kind);
    card(ctx,r);
    let title = match kind { Kind::Ellipse => "长轴方向", Kind::Hyperbola => "实轴方向", _ => "开口方向" };
    ctx.text(fonts, title, (r.x + 12) as f32, (r.y + 21) as f32, 13., c(palette.text));
    let Some(&(tool,_)) = TOOLS.iter().find(|(w,_)| kind_of(*w) == Some(kind)) else { return };
    for (i, &(d,label)) in kind.directions().iter().enumerate() {
        let slot = direction_option_rect(app,kind,i);
        let selected = app.coord.is_some_and(|g| g.conic == kind && g.direction == d);
        if selected { surface(ctx,slot,7.,c(palette.accent_light),c(palette.accent_line)); }
        symbol(ctx,tool,d,(slot.x + (slot.w-24)/2) as f32,(slot.y+4) as f32,24.,
            if selected { palette.accent } else { palette.text });
        centered(ctx,fonts,R {y:slot.y+29,h:22,..slot},label,12.,palette.text);
    }
}

fn action(ctx: &mut Ctx, fonts: &Fonts, r: R, w: W, label: &str) {
    let p = ctx.palette;
    let ink = match w {
        W::Done | W::SelDone => { ctx.fill_round(r,7.,c(p.accent)); p.on_accent }
        W::Delete => p.danger,
        _ => p.text,
    };
    centered(ctx,fonts,r,label,12.5,ink);
}

fn draw_editor(app: &App, fonts: &Fonts, ctx: &mut Ctx, g: &crate::app::CoordGuide) {
    let p = ctx.palette;
    let r = whiteboard_edit_bar_rect(app);
    card(ctx,r);
    let actions_x = wb_action_rect(app, W::ResetView).x;
    let short = if g.conic == Kind::None { "坐标系" } else { g.conic.label() };
    let direction = g.conic.directions().iter().find(|(d,_)| *d == g.direction);
    let name = match direction {
        Some((_,d)) if actions_x - r.x >= 118 => format!("{short} · {d}"),
        _ => short.to_owned(),
    };
    ctx.text(fonts,&name,(r.x+10) as f32,(r.y+24) as f32,14.,c(p.text));
    if r.w >= 510 {
        let metric = match g.conic {
            Kind::Ellipse | Kind::Hyperbola => {
                let (a,b) = if g.conic == Kind::Ellipse { crate::conic::ellipse_axes(g.shape,g.size) }
                    else { crate::conic::hyperbola_axes(g.shape,g.size) };
                format!("a = {a:.2}    b = {b:.2}")
            }
            Kind::Parabola => format!("焦距 = {:.2}",crate::conic::parabola_p(g.shape,g.size)),
            Kind::None => String::new(),
        };
        ctx.text(fonts,&metric,(r.x+22) as f32+fonts.text_width(&name,14.),(r.y+24) as f32,12.,c(p.muted));
    }
    for (w,label) in [(W::ResetView,"归位"),(W::Delete,"移除"),(W::Done,"完成")] {
        action(ctx,fonts,wb_action_rect(app,w),w,label);
    }
    let narrow = narrow_editor(app);
    for &field in parameters(app) {
        if narrow {
            let tab = tab_rect(app,field);
            let title = match field {
                Parameter::Zoom => "格距", Parameter::Size if g.conic == Kind::None => "范围",
                Parameter::Size => "大小", Parameter::Shape if g.conic == Kind::Parabola => "开口", Parameter::Shape => "轴比",
            };
            let active = field == active_parameter(app);
            if active { ctx.fill_round(tab, 5., c(p.accent_light)); }
            centered(ctx,fonts,tab,title,12.,if active {p.text} else {p.muted});
            if !active { continue; }
        }
        let slot = parameter_slot(app,field);
        let value = value_rect(app,field);
        let (title,_,_,_,unit) = field.display(g.conic);
        if !narrow {
            let label_width = (value.x - slot.x - 6).max(1) as f32;
            let px = 12.0 * (label_width / fonts.text_width(title,12.).max(1.)).min(1.);
            ctx.text(fonts,title,(slot.x+3) as f32,(slot.y+16) as f32,px,c(p.text));
        }
        value_input::draw_value(app,fonts,ctx,Target::Guide(field),value,12.,"",unit);
        let (start,end,y) = rail(app,field);
        let thumb = start + field.fraction(g)*(end-start);
        ctx.stroke_line((start,y),(end,y),c(p.border),3.);
        ctx.stroke_line((start,y),(thumb,y),c(p.accent),3.);
        ctx.fill_circle(thumb,y,6.,c(p.accent));
        ctx.fill_circle(thumb,y,3.2,c(p.paper));
    }
    let message = app.settings_ui.editor.as_ref().and_then(|e| value_input::message(app,e.target));
    let hint = message.as_deref().unwrap_or(if narrow { "点击数值可输入" } else if r.w >= 335 {
        "拖中心移动 · 拖圆柄旋转 · 点数值输入"
    } else { "拖中心移动 · 拖圆柄旋转" });
    let error = app.settings_ui.editor.as_ref().is_some_and(|e| !e.error.is_empty());
    let hint_height = if narrow {15} else if r.h == 90 {17} else {22};
    centered(ctx,fonts,R { x:r.x+6,y:r.y+r.h-hint_height-2,w:r.w-12,h:hint_height },
        hint, if narrow {10.5} else {11.5}, if error {p.danger} else {p.muted});
}

fn draw_selection(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let p = ctx.palette;
    let r = selection_bar_rect(app);
    card(ctx,r);
    let lifted = app.floating.is_some();
    let first = selection_action(app,W::SelCancel);
    let message = if lifted { "拖动选中的笔迹" } else if app.selection_missed { "未选中笔迹，请重新框选" } else { "拖出矩形，框选笔迹" };
    let text_area = R { x:r.x+9, y:r.y+4, w:if r.h>48 {r.w-18} else {first.x-r.x-15}, h:if r.h>48 {29} else {r.h-8} };
    ctx.text(fonts,message,(text_area.x+4) as f32,(text_area.y+text_area.h/2+5) as f32,13.,c(p.text));
    action(ctx,fonts,first,W::SelCancel,"取消");
    if lifted { action(ctx,fonts,selection_action(app,W::SelDone),W::SelDone,"完成"); }
}

pub(super) fn draw_whiteboard_controls(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let fonts = fonts.for_settings();
    draw_tools(app,&fonts,ctx);
    if let Some(kind) = app.conic_menu { draw_direction(app,&fonts,ctx,kind); }
    else if let Some(g) = app.coord.filter(|g| g.editing) { draw_editor(app,&fonts,ctx,&g); }
    else if app.select_mode || app.floating.is_some() { draw_selection(app,&fonts,ctx); }
}
