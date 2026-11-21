use std::{collections::{hash_map::Entry, HashMap}, sync::Arc};

use ab_glyph::{Font as _, FontArc};
use ab_glyph_rasterizer::{point, Rasterizer};
use skrifa::{font::Blob, DrawMemory, Font, GlyphId, HintOptions, PathSink, RasterTarget, ScalerFont};
use tiny_skia::{Color, Mask, Path, PathBuilder, PathSegment, Pixmap, PixmapPaint, Point, Transform};

// Share the original face bytes with the hinter, including when the face is
// loaded from a large CJK collection. Both readers open collection index zero.
struct FaceBytes(FontArc);

impl AsRef<[u8]> for FaceBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.font_data()
    }
}

struct DevicePath(PathBuilder);

// Font coordinates point upwards from the baseline, pixmap coordinates down.
impl PathSink for DevicePath {
    fn move_to(&mut self, x: f32, y: f32) { self.0.move_to(x, -y); }
    fn line_to(&mut self, x: f32, y: f32) { self.0.line_to(x, -y); }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.quad_to(cx, -cy, x, -y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(cx0, -cy0, cx1, -cy1, x, -y);
    }
    fn close(&mut self) { self.0.close(); }
}

// Four horizontal positions keep fractional advances at small type sizes.
// Each mask is generated at its final position and physical size.
const PHASES: i32 = 4;

struct GlyphMask {
    left: i32,
    top: i32,
    coverage: Mask,
    image: Pixmap,
    ink: Option<Color>,
}

impl GlyphMask {
    fn rasterize(path: &Path, phase: u8) -> Option<Self> {
        let dx = f32::from(phase) / PHASES as f32;
        let bounds = path.bounds();
        let left = (bounds.left() + dx).floor();
        let top = bounds.top().floor();
        let w = ((bounds.right() + dx).ceil() - left) as u32;
        let h = (bounds.bottom().ceil() - top) as u32;
        // Unusually large outlines can still use the uncached vector painter.
        if w == 0 || h == 0 || w.checked_mul(h)? > 16_384 {
            return None;
        }
        let local = |p: Point| point(p.x + dx - left, p.y - top);
        let mut raster = Rasterizer::new(w as usize, h as usize);
        let mut start = point(0.0, 0.0);
        let mut prev = start;
        for segment in path.segments() {
            match segment {
                PathSegment::MoveTo(p) => {
                    start = local(p);
                    prev = start;
                }
                PathSegment::LineTo(p) => {
                    let end = local(p);
                    raster.draw_line(prev, end);
                    prev = end;
                }
                PathSegment::QuadTo(c, p) => {
                    let end = local(p);
                    raster.draw_quad(prev, local(c), end);
                    prev = end;
                }
                PathSegment::CubicTo(a, b, p) => {
                    let end = local(p);
                    raster.draw_cubic(prev, local(a), local(b), end);
                    prev = end;
                }
                PathSegment::Close => {
                    raster.draw_line(prev, start);
                    prev = start;
                }
            }
        }
        let mut coverage = Mask::new(w, h)?;
        let bytes = coverage.data_mut();
        raster.for_each_pixel(|index, alpha| bytes[index] = (alpha.min(1.0) * 255.0).round() as u8);
        Some(Self { left: left as i32, top: top as i32, coverage, image: Pixmap::new(w, h)?, ink: None })
    }

    fn draw(&mut self, dest: &mut Pixmap, x: i32, y: i32, color: Color) {
        if self.ink != Some(color) {
            self.image.fill(color);
            self.image.apply_mask(&self.coverage);
            self.ink = Some(color);
        }
        dest.draw_pixmap(x + self.left, y + self.top, self.image.as_ref(),
            &PixmapPaint::default(), Transform::identity(), None);
    }
}

#[derive(Default)]
pub(super) struct SettingsGlyphs {
    faces: HashMap<usize, Font>,
    sizes: HashMap<(usize, u32), ScalerFont>,
    masks: HashMap<(usize, u32, u16, u8), Option<GlyphMask>>,
}

impl SettingsGlyphs {
    pub fn draw(&mut self, dest: &mut Pixmap, face: &FontArc, glyph: u16, ppem: f32,
        x: f32, baseline: f32, color: Color) -> Option<()> {
        // A preview can prepend a font to its fallback list. Key by the actual
        // bytes, which the shared Blob keeps alive for the lifetime of the cache.
        let face_id = face.font_data().as_ptr() as usize;
        let font = match self.faces.entry(face_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(Font::new(
                Blob::Shared(Arc::new(FaceBytes(face.clone()))), 0)?),
        };
        let size = (ppem * 64.0).round().max(1.0) as u32;
        let position = (x * PHASES as f32).round() as i32;
        let phase = position.rem_euclid(PHASES) as u8;
        let key = (face_id, size, glyph, phase);
        if self.masks.len() >= 2048 && !self.masks.contains_key(&key) {
            self.masks.clear();
        }
        let entry = self.masks.entry(key).or_insert_with(|| {
            let size_key = (face_id, size);
            if self.sizes.len() >= 64 && !self.sizes.contains_key(&size_key) {
                self.sizes.clear();
            }
            let scaler = self.sizes.entry(size_key).or_insert_with(|| {
                let options = HintOptions::auto_fallback()
                    .with_target(RasterTarget::Light)
                    .with_preserve_linear_metrics(true)
                    .with_symmetric_rendering(false);
                ScalerFont::hinted(font.clone(), size as f32 / 64.0, options)
            });
            let outline = scaler.outline(GlyphId::new(u32::from(glyph)))?;
            let mut pen = DevicePath(PathBuilder::new());
            outline.draw(DrawMemory::Scratch, &mut pen).ok()?;
            GlyphMask::rasterize(&pen.0.finish()?, phase)
        });
        entry.as_mut()?.draw(dest, position.div_euclid(PHASES), baseline.round() as i32, color);
        Some(())
    }
}
