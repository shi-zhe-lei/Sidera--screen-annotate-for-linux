// ============================================================
// Sidera - 笔迹绘制（画笔/橡皮，纯 tiny-skia 光栅）
// 与原 C++ drawing.cpp 语义一致：画笔 SourceOver、橡皮 Clear
// ============================================================
use tiny_skia::{BlendMode, Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Shader, Stroke, Transform};

use crate::app::App;

fn solid(color: Color, blend: BlendMode) -> Paint<'static> {
    Paint {
        shader: Shader::SolidColor(color),
        anti_alias: true,
        blend_mode: blend,
        ..Default::default()
    }
}

pub fn pen_segment(canvas: &mut Pixmap, color: Color, a: (i32, i32), b: (i32, i32), width: i32, tf: Transform) {
    let paint = solid(color, BlendMode::SourceOver);
    if a == b {
        let mut pb = PathBuilder::new();
        pb.push_circle(a.0 as f32, a.1 as f32, width as f32 / 2.0);
        if let Some(p) = pb.finish() {
            canvas.fill_path(&p, &paint, FillRule::Winding, tf, None);
        }
    } else {
        let stroke = Stroke {
            width: width as f32,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Default::default()
        };
        let mut pb = PathBuilder::new();
        pb.move_to(a.0 as f32, a.1 as f32);
        pb.line_to(b.0 as f32, b.1 as f32);
        if let Some(p) = pb.finish() {
            canvas.stroke_path(&p, &paint, &stroke, tf, None);
        }
    }
}

pub fn erase_segment(canvas: &mut Pixmap, a: (i32, i32), b: (i32, i32), width: i32, tf: Transform) {
    let paint = solid(Color::TRANSPARENT, BlendMode::Clear);
    if a == b {
        let mut pb = PathBuilder::new();
        pb.push_circle(a.0 as f32, a.1 as f32, width as f32 / 2.0);
        if let Some(p) = pb.finish() {
            canvas.fill_path(&p, &paint, FillRule::Winding, tf, None);
        }
    } else {
        let stroke = Stroke {
            width: width as f32,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Default::default()
        };
        let mut pb = PathBuilder::new();
        pb.move_to(a.0 as f32, a.1 as f32);
        pb.line_to(b.0 as f32, b.1 as f32);
        if let Some(p) = pb.finish() {
            canvas.stroke_path(&p, &paint, &stroke, tf, None);
        }
    }
}

pub fn pen_tapered(canvas: &mut Pixmap, color: Color, a: (i32, i32), b: (i32, i32), w0: i32, w1: i32, tf: Transform) {
    if a == b {
        pen_segment(canvas, color, a, b, w0.max(1), tf);
        return;
    }
    let pts = [
        (a.0 as f32, a.1 as f32, w0 as f32),
        (b.0 as f32, b.1 as f32, w1 as f32),
    ];
    draw_stroke_union(canvas, color, &pts, tf);
}

/// 把一条笔画（采样点 + 每点宽度）构建为**单个并集路径**：
/// 每个采样点的圆（圆角接头/端帽）+ 相邻点之间的梯形。
/// 一次填充，避免多段分别绘制时抗锯齿边缘互相叠加 → 毛边。
pub fn build_stroke_path(pts: &[(f32, f32, f32)]) -> Option<tiny_skia::Path> {
    if pts.is_empty() {
        return None;
    }
    let mut pb = PathBuilder::new();
    let mut any = false;
    if pts.len() == 1 {
        let (x, y, w) = pts[0];
        pb.push_circle(x, y, (w * 0.5).max(0.5));
        any = true;
    } else {
        for i in 0..pts.len() - 1 {
            let (x0, y0, w0) = pts[i];
            let (x1, y1, w1) = pts[i + 1];
            let dx = x1 - x0;
            let dy = y1 - y0;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 0.01 {
                continue;
            }
            let nx = -dy / len;
            let ny = dx / len;
            let h0 = (w0 * 0.5).max(0.5);
            let h1 = (w1 * 0.5).max(0.5);
            // 梯形顶点顺序必须与 push_circle 的绕向一致（顺时针），
            // 否则 Winding 填充时圆与梯形重叠处会互相抵消，弯折处出现断口。
            pb.move_to(x0 - nx * h0, y0 - ny * h0);
            pb.line_to(x1 - nx * h1, y1 - ny * h1);
            pb.line_to(x1 + nx * h1, y1 + ny * h1);
            pb.line_to(x0 + nx * h0, y0 + ny * h0);
            pb.close();
            any = true;
        }
        for &(x, y, w) in pts {
            pb.push_circle(x, y, (w * 0.5).max(0.5));
            any = true;
        }
    }
    if !any {
        return None;
    }
    pb.finish()
}

