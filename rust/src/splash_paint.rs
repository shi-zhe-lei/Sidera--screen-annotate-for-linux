//! Startup artwork and the small live strip repainted as initialization progresses.
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};
use crate::launch::{Detail, Launch, CARD_HEIGHT, CARD_WIDTH};
use crate::text::Fonts;
use crate::ui::{Ctx, R};
use crate::ui_theme::{self as theme, c};

const ART: &[u8] = include_bytes!("../../assets/source/startup-ink.png");
const ART_HEIGHT: i32 = 224;
const INSET: i32 = 30;
const TRACK: R = R { x: INSET, y: 246, w: CARD_WIDTH - 2 * INSET, h: 4 };
const STATUS: R = R { x: INSET - 1, y: 256, w: TRACK.w + 2, h: 31 };

// Keep one physical-resolution surface per display. Only the track and changed
// status text get repainted; fading reuses the fully composited pixels.
pub(crate) struct Card {
    pub(crate) pixels: Pixmap,
    transform: Transform,
    palette: theme::Palette,
    filled: Option<i32>,
    detail: Option<Detail>,
}

impl Card {
    pub(crate) fn new(fonts: &Fonts, icon: Option<&Pixmap>, w: u32, h: u32, palette: theme::Palette) -> Self {
        let mut pixels = Pixmap::new(w.max(1), h.max(1)).unwrap();
        let scale = (pixels.width() as f32 / CARD_WIDTH as f32).min(pixels.height() as f32 / CARD_HEIGHT as f32);
        let transform = Transform::from_scale(scale, scale);
        let mut ctx = Ctx { palette, pm: &mut pixels, tr: transform };
        // The artwork owns the top corners. Paper behind its antialiased edge
        // would leave a pale fringe on a dark desktop in the light theme.
        ctx.fill_round(R { x: 0, y: ART_HEIGHT - 12, w: CARD_WIDTH, h: CARD_HEIGHT - ART_HEIGHT + 12 }, 12.0, c(palette.paper));

        let art = Pixmap::decode_png(ART).expect("bundled startup illustration");
        let image_paint = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
        ctx.pm.draw_pixmap(0, 0, art.as_ref(), &image_paint, transform.pre_scale(
            CARD_WIDTH as f32 / art.width() as f32, ART_HEIGHT as f32 / art.height() as f32), None);
        if let Some(icon) = icon {
            ctx.pm.draw_pixmap(0, 0, icon.as_ref(), &image_paint, transform.pre_translate(INSET as f32, 298.0)
                .pre_scale(20.0 / icon.width() as f32, 20.0 / icon.height() as f32), None);
        }
        let credit_x = (if icon.is_some() { INSET + 28 } else { INSET }) as f32;
        ctx.text(fonts, "项目作者 Carl_Jin", credit_x, 313.0, 12.0, c(palette.muted));
        let contributors = "定制开发 Gary_6Stones · 教程与反馈 ofkuyu";
        let size = (11.0 * scale).round().max(1.0) / scale;
        let right = (CARD_WIDTH - INSET) as f32 - fonts.text_width(contributors, size);
        ctx.text(fonts, contributors, right, 313.0, size, c(palette.muted));

        Self { pixels, transform, palette, filled: None, detail: None }
    }

    pub(crate) fn update(&mut self, fonts: &Fonts, launch: &Launch) {
        let filled = (launch.progress() * TRACK.w as f32).round() as i32;
        let palette = self.palette;
        let mut ctx = Ctx { palette, pm: &mut self.pixels, tr: self.transform };
        if self.filled != Some(filled) {
            // Clear the antialiased ends so repeated frames never darken them.
            ctx.fill_round(R { x: TRACK.x - 1, y: TRACK.y - 1, w: TRACK.w + 2, h: TRACK.h + 2 }, 0.0, c(palette.paper));
            ctx.fill_round(TRACK, 2.0, c(palette.border));
            if filled > 0 {
                ctx.fill_round(R { w: filled, ..TRACK }, 2.0, c(palette.accent));
            }
            self.filled = Some(filled);
        }
        let detail = launch.detail();
        if self.detail != Some(detail) {
            ctx.fill_round(STATUS, 0.0, c(palette.paper));
            ctx.text(fonts, &launch.status_line(), INSET as f32, 277.0, 15.0, c(palette.text));
            self.detail = Some(detail);
        }
    }
}

pub(crate) fn activity_bounds(card: R) -> R {
    let sx = card.w as f32 / CARD_WIDTH as f32;
    let sy = card.h as f32 / CARD_HEIGHT as f32;
    let left = ((TRACK.x - 2) as f32 * sx).floor() as i32;
    let top = ((TRACK.y - 2) as f32 * sy).floor() as i32;
    let right = ((TRACK.x + TRACK.w + 2) as f32 * sx).ceil() as i32;
    let bottom = ((STATUS.y + STATUS.h + 1) as f32 * sy).ceil() as i32;
    R { x: card.x + left, y: card.y + top, w: right - left, h: bottom - top }
}

pub fn draw(fonts: &Fonts, icon: Option<&Pixmap>, launch: &Launch, w: u32, h: u32, palette: theme::Palette) -> Pixmap {
    let mut card = Card::new(fonts, icon, w, h, palette);
    card.update(fonts, launch);
    card.pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use crate::launch::LinkState;

    #[test]
    fn one_continuous_track_and_a_stationary_cover_in_both_themes() {
        let start = Instant::now();
        let fonts = Fonts::load();
        for palette in [theme::LIGHT, theme::DARK] {
            let mut launch = Launch::begin("GNOME".into(), true, true, start);
            let mut card = Card::new(&fonts, None, CARD_WIDTH as u32, CARD_HEIGHT as u32, palette);
            card.update(&fonts, &launch);
            let end_of_art = CARD_WIDTH as usize * ART_HEIGHT as usize * 4;
            let original_art = card.pixels.data()[..end_of_art].to_vec();
            launch.report_link(LinkState::Waiting, start);
            for ms in [180, 510, 810, 950] {
                launch.advance_at(start + Duration::from_millis(ms));
                card.update(&fonts, &launch);
                let end = TRACK.x + (launch.progress() * TRACK.w as f32).round() as i32;
                let y = (TRACK.y + TRACK.h / 2) as u32;
                for x in TRACK.x + 2..end - 2 {
                    assert_eq!(card.pixels.pixel(x as u32, y).unwrap(), c(palette.accent).to_color_u8().premultiply());
                }
                for x in end + 2..TRACK.x + TRACK.w - 2 {
                    assert_eq!(card.pixels.pixel(x as u32, y).unwrap(), c(palette.border).to_color_u8().premultiply());
                }
                assert_eq!(&card.pixels.data()[..end_of_art], original_art.as_slice());
            }
        }
    }
}
