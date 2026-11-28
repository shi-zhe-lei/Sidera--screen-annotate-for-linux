//! Classroom tools drawn on a shared 24-unit grid, independent of system fonts.
use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Shader, Stroke};
use crate::ui::Ctx;
use crate::ui_theme::c;

/// Shared, version-matched application artwork for the startup and About panels.
/// Embed the shipped PNG so an old system icon cannot override a new binary.
pub fn app_icon(size: u32) -> Option<tiny_skia::Pixmap> {
    if size == 0 { return None; }
    let override_image = std::env::var_os("SIDERA_ICON").and_then(|path| image::open(path).ok());
    let image = override_image.or_else(|| image::load_from_memory(include_bytes!("../../sidera.png")).ok())?;
    let rgba = image::imageops::resize(&image.to_rgba8(), size, size, image::imageops::FilterType::Lanczos3);
    let mut pixels = rgba.into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] { *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8; }
    }
    tiny_skia::Pixmap::from_vec(pixels, tiny_skia::IntSize::from_wh(size, size)?)
}

#[derive(Clone, Copy)]
pub enum Glyph {
    Cursor, Pen, Eraser, Up, Down, More, Screenshot, CollapseLeft, CollapseRight,
    Trash, Undo, Restore, Board, CloseBoard, BoardColor, StartShow, EndShow,
    Settings, Flask, Check, Info, Warning, Document, Left, Right, Reset, Exit,
}

fn paint(ink: Color) -> Paint<'static> {
    Paint { shader: Shader::SolidColor(ink), anti_alias: true, ..Default::default() }
}

fn path(ctx: &mut Ctx, p: Option<Path>, fill: Option<Color>, outline: Option<Color>) {
    let Some(p) = p else { return };
    if let Some(fill) = fill {
        ctx.pm.fill_path(&p, &paint(fill), FillRule::Winding, ctx.tr, None);
    }
    if let Some(outline) = outline {
        ctx.pm.stroke_path(&p, &paint(outline), &Stroke {
            width: 1.8, line_cap: LineCap::Round, line_join: LineJoin::Round,
            ..Default::default()
        }, ctx.tr, None);
    }
}

fn polygon(points: &[(f32, f32)]) -> Option<Path> {
    let mut p = PathBuilder::new();
    p.move_to(points[0].0, points[0].1);
    for &(x, y) in &points[1..] { p.line_to(x, y); }
    p.close();
    p.finish()
}

