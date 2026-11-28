//! The drawing tool drawer is independent of manually tucking the whole sidebar.
//! A frame samples its animation once, so painting and hit testing share geometry.
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use crate::ui::Btn;

const DURATION: Duration = Duration::from_millis(220);
const PAGE_OPEN_DELAY: Duration = Duration::from_millis(400);
const PAGE_CLOSE_DELAY: Duration = Duration::from_millis(650);

const PRESENTATION_IDLE: Duration = Duration::from_secs(6);
const FADE_DURATION: Duration = Duration::from_millis(300);
const IDLE_ALPHA: u8 = 51; // 20% of the toolbar's normal opacity.

#[derive(Default)]
pub(crate) struct ScreenshotFeedback {
    pub saved: Arc<AtomicBool>,
    until: Option<Instant>,
}

impl ScreenshotFeedback {
    pub fn visible(&self) -> bool { self.until.is_some() }

    pub fn advance_at(&mut self, now: Instant) -> bool {
        let was_visible = self.visible();
        if self.saved.swap(false, Ordering::Relaxed) {
            self.until = Some(now + Duration::from_millis(1800));
        } else if self.until.is_some_and(|end| now >= end) {
            self.until = None;
        }
        self.visible() != was_visible
    }
}

pub(crate) struct PresentationFade {
    idle_since: Option<Instant>,
    alpha: u8,
}

impl Default for PresentationFade {
    fn default() -> Self { Self { idle_since: None, alpha: 255 } }
}

impl PresentationFade {
    pub(crate) fn opacity(&self) -> f32 { f32::from(self.alpha) / 255.0 }

    pub(crate) fn reset(&mut self) -> bool {
        self.idle_since = None;
        std::mem::replace(&mut self.alpha, 255) != 255
    }

    /// Returns true only on a frame that actually changes the displayed alpha.
    pub(crate) fn advance_at(&mut self, eligible: bool, now: Instant) -> bool {
        if !eligible { return self.reset(); }
        if self.alpha == IDLE_ALPHA { return false; }
        let elapsed = now.saturating_duration_since(*self.idle_since.get_or_insert(now));
        if elapsed <= PRESENTATION_IDLE { return false; }
        let t = ((elapsed - PRESENTATION_IDLE).as_secs_f32() / FADE_DURATION.as_secs_f32()).min(1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        let alpha = (255.0 - f32::from(255 - IDLE_ALPHA) * eased).round() as u8;
        std::mem::replace(&mut self.alpha, alpha) != alpha
    }
}

#[derive(Clone, Copy)]
struct Transition { started: Instant, from: f32 }

#[derive(Clone, Copy)]
struct ToolTap { button: Btn, right: bool, x: i32, y: i32 }

const TAP_SLOP: u32 = 8; // Logical pixels, independent of display scale.
const RETAP_WINDOW: Duration = Duration::from_millis(400);

#[derive(Clone, Copy)]
struct ToolPress {
    tap: ToolTap,
    dragged: bool,
    preview: Option<f32>,
}

/// The thumb is visual feedback. The canvas keeps using App::mode until release.
#[derive(Default)]
pub struct ToolSelector {
    travel: Option<(Transition, i32)>,
    frame: f32,
    press: Option<ToolPress>,
    opening_tap: Option<(ToolTap, Instant)>,
}

impl ToolSelector {
    pub fn position(&self, mode: i32) -> f32 {
        if let Some(preview) = self.press.and_then(|press| press.preview) { return preview; }
        if self.travel.is_some_and(|(_, target)| target == mode) { self.frame } else { mode as f32 }
    }

    pub fn animating(&self) -> bool { self.travel.is_some() }

    pub(crate) fn begin(&mut self, button: Btn, right: bool, x: i32, y: i32) {
        self.press = Some(ToolPress { tap: ToolTap { button, right, x, y }, dragged: false, preview: None });
    }

    pub(crate) fn remember_opening_tap(&mut self, button: Btn, right: bool, x: i32, y: i32) {
        self.opening_tap = Some((ToolTap { button, right, x, y }, Instant::now()));
    }

    pub(crate) fn forget_opening_tap(&mut self) { self.opening_tap = None; }

    pub(crate) fn anchored_button(&self, mode: i32, right: bool, x: i32, y: i32) -> Option<Btn> {
        let tap = match self.press {
            Some(press) if !press.dragged => press.tap,
            Some(_) => return None,
            None => {
                let (tap, at) = self.opening_tap?;
                if at.elapsed() > RETAP_WINDOW || tap.button.tool_mode() != Some(mode) { return None; }
                tap
            }
        };
        (tap.right == right && x.abs_diff(tap.x) <= TAP_SLOP && y.abs_diff(tap.y) <= TAP_SLOP)
            .then_some(tap.button)
    }

