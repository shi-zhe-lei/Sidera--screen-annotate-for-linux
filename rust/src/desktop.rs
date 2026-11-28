//! Common multi-output controller for native Wayland and the GNOME extension.
//! Each output owns its canvas and caches; controls and WPS events are shared.
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::io::Read;
use crate::instance::Instance as UnixListener;
use std::time::{Duration, Instant};
use tiny_skia::Pixmap;
use crate::{app, App, Backend, Fonts, Ip, Rt, Timers, WpsBridge};
use crate::gesture::TouchKind;

pub struct Output {
    pub rt: Rt,
    pub app: App,
    pub ip: Ip,
    timers: Timers,
    launch_pending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Desktop {
        let (rt, app, _) = crate::navigation_tests::fixture(true);
        let mut desktop = Desktop {
            outputs: BTreeMap::new(), wps: None, controls: Controls::read(&app),
            detached: VecDeque::new(), fonts: rt.fonts.clone(), icon: None, icon_big: None, single: None, starting: false,
        };
        desktop.outputs.insert("laptop".into(), Output { rt, app, ip: Ip::default(), timers: Timers::new(), launch_pending: false });
        let (rt, app, _) = crate::navigation_tests::fixture(true);
        desktop.outputs.insert("projector".into(), Output { rt, app, ip: Ip::default(), timers: Timers::new(), launch_pending: false });
        desktop
    }

    #[test]
    fn collapsed_tab_shares_the_original_deadline_with_other_outputs() {
        let mut desktop = fixture();
        let start = Instant::now() - Duration::from_secs(30);
        let laptop = &mut desktop.outputs.get_mut("laptop").unwrap().app;
        laptop.sidebar_expand_minutes = 1;
        laptop.set_sidebar_collapsed_since(Some(start));
        desktop.synchronize("laptop");
        let projector = &desktop.outputs["projector"].app;
        assert_eq!(projector.sidebar_collapsed_since, Some(start));
        assert_eq!(projector.sidebar_countdown_at(start + Duration::from_secs(30)), Some(128));
        assert_eq!(projector.sidebar_countdown_at(start + Duration::from_secs(60)), Some(0));
    }

    #[test]
    fn cursor_and_pages_follow_ink_on_any_output_without_untucking_sidebars() {
        let mut desktop = fixture();
        desktop.toggle();
        let projector = desktop.outputs.get_mut("projector").unwrap();
        crate::paint::stroke_tapered(&mut projector.app, (20, 20), (80, 20), 6, 6);
        desktop.toggle();
        assert!(desktop.outputs.values().all(|o| o.app.mode == 0 && o.app.drawing_tools.expanded()));
        let laptop = desktop.outputs.get_mut("laptop").unwrap();
        crate::go_next(&laptop.rt, &mut laptop.app, None);
        desktop.synchronize("laptop");
        assert!(desktop.outputs.values().all(|o| o.app.drawing_tools.expanded()));
        desktop.resolve_drawing_tools_at(Instant::now() + Duration::from_secs(1));
        assert!(desktop.outputs.values().all(|o| o.app.current_slide == 2 && !o.app.drawing_tools.expanded()));
        assert!(!desktop.controls.drawing_tools);
        desktop.motion("projector", 50, 50);
        let laptop = desktop.outputs.get_mut("laptop").unwrap();
        crate::go_prev(&laptop.rt, &mut laptop.app, None);
        desktop.synchronize("laptop");
        assert!(desktop.outputs.values().all(|o| !o.app.drawing_tools.expanded()));
        desktop.resolve_drawing_tools_at(Instant::now() + Duration::from_secs(1));
        assert!(desktop.outputs["projector"].app.canvas.pixels().iter().any(|p| p.alpha() != 0));
        assert!(desktop.outputs.values().all(|o| o.app.mode == 0 && o.app.drawing_tools.expanded()));
        assert!(desktop.controls.drawing_tools);
        desktop.motion("projector", 50, 50);
        assert!(desktop.outputs.values().all(|o| o.app.drawing_tools.expanded()));

        let laptop = desktop.outputs.get_mut("laptop").unwrap();
        crate::collapse_sidebars(&laptop.rt, &mut laptop.app);
        desktop.synchronize("laptop");
        for next in [true, false] {
            let laptop = desktop.outputs.get_mut("laptop").unwrap();
            if next { crate::go_next(&laptop.rt, &mut laptop.app, None); }
            else { crate::go_prev(&laptop.rt, &mut laptop.app, None); }
            desktop.synchronize("laptop");
            assert!(desktop.outputs.values().all(|o| o.app.collapsed && !o.app.drawing_tools.expanded()));
            assert!(desktop.controls.collapsed_since.is_some() && !desktop.controls.drawing_tools);
        }
    }