pub fn draw(ctx: &mut Ctx, glyph: Glyph, x: f32, y: f32, size: f32, ink: Color, accent: Color) {
    let palette = ctx.palette;
    let tr = ctx.tr.pre_translate(x, y).pre_scale(size / 24.0, size / 24.0);
    let mut g = Ctx { palette, pm: &mut *ctx.pm, tr };
    let line = |g: &mut Ctx, a, b| g.stroke_line(a, b, ink, 1.8);
    match glyph {
        Glyph::Cursor => {
            path(&mut g, polygon(&[(5.0, 3.0), (19.0, 12.0), (12.5, 13.2),
                (16.0, 19.3), (12.7, 21.0), (9.4, 14.8), (5.0, 19.0)]),
                Some(c(palette.paper)), Some(ink));
        }
        Glyph::Pen => {
            let mut p = PathBuilder::new();
            p.move_to(3.8, 20.2);
            p.line_to(5.3, 14.6);
            p.line_to(15.9, 4.0);
            p.quad_to(17.6, 2.3, 19.3, 4.0);
            p.line_to(20.0, 4.7);
            p.quad_to(21.7, 6.4, 20.0, 8.1);
            p.line_to(9.4, 18.7);
            p.close();
            let outline = p.finish();
            path(&mut g, outline.clone(), Some(c(palette.paper)), None);
            path(&mut g, polygon(&[(14.0, 6.0), (16.7, 3.5), (18.3, 3.5),
                (20.5, 5.7), (20.5, 7.3), (18.0, 10.0)]), Some(c(palette.accent_light)), None);
            // Keep enough coloured area at normal toolbar size; an outlined
            // triangle alone collapses to a dark point at low pixel density.
            path(&mut g, polygon(&[(3.8, 20.2), (5.3, 14.6), (7.5, 12.4),
                (11.6, 16.5), (9.4, 18.7)]),
                Some(accent), None);
            path(&mut g, outline, None, Some(ink));
            g.stroke_line((7.5, 12.4), (11.6, 16.5), ink, 1.3);
            line(&mut g, (14.0, 6.0), (18.0, 10.0));
            g.stroke_line((10.3, 12.1), (13.4, 9.0), ink, 1.1);
        }
        Glyph::Eraser => {
            let mut p = PathBuilder::new();
            p.move_to(4.2, 12.2);
            p.line_to(12.0, 4.4);
            p.quad_to(13.6, 2.8, 15.2, 4.4);
            p.line_to(20.6, 9.8);
            p.quad_to(22.2, 11.4, 20.6, 13.0);
            p.line_to(13.6, 20.0);
            p.line_to(8.0, 20.0);
            p.line_to(4.2, 16.2);
            p.quad_to(2.6, 14.2, 4.2, 12.2);
            p.close();
            let outline = p.finish();
            path(&mut g, outline.clone(), Some(c(palette.eraser)), None);
            let mut p = PathBuilder::new();
            p.move_to(7.5, 8.9); p.line_to(17.1, 16.5); p.line_to(13.6, 20.0);
            p.line_to(8.0, 20.0); p.line_to(4.2, 16.2);
            p.quad_to(2.6, 14.2, 4.2, 12.2); p.close();
            path(&mut g, p.finish(), Some(c(palette.paper)), None);
            path(&mut g, outline, None, Some(ink));
            line(&mut g, (7.5, 8.9), (17.1, 16.5));
            line(&mut g, (17.0, 20.0), (22.0, 20.0));
        }
        Glyph::Up | Glyph::Down => {
            let (outer, center) = if matches!(glyph, Glyph::Up) { (14.5, 8.5) } else { (9.5, 15.5) };
            line(&mut g, (6.0, outer), (12.0, center));
            line(&mut g, (12.0, center), (18.0, outer));
        }
        Glyph::More => {
            for x in [5.0, 12.0, 19.0] { g.fill_circle(x, 12.0, 1.65, ink); }
        }
        Glyph::Screenshot => {
            let mut p = PathBuilder::new();
            p.move_to(5.0, 7.0); p.line_to(7.0, 7.0); p.line_to(9.0, 4.5);
            p.line_to(15.0, 4.5); p.line_to(17.0, 7.0); p.line_to(19.0, 7.0);
            p.quad_to(21.0, 7.0, 21.0, 9.0); p.line_to(21.0, 18.0);
            p.quad_to(21.0, 20.0, 19.0, 20.0); p.line_to(5.0, 20.0);
            p.quad_to(3.0, 20.0, 3.0, 18.0); p.line_to(3.0, 9.0);
            p.quad_to(3.0, 7.0, 5.0, 7.0); p.close();
            path(&mut g, p.finish(), None, Some(ink));
            let mut p = PathBuilder::new(); p.push_circle(12.0, 13.0, 3.5);
            path(&mut g, p.finish(), None, Some(ink));
        }
        Glyph::CollapseLeft | Glyph::CollapseRight => {
            let (edge, tip, tail) = if matches!(glyph, Glyph::CollapseLeft) {
                (4.0, 9.0, 15.0)
            } else { (20.0, 15.0, 9.0) };
            line(&mut g, (edge, 4.0), (edge, 20.0));
            g.stroke_polyline(&[(tail, 6.0), (tip, 12.0), (tail, 18.0)], ink, 2.0, false);
        }
        Glyph::Trash => {
            let mut p = PathBuilder::new();
            p.move_to(3.5, 6.5); p.line_to(20.5, 6.5);
            p.move_to(8.5, 6.5); p.line_to(9.2, 3.9);
            p.quad_to(9.4, 3.0, 10.3, 3.0); p.line_to(13.7, 3.0);
            p.quad_to(14.6, 3.0, 14.8, 3.9); p.line_to(15.5, 6.5);
            p.move_to(5.5, 6.5); p.line_to(6.5, 19.4);
            p.quad_to(6.6, 21.0, 8.3, 21.0); p.line_to(15.7, 21.0);
            p.quad_to(17.4, 21.0, 17.5, 19.4); p.line_to(18.5, 6.5);
            p.move_to(10.0, 10.5); p.line_to(10.0, 17.0);
            p.move_to(14.0, 10.5); p.line_to(14.0, 17.0);
            path(&mut g, p.finish(), None, Some(ink));
        }
        Glyph::Undo => {
            let mut p = PathBuilder::new();
            p.move_to(7.0, 8.4); p.line_to(14.1, 8.4);
            p.cubic_to(22.4, 8.4, 22.4, 19.0, 14.1, 19.0); p.line_to(9.0, 19.0);
            path(&mut g, p.finish(), None, Some(ink));
            path(&mut g, polygon(&[(2.0, 8.4), (9.0, 2.7), (9.0, 14.1)]), Some(ink), None);
        }
        Glyph::Restore => {
            let mut p = PathBuilder::new();
            p.move_to(19.8, 16.0);
            p.cubic_to(18.3, 19.2, 15.4, 21.0, 12.0, 21.0);
            p.cubic_to(7.0, 21.0, 3.0, 17.0, 3.0, 12.0);
            p.cubic_to(3.0, 7.0, 7.0, 3.0, 12.0, 3.0);
            p.cubic_to(15.8, 3.0, 18.7, 5.1, 20.5, 9.5);
            p.move_to(20.5, 3.8); p.line_to(20.5, 9.5); p.line_to(14.8, 9.5);
            path(&mut g, p.finish(), None, Some(ink));
        }
        Glyph::Board => {
            g.stroke_round(crate::ui::R { x: 3, y: 4, w: 18, h: 14 }, 1.8, ink, 1.8);
            g.stroke_polyline(&[(6.5, 12.7), (10.0, 8.7), (11.0, 11.4), (14.4, 8.4)], ink, 1.6, false);
            g.stroke_polyline(&[(13.5, 15.6), (17.5, 15.6)], ink, 1.8, false);
            line(&mut g, (1.8, 20.5), (22.2, 20.5));
        }
        Glyph::CloseBoard => {
            g.stroke_line((4.5, 4.5), (19.5, 19.5), ink, 2.6);
            g.stroke_line((19.5, 4.5), (4.5, 19.5), ink, 2.6);
        }
        Glyph::BoardColor => {
            let mut p = PathBuilder::new();
            p.push_circle(12.0, 12.0, 8.0);
            path(&mut g, p.finish(), None, Some(ink));
            let mut half = PathBuilder::new();
            half.move_to(17.65, 6.35);
            half.cubic_to(20.8, 9.5, 20.8, 14.5, 17.65, 17.65);
            half.cubic_to(14.5, 20.8, 9.5, 20.8, 6.35, 17.65);
            half.close();
            path(&mut g, half.finish(), Some(ink), None);
        }
        Glyph::StartShow => {
            path(&mut g, polygon(&[(7.0, 4.0), (20.0, 12.0), (7.0, 20.0)]), Some(ink), None);
        }
        Glyph::EndShow => {
            g.fill_round(crate::ui::R { x: 5, y: 5, w: 14, h: 14 }, 2.0, ink);
        }
        Glyph::Settings => {
            for (y, x) in [(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)] {
                line(&mut g, (3.0, y), (21.0, y));
                let mut p = PathBuilder::new(); p.push_circle(x, y, 2.2);
                path(&mut g, p.finish(), Some(c(palette.paper)), Some(ink));
            }
        }
        Glyph::Flask => {
            line(&mut g, (8.5, 3.0), (15.5, 3.0));
            g.stroke_polyline(&[(10.0,3.0),(10.0,9.0),(4.4,18.5),(5.3,20.5),
                (18.7,20.5),(19.6,18.5),(14.0,9.0),(14.0,3.0)],ink,1.8,false);
            line(&mut g, (7.5, 14.0), (16.5, 14.0));
        }
        Glyph::Check => {
            line(&mut g, (5.0, 12.0), (10.0, 17.0));
            line(&mut g, (10.0, 17.0), (19.0, 7.0));
        }
        Glyph::Info => {
            let mut p = PathBuilder::new(); p.push_circle(12.0, 12.0, 9.0);
            path(&mut g, p.finish(), None, Some(ink));
            g.fill_circle(12.0, 7.5, 1.1, ink);
            line(&mut g, (12.0, 11.0), (12.0, 17.0));
        }
        Glyph::Warning => {
            let mut p = PathBuilder::new();
            p.move_to(10.3, 4.5); p.quad_to(12.0, 1.8, 13.7, 4.5);
            p.line_to(21.0, 17.5); p.quad_to(22.5, 20.0, 19.5, 20.0);
            p.line_to(4.5, 20.0); p.quad_to(1.5, 20.0, 3.0, 17.5); p.close();
            path(&mut g, p.finish(), None, Some(ink));
            line(&mut g, (12.0, 8.5), (12.0, 12.5));
            g.fill_circle(12.0, 16.0, 1.0, ink);
        }
        Glyph::Document => {
            path(&mut g, polygon(&[(5.0, 3.0), (14.0, 3.0), (19.0, 8.0), (19.0, 21.0), (5.0, 21.0)]),
                None, Some(ink));
            line(&mut g, (14.0, 3.0), (14.0, 8.0));
            line(&mut g, (14.0, 8.0), (19.0, 8.0));
            line(&mut g, (9.0, 12.0), (15.0, 12.0));
            line(&mut g, (9.0, 16.0), (15.0, 16.0));
        }
        Glyph::Left | Glyph::Right => {
            let (outer,center)=if matches!(glyph,Glyph::Left) {(15.0,9.0)}else{(9.0,15.0)};
            line(&mut g, (outer, 6.0), (center, 12.0));
            line(&mut g, (center, 12.0), (outer, 18.0));
        }
        Glyph::Reset => {
            let mut p=PathBuilder::new();
            p.move_to(5.0,9.0);
            p.cubic_to(7.0,3.5,15.0,3.0,18.0,8.0);
            p.cubic_to(22.0,15.0,14.0,22.0,8.0,17.0);
            path(&mut g,p.finish(),None,Some(ink));
            line(&mut g,(4.0,4.0),(4.0,10.0));
            line(&mut g,(4.0,10.0),(10.0,10.0));
        }
        Glyph::Exit => {
            let mut p=PathBuilder::new();
            p.move_to(11.0,4.0);p.line_to(5.0,4.0);p.quad_to(3.0,4.0,3.0,6.0);
            p.line_to(3.0,18.0);p.quad_to(3.0,20.0,5.0,20.0);p.line_to(11.0,20.0);
            path(&mut g,p.finish(),None,Some(ink));
            line(&mut g,(9.0,12.0),(21.0,12.0));
            line(&mut g,(17.0,8.0),(21.0,12.0));
            line(&mut g,(21.0,12.0),(17.0,16.0));
        }
    }
}
