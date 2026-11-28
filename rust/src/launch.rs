//! Startup presentation. Completion follows real setup results; time only
//! controls readability and the visual handoff to the toolbar.
use std::time::{Duration, Instant};
use std::cell::RefCell;
use crate::splash_paint::Card;

pub const CARD_WIDTH: i32 = 560;
pub const CARD_HEIGHT: i32 = 332;
const LOCAL_REPORT: Duration = Duration::from_millis(720);
const LINK_REPORT: Duration = Duration::from_millis(210);
const READ_TIME: Duration = Duration::from_millis(1100);
const READY_TIME: Duration = Duration::from_millis(250);
const FADE_TIME: Duration = Duration::from_millis(220);
const REVEAL_DELAY: Duration = Duration::from_millis(140);
const REVEAL_TIME: Duration = Duration::from_millis(380);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState { Preparing, Disabled, Waiting, Connected, Unavailable }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Detail { Display, Tools, Wps(LinkState) }

impl LinkState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "正在连接",
            Self::Disabled => "未启用",
            Self::Waiting => "等待 WPS",
            Self::Connected => "已连接",
            Self::Unavailable => "连接失败",
        }
    }
}

pub struct Launch {
    pub desktop: String,
    pub resources_ok: bool,
    pub hotkey: bool,
    pub link: LinkState,
    pub(crate) card: RefCell<Option<Card>>,
    started: Option<Instant>,
    ready_at: Option<Instant>,
    progress: f32,
    panel_alpha: f32,
    reveal: f32,
}

impl Default for Launch {
    fn default() -> Self {
        Self { desktop: String::new(), resources_ok: true, hotkey: false,
            link: LinkState::Preparing, card: RefCell::new(None), started: None, ready_at: None,
            progress: 0.0, panel_alpha: 0.0, reveal: 1.0 }
    }
}

impl Launch {
    pub fn begin(desktop: String, resources_ok: bool, hotkey: bool, now: Instant) -> Self {
        Self { desktop, resources_ok, hotkey, link: LinkState::Preparing, card: RefCell::new(None),
            started: Some(now), ready_at: None, progress: 0.0, panel_alpha: 1.0, reveal: 0.0 }
    }
    pub fn active(&self) -> bool { self.started.is_some() }
    pub fn panel_alpha(&self) -> f32 { self.panel_alpha }
    pub fn reveal(&self) -> f32 { self.reveal }
    pub fn ready(&self) -> bool { self.ready_at.is_some() }
    pub fn progress(&self) -> f32 { self.progress }

    pub(crate) fn detail(&self) -> Detail {
        if self.progress < 0.32 { Detail::Display }
        else if self.progress < 0.74 { Detail::Tools }
        else { Detail::Wps(self.link) }
    }