    #[test]
    fn a_cleared_output_keeps_its_own_restore_key() {
        use crate::toolbar::DrawerLayout;
        let mut desktop = fixture();
        desktop.toggle();
        let projector = desktop.outputs.get_mut("projector").unwrap();
        crate::paint::stroke_tapered(&mut projector.app, (20, 20), (80, 20), 5, 5);
        crate::clear_drawing(&projector.rt, &mut projector.app);
        desktop.synchronize("projector");
        assert_eq!(desktop.outputs["projector"].app.drawing_tools.layout(), DrawerLayout::RestoreOnly);
        assert_eq!(desktop.outputs["laptop"].app.drawing_tools.layout(), DrawerLayout::Compact);
        for next in [true, false] {
            let laptop = desktop.outputs.get_mut("laptop").unwrap();
            if next { crate::go_next(&laptop.rt, &mut laptop.app, None); }
            else { crate::go_prev(&laptop.rt, &mut laptop.app, None); }
            desktop.synchronize("laptop");
            desktop.resolve_drawing_tools_at(Instant::now() + Duration::from_secs(1));
            assert_eq!(desktop.outputs["projector"].app.drawing_tools.layout(),
                if next { DrawerLayout::Compact } else { DrawerLayout::RestoreOnly });
            assert_eq!(desktop.outputs["laptop"].app.drawing_tools.layout(), DrawerLayout::Compact);
        }
    }

    #[test]
    fn native_cursor_button_folds_empty_tools_and_whiteboard_clear_keeps_them() {
        let mut desktop = fixture();
        desktop.toggle();
        let laptop = desktop.outputs.get_mut("laptop").unwrap();
        crate::switch_to_cursor(&laptop.rt, &mut laptop.app);
        desktop.synchronize("laptop");
        assert!(desktop.outputs.values().all(|o| !o.app.drawing_tools.expanded()));
        desktop.outputs.get_mut("laptop").unwrap().app.toggle_whiteboard();
        desktop.synchronize("laptop");
        let laptop = desktop.outputs.get_mut("laptop").unwrap();
        crate::clear_drawing(&laptop.rt, &mut laptop.app);
        desktop.synchronize("laptop");
        assert!(desktop.outputs.values().all(|o| o.app.whiteboard && o.app.mode == 1 && o.app.drawing_tools.expanded()));
        desktop.outputs.get_mut("laptop").unwrap().app.toggle_whiteboard();
        desktop.synchronize("laptop");
        assert!(desktop.outputs.values().all(|o| o.app.mode == 0 && !o.app.drawing_tools.expanded()));
    }

    #[test]
    fn toolbar_display_options_in_appearance_reach_other_outputs() {
        let mut desktop = fixture();
        use crate::ui::SetHit::{FadeToolbarDuringShow, ToolbarIconsOnly};
        for hit in [FadeToolbarDuringShow, ToolbarIconsOnly] {
            let app = &mut desktop.outputs.get_mut("laptop").unwrap().app;
            app.configure_geometry(640, 360, 1.0);
            app.settings_open = true;
            app.settings_ui.select(crate::settings_state::Tab::General);
            assert!(!crate::settings_panel::controls(app).iter().any(|(h, _)| *h == hit));
            app.settings_ui.select(crate::settings_state::Tab::Appearance);
            let (_, row) = (0..crate::settings_panel::layout(app).pages).find_map(|page| {
                app.settings_page = page;
                crate::settings_panel::controls(app).into_iter().find(|(h, _)| *h == hit)
            }).unwrap();
            let (x, y) = (row.x + row.w - 32, row.y + row.h / 2);
            for enabled in [true, false] {
                desktop.press("laptop", x, y, 1);
                desktop.release("laptop", x, y, 1);
                assert!(desktop.outputs.values().all(|o| {
                    let actual = if hit == ToolbarIconsOnly { o.app.toolbar_icons_only } else { o.app.fade_toolbar_during_show };
                    actual == enabled
                }));
            }
        }
    }

