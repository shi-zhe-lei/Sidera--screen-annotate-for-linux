use tiny_skia::{FilterQuality, IntSize, PixmapPaint};

use super::{board_palette, coord_bounds, draw_axis_ends, draw_coord_curve, draw_coord_foci,
    draw_coord_grid, App, Ctx, Fonts, Pixmap, Transform, R};
use crate::page_cache::{ink_preview, preview_size, reduced_ink};
use crate::page_picker::PagePicker;

fn live_ink(app: &App) -> Option<Pixmap> {
    if !app.page_has_ink && app.floating.is_none() { return None; }
    let mut ink = ink_preview(&app.canvas)?;
    if let Some(selection) = &app.floating {
        // The lasso has lifted these pixels off the canvas. Show their current
        // physical position without committing the selection or its undo step.
        let sx = ink.width() as f32 / app.canvas.width() as f32;
        let sy = ink.height() as f32 / app.canvas.height() as f32;
        let size = IntSize::from_wh((selection.pixmap.width() as f32 * sx).round().max(1.0) as u32,
                                    (selection.pixmap.height() as f32 * sy).round().max(1.0) as u32)?;
        let lifted = reduced_ink(&selection.pixmap, size)?;
        let transform = Transform::from_scale(sx, sy)
            .pre_translate(selection.x as f32, selection.y as f32)
            .pre_scale(selection.pixmap.width() as f32 / lifted.width() as f32,
                       selection.pixmap.height() as f32 / lifted.height() as f32);
        ink.draw_pixmap(0, 0, lifted.as_ref(), &filtered_paint(), transform, None);
    }
    Some(ink)
}

fn filtered_paint() -> PixmapPaint {
    PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() }
}

pub(super) fn render(app: &App, fonts: &Fonts, picker: &PagePicker, page: i32, bounds: R) -> Option<Pixmap> {
    if bounds.w <= 0 || bounds.h <= 0 { return None; }
    let (w, h) = (app.screen_w, app.screen_h);
    let size = preview_size(w as u32, h as u32,
        (f64::from(bounds.w) * app.scale).round() as u32,
        (f64::from(bounds.h) * app.scale).round() as u32);
    let mut board = Pixmap::new(size.width(), size.height())?;
    board.fill(app.whiteboard_bg_color());
    let tr = Transform::from_scale(size.width() as f32 / w as f32, size.height() as f32 / h as f32);
    let mut ctx = Ctx { pm: &mut board, tr, palette: board_palette(app) };
    let (guide, ink) = if page == app.current_slide {
        (app.coord.as_ref(), picker.live_ink.get_or_init(|| live_ink(app)).as_ref())
    } else {
        (app.coord_pages.get(&page), app.whiteboard_cache.preview(page))
    };
    let board_fonts = fonts.for_settings();
    if let Some(g) = guide {
        if let Some(extents) = coord_bounds(g, 0, 0, w, h) {
            draw_coord_grid(fonts, &mut ctx, g, extents, !g.editing);
            draw_axis_ends(&mut ctx, &board_fonts, g, w, h);
            draw_coord_curve(&mut ctx, g, extents);
        }
    }
    if let Some(ink) = ink {
        // Loading a saved page fits its ink to the current canvas, including
        // when an output has been resized since the page was saved.
        let fit = Transform::from_scale(size.width() as f32 / ink.width() as f32,
                                        size.height() as f32 / ink.height() as f32);
        ctx.pm.draw_pixmap(0, 0, ink.as_ref(), &filtered_paint(), fit, None);
    }
    if let Some(g) = guide { draw_coord_foci(g, &board_fonts, &mut ctx); }
    Some(board)
}
