//! Classroom startup card, shared by every backend and offline previews.
use tiny_skia::{Pixmap, PixmapPaint, Transform};
use crate::launch::{Launch, LinkState, CARD_HEIGHT, CARD_WIDTH};
use crate::text::Fonts;
use crate::ui::{Ctx, R};
use crate::ui_theme::{self as theme, alpha, c};

fn status(ctx: &mut Ctx, y: i32, state: u8) {
    let palette = ctx.palette;
    let (fill, ink) = match state {
        1 => (palette.accent_light, palette.accent),
        2 => (palette.warning_light, palette.warning),
        _ => (palette.surface, palette.muted),
    };
    ctx.fill_circle(43.0, y as f32 + 17.0, 10.0, c(fill));
    match state {
        1 => {
            ctx.stroke_line((39.0, y as f32 + 17.0), (42.0, y as f32 + 20.0), c(ink), 1.7);
            ctx.stroke_line((42.0, y as f32 + 20.0), (47.0, y as f32 + 14.0), c(ink), 1.7);
        }
        2 => {
            ctx.stroke_line((43.0, y as f32 + 12.0), (43.0, y as f32 + 17.0), c(ink), 1.7);
            ctx.fill_circle(43.0, y as f32 + 21.0, 1.1, c(ink));
        }
        _ => {
            ctx.stroke_line((43.0, y as f32 + 12.0), (43.0, y as f32 + 17.0), c(ink), 1.5);
            ctx.stroke_line((43.0, y as f32 + 17.0), (46.0, y as f32 + 18.0), c(ink), 1.5);
        }
    }
}

pub fn draw(fonts: &Fonts, icon: Option<&Pixmap>, launch: &Launch, w: u32, h: u32, palette: theme::Palette) -> Pixmap {
    let mut pm = Pixmap::new(w.max(1), h.max(1)).unwrap();
    let scale = (w as f32 / CARD_WIDTH as f32).min(h as f32 / CARD_HEIGHT as f32);
    let tr = Transform::from_scale(scale, scale);
    let mut ctx = Ctx { palette, pm: &mut pm, tr };
    ctx.fill_round(R { x: 1, y: 1, w: 558, h: 446 }, 26.0, c(palette.paper));
    ctx.stroke_round(R { x: 2, y: 2, w: 556, h: 444 }, 25.0, c(palette.border), 1.2);
    // A quiet mint inset ties the original mark to the existing classroom palette.
    ctx.fill_round(R { x: 18, y: 18, w: 524, h: 183 }, 19.0, c(palette.splash_header));
    ctx.text(fonts, "课堂批注", 32.0, 41.0, 15.0, c(palette.muted));
    ctx.fill_round(R { x: 459, y: 26, w: 67, h: 23 }, 11.0, c(palette.paper));
    let short_version = crate::app::VERSION.split('-').next().unwrap_or(crate::app::VERSION);
    ctx.text_center(fonts, R { x: 459, y: 26, w: 67, h: 23 }, short_version, 14.0, c(palette.muted));
    if let Some(icon) = icon {
        ctx.fill_round(R { x: 237, y: 38, w: 86, h: 86 }, 21.0, c(palette.paper));
        ctx.pm.draw_pixmap(0, 0, icon.as_ref(), &PixmapPaint {
            quality: tiny_skia::FilterQuality::Bilinear, ..Default::default()
        }, tr.pre_translate(242.0, 43.0).pre_scale(76.0 / icon.width() as f32, 76.0 / icon.height() as f32), None);
    } else {
        ctx.fill_circle(280.0, 81.0, 34.0, c(palette.accent_light));
        ctx.text_center(fonts, R { x: 246, y: 47, w: 68, h: 68 }, "S", 43.0, c(palette.accent));
    }
    ctx.text_center(fonts, R { x: 28, y: 121, w: 504, h: 47 }, "Sidera", 40.0, c(palette.text));
    ctx.text_center(fonts, R { x: 28, y: 169, w: 504, h: 21 }, "屏幕批注  ·  白板书写  ·  课件联动", 17.0, c(palette.muted));

    let resource = if launch.resources_ok { "外观设置、字体与图标已载入" }
        else { "外观设置已载入，部分资源使用替代显示" };
    let input = if launch.hotkey { "画笔、白板与快捷键已准备" } else { "画笔与白板已准备" };
    let link_state = match launch.link {
        LinkState::Connected => 1, LinkState::Unavailable => 2, _ => 0,
    };
    let rows = [
        ("桌面连接", launch.desktop.as_str(), 1),
        ("界面资源", resource, if launch.resources_ok { 1 } else { 2 }),
        ("绘图交互", input, 1),
        ("WPS 联动", launch.link.label(), link_state),
    ];
    for (i, (label, detail, state)) in rows.into_iter().enumerate() {
        let y = 211 + i as i32 * 35;
        status(&mut ctx, y, state);
        ctx.text(fonts, label, 65.0, y as f32 + 23.0, 17.0, c(palette.text));
        let size = (17.0 * (340.0 / fonts.text_width(detail, 17.0).max(1.0)).min(1.0)).max(12.0);
        ctx.text(fonts, detail, 177.0, y as f32 + 22.0, size,
            c(if state == 2 { palette.warning } else { palette.muted }));
    }

    ctx.stroke_line((32.0, 359.0), (528.0, 359.0), alpha(palette.border, 180), 1.0);
    ctx.text(fonts, if launch.ready() { "准备就绪" } else { "正在准备课堂工具" },
        32.0, 385.0, 19.0, c(palette.accent));
    let summary = if launch.ready() { "可以开始批注" } else { "正在连接课件服务" };
    ctx.text(fonts, summary, 528.0 - fonts.text_width(summary, 15.0), 384.0, 15.0, c(palette.muted));
    for i in 0..4 {
        ctx.fill_round(R { x: 32 + i * 126, y: 397, w: 118, h: 4 }, 2.0,
            c(if (i as usize) < launch.completed() { palette.accent } else { palette.border }));
    }
    ctx.text(fonts, "developed by jinyicheng", 32.0, 420.0, 13.0, alpha(palette.muted, 215));
    ctx.text(fonts, "定制与改进：shizhelei · Gary_6Stones", 32.0, 437.0, 11.0, alpha(palette.muted, 190));
    let version = crate::app::VERSION;
    ctx.text(fonts, version, 528.0 - fonts.text_width(version, 12.0), 437.0, 12.0, alpha(palette.muted, 190));
    pm
}