    #[test]
    fn theme_changes_synchronize_without_changing_each_displays_ink() {
        let mut desktop = fixture();
        let output = desktop.outputs.get_mut("projector").unwrap();
        crate::paint::stroke_tapered(&mut output.app, (20, 20), (80, 20), 6, 6);
        let ink = output.app.canvas.data().to_vec();
        output.app.apply_theme(crate::ui_theme::Theme {
            preference: crate::ui_theme::Preference::Dark, system_dark: false,
            ..Default::default()
        });
        desktop.synchronize("projector");
        assert!(desktop.outputs.values().all(|o| o.app.theme.dark()));
        assert_eq!(desktop.outputs["projector"].app.canvas.data(), ink);
        assert!(desktop.outputs["laptop"].app.canvas.data().iter().all(|&b| b == 0));
        // A new output takes the saved preference through the same apply path.
        let (rt, app, _) = crate::navigation_tests::fixture(true);
        let mut output = Output { rt, app, ip: Ip::default(), timers: Timers::new(), launch_pending: false };
        desktop.controls.apply(&mut output);
        assert!(output.app.theme.dark());
    }
}

#[derive(Clone, PartialEq)]
struct Controls {
    mode: i32, saved_mode: i32, collapsed_since: Option<Instant>, whiteboard: bool, whiteboard_bg: i32,
    drawing_tools: bool, saved_drawing_tools: bool,
    page: i32, color: usize, pen: usize, eraser: usize, wps: bool, wps_test: bool, linked_show: bool,
    alpha: u8, scale: f64, fade_toolbar_during_show: bool, toolbar_icons_only: bool,
    document_scroll_enabled: bool,
    sidebar_expand_minutes: u32,
    recent_colors: Vec<u32>,
    threshold: f64, erase_scale: i32,
    theme: crate::ui_theme::Theme,
}
impl Controls {
    fn read(app: &App) -> Self {
        // Guide editing and lasso selection belong to the output that owns them.
        let local_edit = app.whiteboard && (app.select_mode || app.coord.is_some_and(|c| c.editing));
        let mode = if local_edit && app.mode == 0 { 1 } else { app.mode };
        Self { mode, saved_mode: app.saved_mode, collapsed_since: app.sidebar_collapsed_since, whiteboard: app.whiteboard,
            drawing_tools: app.drawing_tools.expanded(), saved_drawing_tools: app.saved_drawing_tools,
            whiteboard_bg: app.whiteboard_bg_index, page: app.current_slide,
            color: app.cur_color, pen: app.cur_pen, eraser: app.cur_eraser,
            wps: app.wps_debug, wps_test: app.wps_test_mode, alpha: app.sidebar_alpha, scale: app.sb_scale,
            fade_toolbar_during_show: app.fade_toolbar_during_show,
            toolbar_icons_only: app.toolbar_icons_only,
            document_scroll_enabled: app.document_scroll_enabled,
            linked_show: app.wps_debug && app.wps_connected && app.wps_real_pos > 0,
            sidebar_expand_minutes: app.sidebar_expand_minutes,
            recent_colors: app.settings_ui.recent.clone(),
            theme: app.theme,
            threshold: app.large_touch_threshold, erase_scale: app.large_erase_scale10 }
    }
    fn apply(&self, output: &mut Output) {
        let a = &mut output.app;
        let mode = self.mode;
        let collapsed = self.collapsed_since.is_some();
        if a.mode != mode || a.whiteboard != self.whiteboard
            || a.drawing_tools.expanded() != self.drawing_tools || a.collapsed != collapsed {
            crate::cancel_input(&output.rt, a, &mut output.ip);
            crate::close_all_popups(&output.rt, a);
        }
        if a.whiteboard != self.whiteboard { a.toggle_whiteboard(); }
        if (self.whiteboard || !self.linked_show) && a.current_slide != self.page {
            a.save_current_page(); a.current_slide = self.page; a.load_page(self.page);
        }
        if a.drawing_tools.expanded() != self.drawing_tools || a.mode != mode || a.collapsed != collapsed {
            let layout = crate::toolbar::DrawerLayout::for_page(self.drawing_tools, !collapsed && a.can_restore_cleared());
            a.drawing_tools.set_layout_at(layout, Instant::now());
        }
        a.sidebar_expand_minutes = self.sidebar_expand_minutes;
        a.set_sidebar_collapsed_since(self.collapsed_since);
        a.saved_drawing_tools = self.saved_drawing_tools;
        if a.mode != mode { a.set_tool_mode(mode); }
        a.saved_mode = self.saved_mode;
        a.whiteboard_bg_index = self.whiteboard_bg;
        a.apply_theme(self.theme);
        a.cur_color = self.color; a.cur_pen = self.pen; a.cur_eraser = self.eraser;
        a.wps_debug = self.wps; a.wps_test_mode = self.wps_test;
        a.sidebar_alpha = self.alpha; a.sb_scale = self.scale;
        a.toolbar_icons_only = self.toolbar_icons_only;
        a.document_scroll_enabled = self.document_scroll_enabled;
        if a.fade_toolbar_during_show != self.fade_toolbar_during_show {
            a.fade_toolbar_during_show = self.fade_toolbar_during_show;
            a.presentation_fade.reset();
        }
        a.settings_ui.recent.clone_from(&self.recent_colors);
        a.large_touch_threshold = self.threshold; a.large_erase_scale10 = self.erase_scale;
        crate::apply_input_shape(&output.rt, a);
        output.rt.redraw_full(a);
    }
}