pub fn draw_stroke_union(
    pm: &mut Pixmap,
    color: Color,
    pts: &[(f32, f32, f32)],
    transform: Transform,
) {
    if let Some(p) = build_stroke_path(pts) {
        pm.fill_path(
            &p,
            &solid(color, BlendMode::SourceOver),
            FillRule::Winding,
            transform,
            None,
        );
    }
}

/// 提交当前进行中的画笔笔画到画布（单次填充 → 边缘干净）
pub fn commit_stroke(app: &mut App) {
    if app.stroke_pts.is_empty() {
        return;
    }
    let c = app.pen_color();
    let tf = Transform::from_scale(app.scale as f32, app.scale as f32);
    if let Some(p) = build_stroke_path(&app.stroke_pts) {
        app.invalidate_board_preview();
        app.canvas.fill_path(&p, &solid(c, BlendMode::SourceOver), FillRule::Winding, tf, None);
        app.page_has_ink = true;
    }
    app.stroke_pts.clear();
}

// ---- 带全局状态的操作 ----
fn scale_tf(app: &App) -> Transform {
    Transform::from_scale(app.scale as f32, app.scale as f32)
}

fn save_stroke_undo(app: &mut App, a: (i32, i32), b: (i32, i32), width: i32) {
    let radius = f64::from(width.max(1)) / 2.0;
    // 与绘图 transform 使用相同缩放；转换后向外取整，额外保留 1 个物理像素的抗锯齿边缘。
    let scale = f64::from(app.scale as f32);
    app.save_undo_rect(
        ((f64::from(a.0.min(b.0)) - radius) * scale - 1.0).floor() as i32,
        ((f64::from(a.1.min(b.1)) - radius) * scale - 1.0).floor() as i32,
        ((f64::from(a.0.max(b.0)) + radius) * scale + 1.0).ceil() as i32,
        ((f64::from(a.1.max(b.1)) + radius) * scale + 1.0).ceil() as i32,
    );
}

pub fn stroke_segment(app: &mut App, a: (i32, i32), b: (i32, i32), erase: bool, width: i32) {
    save_stroke_undo(app, a, b, width);
    let tf = scale_tf(app);
    if erase {
        erase_segment(&mut app.canvas, a, b, width, tf);
    } else {
        app.page_has_ink = true;
        let c = app.pen_color();
        pen_segment(&mut app.canvas, c, a, b, width, tf);
    }
}

pub fn stroke_tapered(app: &mut App, a: (i32, i32), b: (i32, i32), w0: i32, w1: i32) {
    save_stroke_undo(app, a, b, w0.max(w1));
    app.page_has_ink = true;
    let c = app.pen_color();
    let tf = scale_tf(app);
    pen_tapered(&mut app.canvas, c, a, b, w0, w1, tf);
}

#[cfg(test)]
mod undo_tests {
    use super::*;

    fn patterned_app(scale: f64) -> App {
        let mut app = App::new(Pixmap::new(641, 479).unwrap(), 641, 479);
        app.set_scale(scale);
        // 非均匀底图用于检查撤回是否逐像素恢复，而非只把画布清空。
        let width = app.canvas.width() as usize;
        for (i, pixel) in app.canvas.data_mut().chunks_exact_mut(4).enumerate() {
            let x = i % width;
            let y = i / width;
            pixel.copy_from_slice(&[(x % 251) as u8, (y % 239) as u8, 127, 255]);
        }
        app.page_has_ink = true;
        app
    }

    fn assert_restored(app: &mut App, before: &[u8], context: &str) {
        assert!(app.canvas.data() != before, "操作必须实际改变像素: {context}");
        app.undo_last();
        assert!(app.canvas.data() == before, "撤回必须恢复全部像素: {context}");
    }

