// ============================================================
// Sidera - 字形栅格化（ab_glyph → tiny-skia 路径）
// 纯 Rust；从系统字体目录按候选列表加载，缺字体时自动降级跳过
// ============================================================
use ab_glyph::{Font, FontArc, OutlineCurve, PxScale, ScaleFont};
use std::path::{Path, PathBuf};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Shader, Transform};

#[derive(Clone)]
pub struct Fonts {
    pub list: Vec<FontArc>,
}

fn score(path: &str) -> i32 {
    let s = path.to_lowercase();
    let mut v = 0;
    if s.contains("msyh") || s.contains("microsoft yahei") || s.contains("simhei") || s.contains("simsun") || s.contains("dengxian") {
        v += 1000;
    }
    if s.contains("cjk")
        || s.contains("wqy")
        || s.contains("sourcehan")
        || s.contains("sarasa")
        || s.contains("droidsansfallback")
        || s.contains("uming")
        || s.contains("ukai")
        || s.contains("microhei")
        || s.contains("zenhei")
    {
        v += 1000;
    }
    if s.contains("dejavusans")
        || s.contains("liberationsans")
        || s.contains("freesans")
        || s.contains("notosans")
        || s.contains("carlito")
        || s.contains("opensans")
        || s.contains("firamsans")
    {
        v += 500;
    }
    if s.contains("sans") {
        v += 50;
    }
    if s.contains("regular") || s.contains("medium") || s.contains("book") {
        v += 20;
    }
    if s.contains("bold") || s.contains("italic") || s.contains("oblique") {
        v -= 20;
    }
    if s.contains("mono")
        || s.contains("nerd")
        || s.contains("serif")
        || s.contains("symbol")
        || s.contains("emoji")
        || s.contains("condensed")
        || s.contains("compressed")
        || s.contains("display")
    {
        v -= 200;
    }
    v
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth > 6 || out.len() > 4000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out, depth + 1);
        } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            let e = ext.to_lowercase();
            if e == "ttf" || e == "otf" || e == "ttc" {
                out.push(p);
            }
        }
    }
}

impl Fonts {
    pub fn load() -> Self {
        static FONTS: std::sync::OnceLock<Fonts> = std::sync::OnceLock::new();
        FONTS.get_or_init(Self::load_system).clone()
    }
    fn load_system() -> Self {
        let mut files: Vec<PathBuf> = Vec::new();
        collect(Path::new("/usr/share/fonts"), &mut files, 0);
        collect(Path::new("/usr/local/share/fonts"), &mut files, 0);
        #[cfg(windows)]
        {
            if let Ok(windir) = std::env::var("WINDIR") {
                collect(&Path::new(&windir).join("Fonts"), &mut files, 0);
            }
            if let Some(d) = dirs::data_local_dir() {
                collect(&d.join("Microsoft").join("Windows").join("Fonts"), &mut files, 0);
            }
        }
        if let Some(d) = dirs::data_dir() {
            collect(&d.join("fonts"), &mut files, 0);
        }
        if let Some(h) = dirs::home_dir() {
            collect(&h.join(".fonts"), &mut files, 0);
        }
        files.sort_by_key(|p| -score(&p.to_string_lossy()));

        let mut list = Vec::new();
        let samples: Vec<char> = "课堂画笔橡皮清除撤回设置页码跳转开始结束放映白板0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz".chars().collect();
        let mut covered = vec![false; samples.len()];
        for p in &files {
            if list.len() >= 8 || covered.iter().all(|found| *found) { break; }
            // Bound malformed/oversized font files without an arbitrary first-40 cutoff.
            if std::fs::metadata(p).map(|m| m.len() > 64 * 1024 * 1024).unwrap_or(true) { continue; }
            if let Ok(data) = std::fs::read(p) {
                if let Ok(f) = FontArc::try_from_vec(data) {
                    let mut useful = list.is_empty();
                    for (index, ch) in samples.iter().enumerate() {
                        if !covered[index] && f.glyph_id(*ch).0 != 0 {
                            covered[index] = true;
                            useful = true;
                        }
                    }
                    if useful { list.push(f); }
                }
            }
        }
        if list.is_empty() {
            log::warn!("[字体] 未找到可用系统字体，界面文字将不显示");
        } else {
            log::info!("[字体] 已加载 {} 个字体", list.len());
            if list.iter().all(|f| f.glyph_id('画').0 == 0) {
                log::warn!("[字体] 缺少中文字形，请安装 fonts-noto-cjk 或等效中文字体");
            }
        }
        Fonts { list }
    }
    fn pick(&self, ch: char) -> Option<&FontArc> {
        self.list.iter().find(|f| f.glyph_id(ch).0 != 0)
    }