pub struct Desktop {
    pub outputs: BTreeMap<String, Output>,
    pub wps: Option<WpsBridge>,
    controls: Controls,
    detached: VecDeque<(String, App)>,
    fonts: Fonts,
    icon: Option<Pixmap>,
    icon_big: Option<Pixmap>,
    single: Option<UnixListener>,
    starting: bool,
}

impl Desktop {
    pub fn new(fonts: Fonts, icon: Option<Pixmap>, icon_big: Option<Pixmap>, single: Option<UnixListener>) -> Self {
        let mut app = App::new(Pixmap::new(1, 1).unwrap(), 1920, 1080);
        app::load_settings(&mut app);
        app.update_system_theme(crate::system_theme::dark());
        crate::process_block::set_daemon_enabled(app.process_block_enabled);
        Self { outputs: BTreeMap::new(), wps: None, controls: Controls::read(&app),
            detached: VecDeque::new(), fonts, icon, icon_big, single, starting: true }
    }

    pub fn add(&mut self, id: String, backend: Box<dyn Backend>) {
        if self.outputs.contains_key(&id) || self.outputs.len() >= 16 { return; }
        self.controls.theme.system_dark = crate::system_theme::dark();
        let saved = self.detached.iter().position(|(key, _)| key == &id)
            .and_then(|index| self.detached.remove(index)).map(|(_, app)| app);
        let app = saved.unwrap_or_else(|| App::new(Pixmap::new(1, 1).unwrap(), 1920, 1080));
        let rt = Rt { backend, fonts: self.fonts.clone(), icon: self.icon.clone(),
            icon_big: self.icon_big.clone(), batching: Cell::new(true), dirty: RefCell::new(Vec::new()) };
        let mut output = Output { rt, app, ip: Ip::default(), timers: Timers::new(), launch_pending: self.starting };
        self.controls.apply(&mut output);
        output.app.hotkey_ok = output.rt.backend.grab_hotkey();
        if let Some(bridge) = self.wps.as_ref() {
            let (connected, position) = {
                let state = bridge.shared.lock().unwrap();
                (state.connected, state.real_pos)
            };
            if connected && position > 0 {
                let update = crate::WpsUpdate { connected: true, events: vec![crate::WpsEvent::RealPos(position)] };
                crate::tick_shared(&output.rt, &mut output.app, &mut output.ip, Some(bridge), &update, &mut output.timers, crate::system_theme::dark());
            }
        }
        self.outputs.insert(id, output);
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(mut output) = self.outputs.remove(id) {
            crate::cancel_input(&output.rt, &mut output.app, &mut output.ip);
            output.app.launch.finish();
            output.app.presentation_fade.reset();
            output.app.settings_open = false;
            output.app.settings_ui.dialog = None;
            output.app.settings_ui.picker = None;
            output.app.settings_ui.editor = None;
            output.rt.backend.set_text_input(false);
            output.app.settings_ui.closing_at = 0;
            output.app.settings_ui.overlay_closing_at = 0;
            self.detached.push_back((id.to_string(), output.app));
            // Keep a bounded number of disconnected displays for projector reconnection.
            while self.detached.len() > 2 { self.detached.pop_front(); }
        }
    }

