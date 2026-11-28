//! Pen palette layout, hit testing and painting for the shared sidebar.
use super::{button_surface, draw_fading_popup, panel, sidebar_pen_rect, sidebar_rect, Control, Ctx, PopupHit, R};
use crate::{app::App, text::Fonts, ui_icons::{self as icons, Glyph}, ui_theme::{self as theme, c}};

const HEIGHT: i32 = 228;
const COLOR_TOP: i32 = 36;
const COLOR_ROW: i32 = 48;

fn palette_y(p: R, offset: i32) -> i32 { p.y + offset * p.h / HEIGHT }

pub fn pen_popup_rect(app: &App) -> R {
    let w = 264.min(app.screen_w);
    let h = HEIGHT.min(app.screen_h);
    let sb = sidebar_rect(app, app.popup_on_right);
    let x = if app.popup_on_right {
        sb.x - w - 8
    } else {
        sb.x + sb.w + 8
    };
    // Open the first row of ink beside the pen, even when the drawer grows.
    let pen = sidebar_pen_rect(app, app.popup_on_right);
    let y = pen.y + app.opening_sidebar_shift() + pen.h / 2 - (COLOR_TOP + COLOR_ROW / 2) * h / HEIGHT;
    R {
        x: x.clamp(0, (app.screen_w - w).max(0)),
        y: y.clamp(0, (app.screen_h - h).max(0)),
        w,
        h,
    }
}

// Each whole cell is interactive, including the space around a swatch or a
// line sample. Use the same cells for painting, press feedback and hit testing.
fn cells(p: R, count: usize, columns: usize, top: i32, height: i32) -> impl Iterator<Item = (usize, R)> {
    let inset = 12.min(p.w / 10);
    let edge = move |column: usize| p.x + inset + (p.w - 2 * inset) * column as i32 / columns as i32;
    (0..count).map(move |i| {
        let column = i % columns;
        let row_y = top + (i / columns) as i32 * height;
        (i, R { x: edge(column), y: palette_y(p, row_y), w: edge(column + 1) - edge(column),
            h: palette_y(p, row_y + height) - palette_y(p, row_y) })
    })
}

fn color_cells(p: R) -> impl Iterator<Item = (usize, R)> {
    cells(p, crate::app::COLORS.len(), 5, COLOR_TOP, COLOR_ROW)
}

fn width_cells(p: R) -> impl Iterator<Item = (usize, R)> {
    let count = crate::app::PEN_SIZES.len();
    cells(p, count, count, 168, 48)
}

pub(super) fn pen_controls(p: R) -> impl Iterator<Item = (PopupHit, R)> {
    color_cells(p).map(|(i, r)| (PopupHit::Color(i), r))
        .chain(width_cells(p).map(|(i, r)| (PopupHit::PenSize(i), r)))
}

pub fn pen_popup_hit(app: &App, x: i32, y: i32) -> Option<PopupHit> {
    pen_controls(pen_popup_rect(app)).find(|(_, r)| r.contains(x, y)).map(|(hit, _)| hit)
}

pub(super) fn draw_pen_popup(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let p = pen_popup_rect(app);
    draw_fading_popup(app, ctx, p, |ctx| {
        const NAMES: [&str; crate::app::COLORS.len()] =
            ["红色", "蓝色", "绿色", "黄色", "白色", "青色", "紫色", "黄绿", "橙色", "黑色"];
        let palette = ctx.palette;
        let x = p.x as f32;
        let y = |offset| palette_y(p, offset) as f32;
        panel(ctx, p, 13.0, 1.0);
        ctx.text(fonts, "笔色", x + 16.0, y(27), 15.0, c(palette.text));
        let name = NAMES[app.cur_color];
        let end = x + p.w as f32 - 16.0;
        ctx.text(fonts, name, end - fonts.text_width(name, 13.0), y(27), 13.0, c(palette.muted));
        ctx.stroke_line((x + 16.0, y(141)), (end, y(141)), c(palette.border), 1.0);
        ctx.text(fonts, "笔宽", x + 16.0, y(161), 15.0, c(palette.text));
        for (i, cell) in color_cells(p) { draw_ink_swatch(app, ctx, cell, i); }
        for (i, cell) in width_cells(p) { draw_pen_width(ctx, fonts, cell, i, app); }
    });
}

fn draw_ink_swatch(app: &App, ctx: &mut Ctx, cell: R, index: usize) {
    let palette = ctx.palette;
    let selected = index == app.cur_color;
    let pressed = Control::Popup(PopupHit::Color(index)).pressed(app);
    let (r, g, b) = crate::app::COLORS[index];
    let rgb = u32::from_be_bytes([0, r, g, b]);
    let (cx, cy) = ((cell.x + cell.w / 2) as f32, (cell.y + cell.h / 2) as f32);
    let radius = 16.0f32.min(cell.w.min(cell.h) as f32 * 0.34);
    if selected {
        let outline = if pressed { theme::mix(palette.accent, palette.on_accent, 0.40) } else { palette.accent };
        ctx.fill_circle(cx, cy, radius + 5.0, c(outline));
        ctx.fill_circle(cx, cy, radius + 3.0, c(palette.paper));
    }
    let rim = if theme::contrast(rgb, palette.paper) < 3.0 { palette.muted } else { theme::mix(rgb, palette.text, 0.32) };
    let ink = if pressed { theme::on_color(c(rgb)) } else { c(rim) };
    ctx.fill_circle(cx, cy, radius + 1.0, ink);
    let inner = if pressed { (radius - 1.5).max(0.0) } else { radius };
    ctx.fill_circle(cx, cy, inner, c(rgb));
    if selected {
        let ink = theme::on_color(c(rgb));
        icons::draw(ctx, Glyph::Check, cx - 9.0, cy - 9.0, 18.0, ink, ink);
    }
}

fn draw_pen_width(ctx: &mut Ctx, fonts: &Fonts, cell: R, index: usize, app: &App) {
    let palette = ctx.palette;
    let selected = index == app.cur_pen;
    let background = if selected { palette.accent_light } else { palette.paper };
    let face = R { x: cell.x + 2, y: cell.y + 1, w: cell.w - 4, h: cell.h - 2 };
    let rim = if selected { palette.accent } else { palette.control_border() };
    button_surface(app, ctx, Control::Popup(PopupHit::PenSize(index)), face, 7.0, c(background), c(rim));
    let width = crate::app::PEN_SIZES[index];
    let y = (cell.y + cell.h / 3) as f32;
    let cx = (cell.x + cell.w / 2) as f32;
    let half = ((cell.w - 16).max(0) as f32 / 2.0).min(12.0);
    let (start, end) = ((cx - half, y), (cx + half, y));
    let (r, g, b) = crate::app::COLORS[app.cur_color];
    if theme::contrast(u32::from_be_bytes([0, r, g, b]), background) < 3.0 {
        ctx.stroke_line(start, end, c(palette.muted), width as f32 + 2.0);
    }
    ctx.stroke_line(start, end, app.pen_color(), width as f32);
    let label = R { y: cell.y + cell.h / 2, h: cell.h / 2 - 3, ..cell };
    ctx.text_center(fonts, label, &format!("{width} px"), 12.0, c(palette.text));
}