    #[test]
    fn scaled_tapered_pen_undo_restores_pixels() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for (a, b, w0, w1) in [
                ((207, 157), (207, 157), 3, 3),
                ((510, 350), (630, 470), 5, 19),
                ((-5, 2), (4, 100), 5, 17),
                ((635, 477), (647, 489), 3, 11),
                ((430, 300), (175, 185), 19, 3),
            ] {
                let mut app = patterned_app(scale);
                let before = app.canvas.data().to_vec();
                app.begin_undo();
                stroke_tapered(&mut app, a, b, w0, w1);
                assert_restored(&mut app, &before, &format!("scale={scale}, {a:?}->{b:?}"));
            }
        }
    }

    #[test]
    fn scaled_pen_and_eraser_segments_undo_restore_pixels() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for erase in [false, true] {
                for (a, b, width) in [
                    ((205, 205), (205, 205), 3),
                    ((205, 205), (420, 350), 7),
                    ((610, 450), (638, 476), 64),
                    ((-12, -12), (150, 90), 128),
                ] {
                    let mut app = patterned_app(scale);
                    let before = app.canvas.data().to_vec();
                    app.begin_undo();
                    stroke_segment(&mut app, a, b, erase, width);
                    assert_restored(
                        &mut app, &before,
                        &format!("scale={scale}, erase={erase}, width={width}, {a:?}->{b:?}"),
                    );
                }
            }
        }
    }

    #[test]
    fn scaled_overlapping_curve_undo_keeps_original_tile_contents() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let mut app = patterned_app(scale);
            let before = app.canvas.data().to_vec();
            app.begin_undo();
            let mut prev = (210, 220);
            for i in 1..=48 {
                let angle = i as f64 * std::f64::consts::TAU / 24.0;
                let next = (
                    310 + (angle.cos() * 100.0) as i32,
                    220 + (angle.sin() * 90.0) as i32,
                );
                stroke_tapered(&mut app, prev, next, 3 + i % 11, 5 + i % 17);
                prev = next;
            }
            assert_restored(&mut app, &before, &format!("重叠曲线 scale={scale}"));
        }
    }

    #[test]
    fn scaled_pen_and_eraser_undo_in_reverse_order() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let mut app = patterned_app(scale);
            let before_pen = app.canvas.data().to_vec();
            app.begin_undo();
            stroke_tapered(&mut app, (190, 210), (420, 350), 3, 19);
            let before_eraser = app.canvas.data().to_vec();
            assert!(before_eraser != before_pen);
            app.begin_undo();
            stroke_segment(&mut app, (220, 220), (400, 330), true, 64);

            assert_restored(&mut app, &before_eraser, &format!("撤回橡皮 scale={scale}"));
            assert_restored(&mut app, &before_pen, &format!("撤回画笔 scale={scale}"));
            assert!(app.undo.is_empty());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stroke_union_is_continuous() {
        // 模拟一串鼠标采样点（水平），整条笔画的中心线应处处不透明
        let pts: Vec<(f32, f32, f32)> = (0..40)
            .map(|i| (20.0 + i as f32 * 4.0, 50.0, 8.0))
            .collect();
        let mut pm = Pixmap::new(200, 100).unwrap();
        draw_stroke_union(&mut pm, crate::app::color(0, 0, 0, 255), &pts, Transform::identity());
        let w = pm.width() as usize;
        let mut holes = Vec::new();
        for i in 0..40 {
            let x = (20.0 + i as f32 * 4.0) as usize;
            let a = pm.data()[(50 * w + x) * 4 + 3];
            if a < 200 {
                holes.push((x, a));
            }
        }
        assert!(holes.is_empty(), "笔画在采样点处有空洞: {:?}", holes);
    }
}

#[cfg(test)]
mod tests2 {
    use super::*;

    #[test]
    fn curved_stroke_continuous() {
        let mut pts: Vec<(f32, f32, f32)> = Vec::new();
        for i in 0..220 {
            let t = i as f32 / 219.0;
            let x = 100.0 + t * 600.0;
            let y = 60.0 + (t * 12.0).sin() * 20.0;
            pts.push((x, y, 10.0));
        }
        let mut pm = Pixmap::new(760, 120).unwrap();
        draw_stroke_union(&mut pm, crate::app::color(0, 0, 0, 255), &pts, Transform::identity());
        let w = pm.width() as usize;
        let mut holes = Vec::new();
        for (i, (x, y, _)) in pts.iter().enumerate() {
            let a = pm.data()[((*y as usize) * w + (*x as usize)) * 4 + 3];
            if a < 200 {
                holes.push((i, *x as i32, *y as i32, a));
            }
        }
        eprintln!("holes={}/{} first={:?}", holes.len(), pts.len(), holes.first());
        assert!(holes.is_empty(), "曲线笔迹有空洞");
    }
}
