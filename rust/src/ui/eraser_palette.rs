//! Eraser widths and the page-clear action share one touch layout on both rails.
use super::{button_surface, draw_fading_popup, panel, sidebar_rect, tool_segments, Control, Ctx, PopupHit, R};
use crate::{app::{App, ERASER_SIZES}, text::Fonts, ui_icons::{self as icons, Glyph}, ui_theme::c};

const HEADER: i32 = 38;
const SIZE_ROW: i32 = 68;

fn columns(width: i32) -> usize {
    if width < 216 { 2 } else { ERASER_SIZES.len() }
}

fn content_height(width: i32) -> i32 {
    HEADER + ERASER_SIZES.len().div_ceil(columns(width)) as i32 * SIZE_ROW + 16 + 48 + 12
}

pub fn eraser_popup_rect(app: &App) -> R {
    let w = 264.min(app.screen_w);
    let height = content_height(w);
    let h = height.min(app.screen_h);
    let side = app.popup_on_right;
    let rail = sidebar_rect(app, side);
    let eraser = tool_segments(app, side)[2].1;
    let x = if side { rail.x - w - 8 } else { rail.x + rail.w + 8 };
    let y = eraser.y + app.opening_sidebar_shift() + eraser.h / 2 - (HEADER + SIZE_ROW / 2) * h / height;
    R { x: x.clamp(0, app.screen_w - w), y: y.clamp(0, app.screen_h - h), w, h }
}

pub(super) fn eraser_controls(p: R) -> impl Iterator<Item = (PopupHit, R)> {
    let columns = columns(p.w);
    let height = content_height(p.w);
    let inset = 12.min(p.w / 10);
    let x = move |col: usize| p.x + inset + (p.w - 2 * inset) * col as i32 / columns as i32;
    let y = move |offset: i32| p.y + offset * p.h / height;
    let sizes = (0..ERASER_SIZES.len()).map(move |i| {
        let col = i % columns;
        let top = HEADER + (i / columns) as i32 * SIZE_ROW;
        (PopupHit::EraserSize(i), R { x: x(col), y: y(top), w: x(col + 1) - x(col), h: y(top + SIZE_ROW) - y(top) })
    });
    let clear = R { x: x(0), y: y(height - 60), w: x(columns) - x(0), h: y(height - 12) - y(height - 60) };
    sizes.chain(std::iter::once((PopupHit::ClearAll, clear)))
}

pub fn eraser_popup_hit(app: &App, x: i32, y: i32) -> Option<PopupHit> {
    eraser_controls(eraser_popup_rect(app)).find(|(_, r)| r.contains(x, y)).map(|(hit, _)| hit)
}

pub(super) fn draw_eraser_popup(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let p = eraser_popup_rect(app);
    draw_fading_popup(app, ctx, p, |ctx| {
        let palette = ctx.palette;
        panel(ctx, p, 13.0, 1.0);
        let baseline = (p.y + 27 * p.h / content_height(p.w)) as f32;
        ctx.text(fonts, "橡皮大小", (p.x + 16) as f32, baseline, 15.0, c(palette.text));
        let current = format!("{} px", app.eraser_width());
        ctx.text(fonts, &current, (p.x + p.w - 16) as f32 - fonts.text_width(&current, 13.0),
            baseline, 13.0, c(palette.muted));
        for (hit, cell) in eraser_controls(p) {
            match hit {
                PopupHit::EraserSize(i) => {
                    let selected = i == app.cur_eraser;
                    let face = R { x: cell.x + 2, y: cell.y + 2, w: cell.w - 4, h: cell.h - 4 };
                    let (fill, border) = if selected { (palette.accent_light, palette.accent) }
                        else { (palette.surface, palette.control_border()) };
                    let background = button_surface(app, ctx, Control::Popup(hit), face, 9.0, c(fill), c(border));
                    let cx = (cell.x + cell.w / 2) as f32;
                    let cy = (cell.y + cell.h * 2 / 5) as f32;
                    let radius = (ERASER_SIZES[i] as f32 * 0.165).min(cell.h as f32 * 0.22);
                    // A filled sample identifies the chosen width without relying on hue.
                    ctx.fill_circle(cx, cy, radius, c(if selected { palette.text } else { palette.muted }));
                    if !selected && radius > 1.5 { ctx.fill_circle(cx, cy, radius - 1.5, background); }
                    let label = R { y: cell.y + cell.h * 3 / 5, h: cell.h / 3, ..cell };
                    ctx.text_center(fonts, label, &ERASER_SIZES[i].to_string(), 14.0, c(palette.text));
                }
                PopupHit::ClearAll => {
                    let y = (cell.y - 8) as f32;
                    ctx.stroke_line(((p.x + 16) as f32, y), ((p.x + p.w - 16) as f32, y), c(palette.border), 1.0);
                    button_surface(app, ctx, Control::Popup(hit), cell, 9.0, c(palette.paper), c(palette.control_border()));
                    let title = "清空当前页";
                    let x = cell.x as f32 + (cell.w as f32 - 32.0 - fonts.text_width(title, 15.0)) / 2.0;
                    let cy = cell.y as f32 + cell.h as f32 / 2.0;
                    icons::draw(ctx, Glyph::Trash, x, cy - 12.5, 25.0, c(palette.danger), c(palette.danger));
                    ctx.text(fonts, title, x + 32.0, cy + 5.0, 15.0, c(palette.text));
                }
                _ => unreachable!(),
            }
        }
    });
}
