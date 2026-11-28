//! A spring-centred rate control. The rail morph and thumb return are visual
//! only. A held gesture is the sole owner of a foreground document target.
use std::time::{Duration, Instant};

pub trait ScrollTarget {
    /// Signed logical pixels. False means the original document is unavailable.
    fn scroll_by(&mut self, pixels: f64) -> bool;
    fn clear_pending(&mut self) {}
}

#[derive(Default)]
pub(crate) struct Rail {
    pub ppt_foreground: bool,
    pub mix: f32,
    target: bool,
    morph: Option<(Instant, f32)>,
    pub thumb: f32,
    pub held: Option<bool>,
    returning: Option<(Instant, f32)>,
    pub last_page: (i32, i32),
    pub unavailable: bool,
}

fn ease(now: Instant, start: Instant, duration: f32) -> f32 {
    let t = (now.saturating_duration_since(start).as_secs_f32() / duration).min(1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Rail {
    pub fn advance(&mut self, slider: bool, now: Instant) -> bool {
        let before = (self.mix, self.thumb);
        if let Some((at, from)) = self.morph {
            let t = ease(now, at, 0.19);
            self.mix = from + (f32::from(self.target) - from) * t;
            if t >= 1.0 {
                self.morph = None;
            }
        }
        if self.target != slider {
            self.target = slider;
            self.morph = Some((now, self.mix));
        }
        if let Some((at, from)) = self.returning {
            let t = ease(now, at, 0.13);
            self.thumb = from * (1.0 - t);
            if t >= 1.0 {
                self.returning = None;
            }
        }
        before != (self.mix, self.thumb)
    }

    pub fn begin(&mut self, right: bool) {
        self.held = Some(right);
        self.returning = None;
        self.unavailable = false;
    }

    pub fn release(&mut self, now: Instant) {
        if self.held.take().is_some() && self.thumb != 0.0 {
            self.returning = Some((now, self.thumb));
        }
        self.unavailable = false;
    }
}

pub(crate) struct Drag {
    origin_y: f32,
    travel: f32,
    previous_frame: Instant,
    pub target: Option<Box<dyn ScrollTarget>>,
    pub notice: Option<crate::ui::R>,
}

impl Drag {
    pub fn new(y: i32, centre: i32, travel: f32, thumb: f32, grip_height: i32, now: Instant) -> Self {
        // Pick up the thumb where the finger landed without jumping. Touching
        // the track directly selects a speed without requiring a second motion.
        let current = centre as f32 + thumb * travel;
        let offset = if (y as f32 - current).abs() <= grip_height as f32 / 2.0 {
            y as f32 - current
        } else {
            0.0
        };
        Self {
            origin_y: centre as f32 + offset,
            travel: travel.max(1.0),
            previous_frame: now,
            target: None,
            notice: None,
        }
    }

    pub fn position(&self, y: i32) -> f32 {
        let dy = y as f32 - self.origin_y;
        let neutral = 3.0_f32.min(self.travel / 5.0);
        (dy.signum() * (dy.abs() - neutral).max(0.0) / (self.travel - neutral)).clamp(-1.0, 1.0)
    }

    pub fn distance(&mut self, position: f32, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.previous_frame);
        if position != 0.0 && elapsed < Duration::from_millis(16) {
            return 0.0;
        }
        let dt = elapsed.min(Duration::from_millis(40));
        self.previous_frame = now;
        1400.0 * f64::from(position) * dt.as_secs_f64()
    }
}