    pub(crate) fn motion(&mut self, y: i32, hit: Option<(Btn, f32)>) -> bool {
        let Some(press) = self.press.as_mut() else { return false };
        // Sliding into another tool commits a drag. Finger wobble within the same
        // button and a surface leave without coordinates must still allow a tap.
        let preview = if !press.dragged && y.abs_diff(press.tap.y) <= TAP_SLOP { None } else {
            press.dragged |= hit.is_some_and(|(button, _)| button != press.tap.button);
            hit.map(|(_, position)| position)
        };
        let changed = press.preview != preview;
        press.preview = preview;
        changed
    }

    pub(crate) fn dragged(&self) -> Option<bool> { self.press.map(|press| press.dragged) }

    pub(crate) fn cancel_press(&mut self) { self.press = None; self.opening_tap = None; }

    pub(crate) fn select(&mut self, old: i32, target: i32) {
        self.opening_tap = None;
        let from = self.position(old);
        let preview = self.press.take().and_then(|press| press.preview);
        if old == target && preview.is_none() { return; }
        self.frame = from;
        self.travel = (from != target as f32).then(|| (Transition { started: Instant::now(), from }, target));
    }

    pub fn advance_at(&mut self, now: Instant) -> bool {
        let Some((motion, target)) = self.travel else { return false };
        let t = (now.saturating_duration_since(motion.started).as_secs_f32() / 0.15).min(1.0);
        self.frame = motion.from + (target as f32 - motion.from) * (1.0 - (1.0 - t).powi(3));
        if t >= 1.0 { self.travel = None; }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoUpdate { CollapseIfEmpty, FollowPage }

impl AutoUpdate {
    pub fn expanded_for(self, has_ink: bool) -> Option<bool> {
        if !has_ink { Some(false) }
        else if self == Self::FollowPage { Some(true) }
        else { None }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DrawerLayout { #[default] Compact, RestoreOnly, Expanded }

impl DrawerLayout {
    pub(crate) fn for_page(expanded: bool, can_restore: bool) -> Self {
        if expanded { Self::Expanded } else if can_restore { Self::RestoreOnly } else { Self::Compact }
    }

    fn frame(self) -> DrawerFrame {
        DrawerFrame {
            tools: if self == Self::Expanded { 1.0 } else { 0.0 },
            action: if self == Self::Compact { 0.0 } else { 1.0 },
        }
    }
}

#[derive(Clone, Copy, Default)]
struct DrawerFrame { tools: f32, action: f32 }

#[derive(Default)]
pub struct DrawingTools {
    layout: DrawerLayout,
    frame: DrawerFrame,
    transition: Option<(Instant, DrawerFrame)>,
    auto_update: Option<AutoUpdate>,
    page_deadline: Option<Instant>,
}

impl DrawingTools {
    pub fn expanded(&self) -> bool { self.layout == DrawerLayout::Expanded }
    pub(crate) fn layout(&self) -> DrawerLayout { self.layout }
    pub fn fraction(&self) -> f32 { self.frame.tools }
    pub(crate) fn action_fraction(&self) -> f32 { self.frame.action }
    pub fn animating(&self) -> bool { self.transition.is_some() }

    pub fn request_auto_collapse(&mut self) {
        if self.layout != DrawerLayout::Compact && self.auto_update != Some(AutoUpdate::FollowPage) {
            self.auto_update = Some(AutoUpdate::CollapseIfEmpty);
        }
    }

    pub fn request_page_update(&mut self) {
        self.auto_update = Some(AutoUpdate::FollowPage);
        self.page_deadline = None;
    }

    pub(crate) fn page_changed_at(&mut self, now: Instant) {
        self.auto_update = Some(AutoUpdate::FollowPage);
        self.page_deadline = Some(now + if self.expanded() { PAGE_CLOSE_DELAY } else { PAGE_OPEN_DELAY });
    }

    pub(crate) fn navigation_queued_at(&mut self, now: Instant) {
        // WPS can acknowledge a rapid second tap after the first page's dwell.
        // An animation step on an unchanged slide must not start a new check.
        if self.page_deadline.is_some() { self.page_changed_at(now); }
    }

    pub(crate) fn waiting_for_page(&self, now: Instant, in_use: bool) -> bool {
        self.page_deadline.is_some_and(|due| now < due || in_use)
    }

    pub fn take_auto_update(&mut self) -> Option<AutoUpdate> {
        self.page_deadline = None;
        self.auto_update.take()
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.set_expanded_at(expanded, Instant::now());
    }

    pub fn set_expanded_at(&mut self, expanded: bool, now: Instant) {
        self.set_layout_at(DrawerLayout::for_page(expanded, false), now);
    }

    pub(crate) fn set_layout_at(&mut self, layout: DrawerLayout, now: Instant) {
        self.auto_update = None;
        self.page_deadline = None;
        if self.layout == layout { return; }
        self.advance_at(now);
        self.layout = layout;
        self.transition = Some((now, self.frame));
    }

    pub fn advance_at(&mut self, now: Instant) -> bool {
        let Some((started, from)) = self.transition else { return false };
        let t = (now.saturating_duration_since(started).as_secs_f32()
            / DURATION.as_secs_f32()).clamp(0.0, 1.0);
        let target = self.layout.frame();
        let eased = 1.0 - (1.0 - t).powi(3);
        self.frame.tools = from.tools + (target.tools - from.tools) * eased;
        self.frame.action = from.action + (target.action - from.action) * eased;
        if t >= 1.0 { self.settle(); }
        true
    }

    pub fn settle(&mut self) {
        self.frame = self.layout.frame();
        self.transition = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PresentationAction { Escape, Start, End }

const PRESENTATION_DURATION: Duration = Duration::from_millis(240);

#[derive(Clone, Copy)]
struct PresentationFrame { started: Instant, from: [f32; 3] }

/// Visual state only: commands and hit testing use the current action immediately.
pub struct PresentationTransition {
    action: PresentationAction,
    weights: [f32; 3],
    transition: Option<PresentationFrame>,
}

impl Default for PresentationTransition {
    fn default() -> Self {
        Self { action: PresentationAction::Escape, weights: [1.0, 0.0, 0.0], transition: None }
    }
}

impl PresentationTransition {
    pub fn weights(&self) -> [f32; 3] { self.weights }

    pub fn set_action_at(&mut self, action: PresentationAction, now: Instant) {
        // Repeated WPS reports must not restart a transition that is in flight.
        if self.action == action { return; }
        self.advance_at(now);
        self.action = action;
        self.transition = Some(PresentationFrame { started: now, from: self.weights });
    }

    pub fn advance_at(&mut self, now: Instant) -> bool {
        let Some(frame) = self.transition else { return false };
        let t = (now.saturating_duration_since(frame.started).as_secs_f32()
            / PRESENTATION_DURATION.as_secs_f32()).clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        for (index, weight) in self.weights.iter_mut().enumerate() {
            let target = if index == self.action as usize { 1.0 } else { 0.0 };
            *weight = frame.from[index] + (target - frame.from[index]) * eased;
        }
        if t >= 1.0 { self.settle(); }
        true
    }

    pub fn settle(&mut self) {
        self.weights = [0.0; 3];
        self.weights[self.action as usize] = 1.0;
        self.transition = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_feedback_starts_on_save_and_renews_on_a_second_save() {
        let mut feedback = ScreenshotFeedback::default();
        let now = Instant::now();
        assert!(!feedback.advance_at(now) && !feedback.visible());
        feedback.saved.store(true, Ordering::Relaxed);
        assert!(feedback.advance_at(now) && feedback.visible());
        feedback.saved.store(true, Ordering::Relaxed);
        assert!(!feedback.advance_at(now + Duration::from_secs(1)));
        assert!(!feedback.advance_at(now + Duration::from_secs(2)) && feedback.visible());
        assert!(feedback.advance_at(now + Duration::from_secs(3)) && !feedback.visible());
    }

    #[test]
    fn presentation_fade_waits_six_seconds_and_only_repaints_changed_frames() {
        let mut fade = PresentationFade::default();
        let start = Instant::now();
        assert!(!fade.advance_at(true, start));
        assert!(!fade.advance_at(true, start + Duration::from_secs(6)));
        assert_eq!(fade.alpha, 255);
        assert!(fade.advance_at(true, start + Duration::from_millis(6150)));
        assert_eq!(fade.alpha, 153);
        assert!(fade.reset());
        let again = start + Duration::from_secs(20);
        assert!(!fade.advance_at(true, again));
        assert!(!fade.advance_at(true, again + Duration::from_secs(6)));
        assert!(fade.advance_at(true, again + Duration::from_millis(6300)));
        assert_eq!(fade.alpha, 51);
        assert!(!fade.advance_at(true, again + Duration::from_secs(300)));
        assert!(fade.advance_at(false, again + Duration::from_secs(301)));
        assert_eq!(fade.alpha, 255);
    }

    #[test]
    fn cursor_page_dwell_restarts_on_navigation_and_yields_to_an_explicit_tool() {
        let mut tools = DrawingTools::default();
        let start = Instant::now();
        tools.navigation_queued_at(start);
        assert_eq!(tools.take_auto_update(), None);
        tools.page_changed_at(start);
        assert!(tools.waiting_for_page(start + Duration::from_millis(399), false));
        tools.navigation_queued_at(start + Duration::from_millis(300));
        assert!(tools.waiting_for_page(start + Duration::from_millis(699), false));
        assert!(!tools.waiting_for_page(start + Duration::from_millis(700), false));
        assert!(tools.waiting_for_page(start + Duration::from_secs(3), true));
        tools.set_expanded_at(true, start);
        assert_eq!(tools.take_auto_update(), None);
        tools.page_changed_at(start);
        assert!(tools.waiting_for_page(start + Duration::from_millis(649), false));
        assert!(!tools.waiting_for_page(start + Duration::from_millis(650), false));
        tools.set_expanded_at(true, start);
        assert_eq!(tools.take_auto_update(), None);
    }

    #[test]
    fn page_updates_reconcile_folded_tools_but_explicit_choices_win() {
        let mut tools = DrawingTools::default();
        tools.request_page_update();
        tools.request_auto_collapse();
        assert_eq!(tools.take_auto_update(), Some(AutoUpdate::FollowPage));
        assert_eq!(AutoUpdate::FollowPage.expanded_for(true), Some(true));
        assert_eq!(AutoUpdate::FollowPage.expanded_for(false), Some(false));
        assert_eq!(AutoUpdate::CollapseIfEmpty.expanded_for(true), None);
        tools.request_page_update();
        tools.set_expanded(false);
        assert_eq!(tools.take_auto_update(), None, "a manual tuck cancels a queued reopen");
        tools.request_page_update();
        tools.set_expanded(true);
        assert_eq!(tools.take_auto_update(), None, "choosing the pen cancels a queued empty-page fold");
    }

    #[test]
    fn presentation_transition_keeps_the_visible_mix_when_retargeted() {
        let mut animation = PresentationTransition::default();
        let start = Instant::now();
        animation.set_action_at(PresentationAction::Start, start);
        let interrupted = start + Duration::from_millis(80);
        animation.advance_at(interrupted);
        let visible = animation.weights();
        assert!(visible[0] > 0.0 && visible[1] > 0.0);
        animation.set_action_at(PresentationAction::End, interrupted);
        assert_eq!(animation.weights(), visible);
        animation.advance_at(interrupted + Duration::from_millis(60));
        let visible = animation.weights();
        animation.set_action_at(PresentationAction::Escape, interrupted + Duration::from_millis(60));
        assert_eq!(animation.weights(), visible);
        // A heartbeat with the same action cannot postpone the final frame.
        animation.set_action_at(PresentationAction::Escape, interrupted + Duration::from_millis(180));
        assert!(animation.advance_at(interrupted + Duration::from_millis(300)));
        assert_eq!(animation.weights(), [1.0, 0.0, 0.0]);
        assert!(!animation.advance_at(start + Duration::from_secs(60)));
    }

    #[test]
    fn reversing_is_continuous_and_idle_ticks_never_reopen_the_drawer() {
        let mut tools = DrawingTools::default();
        let start = Instant::now();
        tools.set_expanded_at(true, start);
        let reverse = start + Duration::from_millis(70);
        tools.advance_at(reverse);
        let visible = tools.fraction();
        assert!(visible > 0.0 && visible < 1.0);
        tools.set_expanded_at(false, reverse);
        assert_eq!(tools.fraction(), visible);
        tools.advance_at(reverse + Duration::from_millis(60));
        assert!(tools.fraction() < visible && tools.fraction() > 0.0);
        tools.advance_at(reverse + DURATION);
        assert_eq!(tools.fraction(), 0.0);
        assert!(!tools.animating());
        tools.advance_at(start + Duration::from_secs(600));
        assert!(!tools.expanded());
        assert_eq!(tools.fraction(), 0.0);
    }
}