    pub fn configure(&mut self, id: &str, w: i32, h: i32, scale: f64) {
        if !app::valid_geometry(w, h, scale) {
            log::error!("拒绝无效输出尺寸: {id} {w}x{h}@{scale}"); return;
        }
        if let Some(output) = self.outputs.get_mut(id) {
            if (output.app.screen_w, output.app.screen_h, output.app.scale) != (w, h, scale) {
                crate::cancel_input(&output.rt, &mut output.app, &mut output.ip);
                output.app.configure_geometry(w, h, scale);
            } else {
                output.app.resize_screen(w, h); // Repair the initial 1x1 placeholder.
            }
            // Only initial outputs get the introduction. Resizing or reconnecting
            // a projector later must not replay it or hide existing annotations.
            let first_frame = std::mem::take(&mut output.launch_pending);
            if first_frame { crate::begin_launch(&output.rt, &mut output.app); }
            crate::apply_input_shape(&output.rt, &output.app);
            output.rt.redraw_full(&output.app);
            if first_frame {
                output.rt.flush_dirty(&output.app);
                output.rt.backend.flush();
            }
        }
    }

    fn synchronize(&mut self, id: &str) {
        let Some(source) = self.outputs.get(id) else { return };
        let controls = Controls::read(&source.app);
        if controls != self.controls {
            self.controls = controls;
            for (key, output) in self.outputs.iter_mut() {
                if key != id { self.controls.apply(output); }
            }
        }
        self.resolve_drawing_tools_auto_update();
    }

    fn resolve_drawing_tools_auto_update(&mut self) {
        self.resolve_drawing_tools_at(Instant::now());
    }

    fn resolve_drawing_tools_at(&mut self, now: Instant) {
        if self.outputs.values().any(|o| o.app.waiting_for_drawer(now)) { return; }
        // Consume every request, then decide after all displays have changed
        // page/mode. Ink on any projector also reopens the shared tool drawer.
        let requested = self.outputs.values_mut().fold(None, |pending, output| {
            let request = output.app.drawing_tools.take_auto_update();
            if request == Some(crate::toolbar::AutoUpdate::FollowPage) { request }
            else { pending.or(request) }
        });
        let Some(update) = requested else { return };
        let mut has_ink = false;
        for output in self.outputs.values_mut() {
            let Some(ink) = output.app.drawing_tools_auto_state() else { return };
            has_ink |= ink;
        }
        let Some(expanded) = update.expanded_for(has_ink) else { return };
        self.controls.drawing_tools = expanded;
        for output in self.outputs.values_mut() {
            crate::set_drawing_tools_expanded(&output.rt, &mut output.app, expanded);
        }
    }

