//! Raster lasso selection for the whiteboard. The canvas stores pixels, not
//! vector strokes, so a selection lifts the pixels inside the closed lasso.
use tiny_skia::{BlendMode, Color, FillRule, PathBuilder, Pixmap, PixmapPaint, Shader, Transform};

use crate::app::App;

pub struct FloatingSelection {
    pub pixmap: Pixmap,
    /// Current top-left in physical canvas pixels.
    pub x: i32,
    pub y: i32,
    /// Original top-left, used when cancelling.
    pub origin_x: i32,
    pub origin_y: i32,
    /// Lasso outline in logical screen coordinates.
    pub outline: Vec<(f32, f32)>,
}

fn lasso_path(points: &[(f32, f32)]) -> Option<tiny_skia::Path> {
    if points.len() < 3 {
        return None;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(points[0].0, points[0].1);
    for &(x, y) in &points[1..] {
        pb.line_to(x, y);
    }
    pb.close();
    pb.finish()
}

/// Cut the ink inside `lasso` (logical coordinates) into a floating layer.
pub fn lift(app: &mut App, lasso: &[(f32, f32)]) -> Option<FloatingSelection> {
    let scale = app.scale as f32;
    if scale <= 0.0 || lasso.len() < 3 {
        return None;
    }
    let cw = app.canvas.width() as i32;
    let ch = app.canvas.height() as i32;
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let physical: Vec<(f32, f32)> = lasso.iter().map(|&(x, y)| {
        let (px, py) = (x * scale, y * scale);
        min_x = min_x.min(px); min_y = min_y.min(py);
        max_x = max_x.max(px); max_y = max_y.max(py);
        (px, py)
    }).collect();
    let x0 = (min_x.floor() as i32).clamp(0, cw);
    let y0 = (min_y.floor() as i32).clamp(0, ch);
    let x1 = (max_x.ceil() as i32 + 1).clamp(0, cw);
    let y1 = (max_y.ceil() as i32 + 1).clamp(0, ch);
    let bw = x1 - x0;
    let bh = y1 - y0;
    if bw <= 0 || bh <= 0 {
        return None;
    }
    let local: Vec<(f32, f32)> = physical.iter()
        .map(|&(x, y)| (x - x0 as f32, y - y0 as f32)).collect();
    let path = lasso_path(&local)?;
    let mut mask = Pixmap::new(bw as u32, bh as u32)?;
    mask.fill_path(&path, &tiny_skia::Paint {
        shader: Shader::SolidColor(Color::WHITE),
        anti_alias: true,
        ..Default::default()
    }, FillRule::Winding, Transform::identity(), None);

    let mut floating = Pixmap::new(bw as u32, bh as u32)?;
    {
        let src = app.canvas.data();
        let dst = floating.data_mut();
        let mask_data = mask.data();
        let cw = cw as usize;
        let mut any = false;
        for row in 0..bh as usize {
            for col in 0..bw as usize {
                let mi = (row * bw as usize + col) * 4;
                if mask_data[mi + 3] == 0 { continue; }
                let si = ((y0 as usize + row) * cw + x0 as usize + col) * 4;
                let di = (row * bw as usize + col) * 4;
                let a = src[si + 3];
                if a == 0 { continue; }
                any = true;
                dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
        if !any { return None; }
    }
    app.begin_undo();
    app.save_undo_rect(x0, y0, x1, y1);
    {
        let data = app.canvas.data_mut();
        let cw = cw as usize;
        let mask_data = mask.data();
        for row in 0..bh as usize {
            for col in 0..bw as usize {
                let mi = (row * bw as usize + col) * 4;
                if mask_data[mi + 3] == 0 { continue; }
                let si = ((y0 as usize + row) * cw + x0 as usize + col) * 4;
                data[si..si + 4].fill(0);
            }
        }
    }
    app.page_has_ink = true;
    Some(FloatingSelection {
        pixmap: floating,
        x: x0,
        y: y0,
        origin_x: x0,
        origin_y: y0,
        outline: lasso.to_vec(),
    })
}

/// Anchor movement in physical pixels, including at fractional display scales.
pub fn move_to_pointer(app: &mut App, offset: (i32, i32), x: i32, y: i32) {
    app.invalidate_board_preview();
    let Some(sel) = app.floating.as_mut() else { return };
    let scale = app.scale as f32;
    let max_x = app.canvas.width().saturating_sub(sel.pixmap.width()) as i32;
    let max_y = app.canvas.height().saturating_sub(sel.pixmap.height()) as i32;
    sel.x = ((x as f32 * scale).round() as i32 - offset.0).clamp(0, max_x);
    sel.y = ((y as f32 * scale).round() as i32 - offset.1).clamp(0, max_y);
}

pub fn commit(app: &mut App) {
    let Some(sel) = app.floating.take() else { return };
    let cw = app.canvas.width() as i32;
    let ch = app.canvas.height() as i32;
    let x1 = (sel.x + sel.pixmap.width() as i32).min(cw);
    let y1 = (sel.y + sel.pixmap.height() as i32).min(ch);
    if (sel.x, sel.y) == (sel.origin_x, sel.origin_y) {
        app.discard_last_undo();
    } else {
        app.save_undo_rect(sel.x.max(0), sel.y.max(0), x1, y1);
    }
    app.canvas.draw_pixmap(sel.x, sel.y, sel.pixmap.as_ref(),
        &PixmapPaint { blend_mode: BlendMode::SourceOver, ..Default::default() },
        Transform::identity(), None);
    app.page_has_ink = true;
}

pub fn cancel(app: &mut App) {
    let Some(sel) = app.floating.take() else { return };
    app.canvas.draw_pixmap(sel.origin_x, sel.origin_y, sel.pixmap.as_ref(),
        &PixmapPaint { blend_mode: BlendMode::SourceOver, ..Default::default() },
        Transform::identity(), None);
    app.discard_last_undo();
    app.page_has_ink = app.canvas.pixels().iter().any(|p| p.alpha() != 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::Pixmap;

    #[test]
    fn lasso_lifts_moves_and_commits_ink() {
        let mut app = App::new(Pixmap::new(200, 120).unwrap(), 200, 120);
        app.whiteboard = true;
        {
            let data = app.canvas.data_mut();
            let i = (50 * 200 + 50) * 4;
            data[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
        }
        app.page_has_ink = true;
        let lasso = [(40.0, 40.0), (70.0, 40.0), (70.0, 70.0), (40.0, 70.0)];
        let mut selection = lift(&mut app, &lasso).expect("selection");
        assert_eq!(app.canvas.data()[(50 * 200 + 50) * 4 + 3], 0);
        selection.x += 20;
        selection.y += 10;
        app.floating = Some(selection);
        commit(&mut app);
        assert_eq!(app.canvas.data()[(60 * 200 + 70) * 4 + 3], 255);
    }
}