    pub fn text_width(&self, s: &str, px: f32) -> f32 {
        let mut w = 0.0;
        for ch in s.chars() {
            if let Some(f) = self.pick(ch) {
                let sf = f.as_scaled(PxScale::from(px));
                let gid = sf.glyph_id(ch);
                w += sf.h_advance(gid);
            } else {
                w += px * 0.5;
            }
        }
        w
    }

    /// 以 (x, baseline_y) 为起点绘制文本，颜色 color，字号 px。
    pub fn draw(
        &self,
        pm: &mut Pixmap,
        s: &str,
        x: f32,
        baseline_y: f32,
        px: f32,
        col: Color,
        transform: Transform,
    ) {
        if self.list.is_empty() {
            return;
        }
        let paint = Paint {
            shader: Shader::SolidColor(col),
            anti_alias: true,
            ..Default::default()
        };
        let mut caret = x;
        for ch in s.chars() {
            let Some(f) = self.pick(ch) else {
                caret += px * 0.5;
                continue;
            };
            let sf = f.as_scaled(PxScale::from(px));
            let gid = sf.glyph_id(ch);
            let adv = sf.h_advance(gid);
            // 必须与 ab_glyph 的 h_advance 用同一个缩放因子（px / height_unscaled），
            // 否则字宽与字形大小不一致 → 字挤在一起。
            let sc = sf.h_scale_factor();
            if let Some(ol) = f.outline(gid) {
                let map = |p: &ab_glyph::Point| (caret + p.x * sc, baseline_y - p.y * sc);
                let mut pb = PathBuilder::new();
                // 轮廓由多段曲线组成且共享端点；只有新轮廓起点才 move_to，
                // 否则每段都开新子路径，填充时会互相抵消（字变骨架）。
                let mut cur: Option<(f32, f32)> = None;
                for c in &ol.curves {
                    match c {
                        OutlineCurve::Line(p0, p1) => {
                            let a = map(p0);
                            let b = map(p1);
                            if cur != Some(a) {
                                pb.move_to(a.0, a.1);
                            }
                            pb.line_to(b.0, b.1);
                            cur = Some(b);
                        }
                        OutlineCurve::Quad(p0, p1, p2) => {
                            let a = map(p0);
                            let b = map(p1);
                            let c = map(p2);
                            if cur != Some(a) {
                                pb.move_to(a.0, a.1);
                            }
                            pb.quad_to(b.0, b.1, c.0, c.1);
                            cur = Some(c);
                        }
                        OutlineCurve::Cubic(p0, p1, p2, p3) => {
                            let a = map(p0);
                            let b = map(p1);
                            let c = map(p2);
                            let d = map(p3);
                            if cur != Some(a) {
                                pb.move_to(a.0, a.1);
                            }
                            pb.cubic_to(b.0, b.1, c.0, c.1, d.0, d.1);
                            cur = Some(d);
                        }
                    }
                }
                if let Some(path) = pb.finish() {
                    pm.fill_path(&path, &paint, FillRule::Winding, transform, None);
                }
            }
            caret += adv;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_text() {
        let fonts = Fonts::load();
        for (i, f) in fonts.list.iter().enumerate() {
            eprintln!(
                "font[{}] units={:?} gid(测)={} gid(A)={}",
                i,
                f.units_per_em(),
                f.glyph_id('测').0,
                f.glyph_id('A').0
            );
        }
        let mut pm = Pixmap::new(240, 60).unwrap();
        fonts.draw(
            &mut pm,
            "测试画笔",
            5.0,
            40.0,
            30.0,
            Color::WHITE,
            Transform::identity(),
        );
        let nonzero = pm.data().chunks_exact(4).filter(|p| p[3] != 0).count();
        eprintln!("nonzero px = {}", nonzero);
        assert!(nonzero > 50, "Chinese glyphs did not render; install fonts-noto-cjk (nonzero={})", nonzero);
    }

    #[test]
    fn missing_glyphs_reserve_the_same_width_as_drawing() {
        let fonts = Fonts { list: Vec::new() };
        assert_eq!(fonts.text_width("画笔A", 20.0), 30.0);
    }
}
