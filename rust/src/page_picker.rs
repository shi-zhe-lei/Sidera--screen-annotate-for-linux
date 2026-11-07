//! A bounded page wheel. Selection is a preview; only the Jump button commits it.
use std::time::{Duration, Instant};

pub const ROW: f64 = 44.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source { Whiteboard, Wps(i32) }

struct Drag {
    start_y: i32, last_y: i32, start_position: f64,
    last_at: Instant, velocity: f64, moved: bool,
}
struct Snap { from: f64, to: f64, started: Instant, duration: Duration }

pub struct PagePicker {
    pub source: Source,
    pub total: i32,
    pub position: f64,
    drag: Option<Drag>,
    snap: Option<Snap>,
    scroll_goal: f64,
    last_scroll: Option<Instant>,
}

impl PagePicker {
    pub fn new(page: i32, total: i32, source: Source) -> Self {
        let total = total.max(1);
        let position = page.clamp(1, total) as f64;
        Self { source, total, position, drag: None, snap: None,
            scroll_goal: position, last_scroll: None }
    }
    pub fn dragging(&self) -> bool { self.drag.is_some() }
    pub fn set_selected(&mut self,page:i32) {
        self.drag=None;self.snap=None;self.last_scroll=None;
        self.position=page.clamp(1,self.total) as f64;self.scroll_goal=self.position;
    }
    pub fn selected(&self) -> i32 {
        self.snap.as_ref().map_or(self.position, |s| s.to).round().clamp(1.0, self.total as f64) as i32
    }
    pub fn advance_at(&mut self, now: Instant) -> bool {
        let Some(snap) = &self.snap else { return false };
        let t = (now.saturating_duration_since(snap.started).as_secs_f64()
            / snap.duration.as_secs_f64()).clamp(0.0, 1.0);
        self.position = snap.from + (snap.to - snap.from) * (1.0 - (1.0 - t).powi(3));
        if t >= 1.0 { self.position = snap.to; self.snap = None; }
        true
    }
    fn snap_to(&mut self, target: f64, now: Instant, millis: u64) {
        let target = target.round().clamp(1.0, self.total as f64);
        self.snap = Some(Snap { from: self.position, to: target, started: now,
            duration: Duration::from_millis(millis) });
    }
    pub fn scroll(&mut self, pages: f64, now: Instant) {
        if !pages.is_finite() || self.dragging() || pages == 0.0 { return; }
        self.advance_at(now);
        if self.last_scroll.is_none_or(|t| now.saturating_duration_since(t) > Duration::from_millis(200)) {
            self.scroll_goal = self.selected() as f64;
        }
        // Discrete wheels use exactly one page per notch. High-resolution input
        // accumulates fractions rather than losing small deltas or doubling OS inertia.
        self.scroll_goal = (self.scroll_goal + pages.clamp(-8.0, 8.0)).clamp(1.0, self.total as f64);
        self.last_scroll = Some(now);
        self.snap_to(self.scroll_goal, now, 160);
    }
    pub fn begin(&mut self, y: i32, now: Instant) {
        self.advance_at(now);
        self.snap = None;
        self.last_scroll = None;
        self.drag = Some(Drag { start_y: y, last_y: y, start_position: self.position,
            last_at: now, velocity: 0.0, moved: false });
    }
    pub fn motion(&mut self, y: i32, now: Instant) {
        let Some(drag) = self.drag.as_mut() else { return };
        let dy = y - drag.start_y;
        if !drag.moved && dy.abs() <= 4 { return; }
        drag.moved = true;
        let dt = now.saturating_duration_since(drag.last_at).as_secs_f64();
        if dt > 0.0 {
            let speed = (-(y - drag.last_y) as f64 / ROW / dt.max(0.008)).clamp(-18.0, 18.0);
            let blend = 1.0 - (-dt / 0.045).exp();
            drag.velocity += (speed - drag.velocity) * blend;
        }
        drag.last_y = y; drag.last_at = now;
        let raw = drag.start_position - dy as f64 / ROW;
        let bound = raw.clamp(1.0, self.total as f64);
        let excess = raw - bound;
        // Short rubber-band resistance, never wrapping to the other end.
        self.position = bound + excess * 0.30 / (1.0 + excess.abs());
    }
    pub fn release(&mut self, y: i32, center_y: i32, now: Instant) {
        if self.drag.as_ref().is_some_and(|d| d.last_y != y) { self.motion(y, now); }
        let Some(drag) = self.drag.take() else { return };
        let target = if drag.moved {
            let speed = if now.saturating_duration_since(drag.last_at) > Duration::from_millis(100) {
                0.0
            } else { drag.velocity };
            self.position + (speed * 0.16).clamp(-3.0, 3.0)
        } else {
            // Tapping an adjacent visible row centres that page without committing.
            self.position + (y - center_y) as f64 / ROW
        };
        let distance = (target - self.position).abs().min(6.0);
        self.snap_to(target, now, 220 + (distance * 16.0) as u64);
    }
    pub fn cancel_drag(&mut self) {
        self.drag = None; self.snap = None; self.last_scroll = None;
        self.position = self.position.round().clamp(1.0, self.total as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wheel_accumulates_notches_and_small_deltas_without_wrapping() {
        let now = Instant::now();
        let mut wheel = PagePicker::new(5, 20, Source::Whiteboard);
        for i in 0..3 { wheel.scroll(1.0, now + Duration::from_millis(i * 30)); }
        wheel.advance_at(now + Duration::from_secs(1));
        assert_eq!(wheel.position, 8.0);
        for i in 0..4 { wheel.scroll(0.25, now + Duration::from_millis(1100 + i * 20)); }
        wheel.advance_at(now + Duration::from_secs(2));
        assert_eq!(wheel.position, 9.0);
        for _ in 0..10 { wheel.scroll(-8.0, now + Duration::from_secs(3)); }
        wheel.advance_at(now + Duration::from_secs(4));
        assert_eq!(wheel.position, 1.0);
    }
    #[test]
    fn held_drag_loses_inertia_and_end_resistance_stays_bounded() {
        let now = Instant::now();
        let mut wheel = PagePicker::new(5, 10, Source::Whiteboard);
        wheel.begin(200, now);
        wheel.motion(112, now + Duration::from_millis(80));
        wheel.release(112, 200, now + Duration::from_millis(400));
        wheel.advance_at(now + Duration::from_secs(1));
        assert_eq!(wheel.position, 7.0);
        wheel.begin(200, now + Duration::from_secs(2));
        wheel.motion(-2000, now + Duration::from_millis(2100));
        assert!(wheel.position > 10.0 && wheel.position < 10.30);
        wheel.release(-2000, 200, now + Duration::from_millis(2110));
        wheel.advance_at(now + Duration::from_secs(3));
        assert_eq!(wheel.position, 10.0);
        assert!(!wheel.dragging());
    }
}