    pub(crate) fn status_line(&self) -> std::borrow::Cow<'_, str> {
        use std::borrow::Cow;
        let message = match self.detail() {
            Detail::Display => return Cow::Owned(format!("已连接桌面 · {}", self.desktop)),
            Detail::Tools if !self.resources_ok => "部分字体或图标未载入",
            Detail::Tools if !self.hotkey => "批注工具已就绪，全局快捷键不可用",
            Detail::Tools => "批注工具已就绪 · Ctrl+Shift+D",
            Detail::Wps(LinkState::Preparing) => "正在连接 WPS 演示",
            Detail::Wps(LinkState::Waiting) => "工具栏已就绪，等待 WPS 演示",
            Detail::Wps(LinkState::Connected) => "已连接 WPS 演示",
            Detail::Wps(LinkState::Unavailable) => "WPS 联动暂不可用，可继续批注",
            Detail::Wps(LinkState::Disabled) => "工具栏已就绪",
        };
        Cow::Borrowed(message)
    }

    pub fn report_link(&mut self, link: LinkState, now: Instant) -> bool {
        if !self.active() { return false; }
        let changed = self.link != link;
        self.link = link;
        if link != LinkState::Preparing && self.ready_at.is_none() { self.ready_at = Some(now); }
        changed
    }

    pub fn advance_at(&mut self, now: Instant) -> bool {
        let Some(started) = self.started else { return false; };
        // Display, fonts and input have already been initialized at begin(). Present
        // their results smoothly; only a real bridge result unlocks the final stretch.
        let before = (self.progress, self.panel_alpha, self.reveal);
        let local = (now.saturating_duration_since(started).as_secs_f32() / LOCAL_REPORT.as_secs_f32()).min(1.0);
        self.progress = 0.74 * local;
        let Some(ready) = self.ready_at else { return self.progress != before.0; };
        let link_start = (started + LOCAL_REPORT).max(ready);
        let link = (now.saturating_duration_since(link_start).as_secs_f32() / LINK_REPORT.as_secs_f32()).min(1.0);
        self.progress += 0.26 * link;
        let exit = (started + READ_TIME).max(ready + READY_TIME);
        let elapsed = now.saturating_duration_since(exit);
        let fade = (elapsed.as_secs_f32() / FADE_TIME.as_secs_f32()).clamp(0.0, 1.0);
        let reveal = (elapsed.saturating_sub(REVEAL_DELAY).as_secs_f32()
            / REVEAL_TIME.as_secs_f32()).clamp(0.0, 1.0);
        self.panel_alpha = 1.0 - fade * fade * (3.0 - 2.0 * fade);
        self.reveal = 1.0 - (1.0 - reveal).powi(3);
        if reveal >= 1.0 { self.finish(); }
        before != (self.progress, self.panel_alpha, self.reveal)
    }

    pub fn finish(&mut self) {
        self.started = None;
        self.panel_alpha = 0.0;
        self.reveal = 1.0;
        self.card.get_mut().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_presents_local_results_and_waits_for_the_bridge_result() {
        let start = Instant::now();
        let mut launch = Launch::begin("X11 · 1920 × 1080".into(), true, false, start);
        let moment = |ms| start + Duration::from_millis(ms);
        assert!(launch.status_line().contains("1920 × 1080"));
        let mut last = 0.0;
        for ms in (0..=950).step_by(15) {
            launch.advance_at(moment(ms));
            let progress = launch.progress();
            assert!(progress >= last && progress - last < 0.025);
            last = progress;
            if ms == 450 { assert!(launch.status_line().contains("快捷键不可用")); }
        }
        assert_eq!(launch.detail(), Detail::Wps(LinkState::Preparing));
        assert!(last < 1.0);
        launch.advance_at(moment(3000));
        assert_eq!(launch.progress(), last);
        launch.report_link(LinkState::Unavailable, moment(3000));
        assert!(launch.status_line().contains("可继续批注"));
        launch.advance_at(moment(3100));
        assert!(launch.progress() > last && launch.progress() < 1.0);
        launch.advance_at(moment(3210));
        assert_eq!(launch.progress(), 1.0);
        assert_eq!(launch.panel_alpha(), 1.0);
    }

    #[test]
    fn time_cannot_invent_completion_and_optional_wps_never_blocks_startup() {
        let start = Instant::now();
        for link in [LinkState::Waiting, LinkState::Disabled, LinkState::Unavailable, LinkState::Connected] {
            let mut launch = Launch::begin("GNOME".into(), true, true, start);
            launch.advance_at(start + Duration::from_secs(10));
            assert!(launch.active() && !launch.ready());
            assert_eq!(launch.reveal(), 0.0);
            let ready = start + Duration::from_secs(10);
            launch.report_link(link, ready);
            launch.advance_at(ready + Duration::from_millis(200));
            assert_eq!(launch.panel_alpha(), 1.0);
            launch.advance_at(ready + Duration::from_secs(1));
            assert!(!launch.active());
            assert_eq!((launch.panel_alpha(), launch.reveal()), (0.0, 1.0));
        }
    }

    #[test]
    fn handoff_is_monotonic_and_heartbeats_do_not_restart_it() {
        let start = Instant::now();
        let mut launch = Launch::begin("X11".into(), true, false, start);
        launch.report_link(LinkState::Waiting, start);
        let (mut alpha, mut reveal) = (1.0, 0.0);
        for ms in (0..=1800).step_by(20) {
            let now = start + Duration::from_millis(ms);
            launch.report_link(LinkState::Waiting, now);
            launch.advance_at(now);
            assert!(launch.panel_alpha() <= alpha && launch.reveal() >= reveal);
            (alpha, reveal) = (launch.panel_alpha(), launch.reveal());
        }
        assert!(!launch.active());
        assert!(!launch.report_link(LinkState::Connected, start + Duration::from_secs(10)));
        assert!(!launch.advance_at(start + Duration::from_secs(10)));
    }
}