    pub fn motion(&mut self, id: &str, x: i32, y: i32) {
        if let Some(o) = self.outputs.get_mut(id) { crate::on_motion(&o.rt, &mut o.app, &mut o.ip, x, y); }
        self.synchronize(id);
    }
    pub fn scroll(&mut self, id: &str, x: i32, y: i32, pages: f64) {
        if let Some(o) = self.outputs.get_mut(id) { crate::on_scroll(&o.rt, &mut o.app, &o.ip, x, y, pages); }
    }
    pub fn press(&mut self, id: &str, x: i32, y: i32, button: u8) {
        if let Some(o) = self.outputs.get_mut(id) { crate::on_press(&o.rt, &mut o.app, &mut o.ip, self.wps.as_ref(), x, y, button); }
        self.synchronize(id);
    }
    pub fn value_key(&mut self, id: &str, key: crate::value_input::Key) {
        if let Some(o) = self.outputs.get_mut(id) { crate::text_input_key(&o.rt, &mut o.app, key); }
        self.synchronize(id);
    }
    pub fn release(&mut self, id: &str, x: i32, y: i32, button: u8) {
        if let Some(o) = self.outputs.get_mut(id) { crate::on_release(&o.rt, &mut o.app, &mut o.ip, self.wps.as_ref(), x, y, button); }
        self.synchronize(id);
    }
    pub fn touch(&mut self, id: &str, touch: i32, x: i32, y: i32, kind: TouchKind, diameter: f64) {
        if let Some(o) = self.outputs.get_mut(id) {
            crate::handle_touch(&o.rt, &mut o.app, &mut o.ip, self.wps.as_ref(), touch, x, y, kind, diameter);
        }
        self.synchronize(id);
    }
    pub fn cancel(&mut self, id: &str) {
        if let Some(o) = self.outputs.get_mut(id) { crate::cancel_input(&o.rt, &mut o.app, &mut o.ip); }
    }
    pub fn leave(&mut self, id: &str) {
        if let Some(o) = self.outputs.get_mut(id) {
            if o.ip.pending_ui.is_some() { crate::on_motion(&o.rt, &mut o.app, &mut o.ip, -1, -1); }
        }
    }
    pub fn toggle(&mut self) {
        if self.outputs.values().any(|o| o.app.launch.active()) { return; }
        if self.controls.whiteboard { return; }
        self.controls.mode = if self.controls.mode == 0 { 1 } else { 0 };
        if self.controls.mode != 0 {
            self.controls.collapsed_since = None;
            self.controls.drawing_tools = true;
        }
        for output in self.outputs.values_mut() {
            self.controls.apply(output);
            output.app.request_drawing_tools_collapse();
        }
        self.resolve_drawing_tools_auto_update();
    }
    pub(crate) fn settings_animating(&self) -> bool {
        self.outputs.values().any(|output| output.app.settings_ui.animating())
    }

    pub fn tick(&mut self) {
        if self.outputs.values().any(|o| !o.launch_pending && o.app.canvas.width() > 1) { self.starting = false; }
        let window_inputs: Vec<_> = self.outputs.iter_mut()
            .filter_map(|(id, output)| {
                crate::settings_window::poll(&output.rt, &mut output.app, &mut output.ip, self.wps.as_ref())
                    .then(|| id.clone())
            })
            .collect();
        for id in window_inputs {
            self.synchronize(&id);
        }
        let update = crate::poll_wps(self.controls.wps, self.controls.whiteboard, self.controls.wps_test, &mut self.wps);
        for output in self.outputs.values_mut() {
            crate::tick_shared(&output.rt, &mut output.app, &mut output.ip, self.wps.as_ref(), &update, &mut output.timers, crate::system_theme::dark());
        }
        if let Some(output) = self.outputs.values().next() { self.controls = Controls::read(&output.app); }
        self.resolve_drawing_tools_auto_update();
        for output in self.outputs.values_mut() {
            output.rt.flush_dirty(&output.app);
            output.rt.backend.flush();
        }
        while let Some(action) = crate::tray::poll() {
            match action {
                crate::tray::TrayAction::Settings => {
                    if let Some(output) = self.outputs.values_mut().next() {
                        crate::open_settings(&output.rt, &mut output.app);
                    }
                }
                crate::tray::TrayAction::Toggle => self.toggle(),
                crate::tray::TrayAction::Whiteboard => {
                    let id = self.outputs.keys().next().cloned();
                    if let Some(id) = id {
                        if let Some(output) = self.outputs.get_mut(&id) {
                            crate::handle_btn(&output.rt, &mut output.app, self.wps.as_ref(),
                                crate::ui::Btn::Whiteboard, false);
                        }
                        self.synchronize(&id);
                    }
                }
                crate::tray::TrayAction::Show => (),
                crate::tray::TrayAction::Quit => {
                    crate::process_block::stop_daemon();
                    std::process::exit(0);
                }
            }
        }
        let mut toggle = false;
        if let Some(listener) = self.single.as_ref() {
            while let Ok((mut client, _)) = listener.accept() {
                let _ = client.set_read_timeout(Some(Duration::from_millis(20)));
                let mut data = [0; 16];
                if let Ok(n) = client.read(&mut data) { toggle |= &data[..n] == b"toggle"; }
                for output in self.outputs.values_mut() { output.rt.redraw_full(&output.app); }
            }
        }
        if toggle { self.toggle(); }
    }
}
