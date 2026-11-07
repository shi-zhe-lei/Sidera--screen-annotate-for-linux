//! Common multi-output controller for native Wayland and the GNOME extension.
//! Each output owns its canvas and caches; controls and WPS events are shared.
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::io::Read;
use crate::instance::Instance as UnixListener;
use std::time::Duration;
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
        assert!(desktop.outputs.values().all(|o| o.app.current_slide == 2 && !o.app.drawing_tools.expanded()));
        assert!(!desktop.controls.drawing_tools);
        desktop.motion("projector", 50, 50);
        let laptop = desktop.outputs.get_mut("laptop").unwrap();
        crate::go_prev(&laptop.rt, &mut laptop.app, None);
        desktop.synchronize("laptop");
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
            assert!(desktop.controls.collapsed && !desktop.controls.drawing_tools);
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
    mode: i32, saved_mode: i32, collapsed: bool, whiteboard: bool, whiteboard_bg: i32,
    drawing_tools: bool, saved_drawing_tools: bool,
    page: i32, color: usize, pen: usize, eraser: usize, wps: bool, wps_test: bool, linked_show: bool,
    alpha: u8, scale: f64, right_click: bool,
    sidebar_expand_minutes: u32,
    recent_colors: Vec<u32>,
    threshold: f64, erase_scale: i32,
    theme: crate::ui_theme::Theme,
}
impl Controls {
    fn read(app: &App) -> Self {
        Self { mode: app.mode, saved_mode: app.saved_mode, collapsed: app.collapsed, whiteboard: app.whiteboard,
            drawing_tools: app.drawing_tools.expanded(), saved_drawing_tools: app.saved_drawing_tools,
            whiteboard_bg: app.whiteboard_bg_index, page: app.current_slide,
            color: app.cur_color, pen: app.cur_pen, eraser: app.cur_eraser,
            wps: app.wps_debug, wps_test: app.wps_test_mode, alpha: app.sidebar_alpha, scale: app.sb_scale,
            linked_show: app.wps_debug && app.wps_connected && app.wps_real_pos > 0,
            right_click: app.right_click_cursor_on,
            sidebar_expand_minutes: app.sidebar_expand_minutes,
            recent_colors: app.settings_ui.recent.clone(),
            theme: app.theme,
            threshold: app.large_touch_threshold, erase_scale: app.large_erase_scale10 }
    }
    fn apply(&self, output: &mut Output) {
        let a = &mut output.app;
        let mode = if self.whiteboard && self.mode == 0 { 1 } else { self.mode };
        if a.mode != mode || a.whiteboard != self.whiteboard
            || a.drawing_tools.expanded() != self.drawing_tools || a.collapsed != self.collapsed {
            crate::cancel_input(&output.rt, a, &mut output.ip);
            crate::close_all_popups(&output.rt, a);
        }
        if a.whiteboard != self.whiteboard { a.toggle_whiteboard(); }
        if (self.whiteboard || !self.linked_show) && a.current_slide != self.page {
            a.save_current_page(); a.current_slide = self.page; a.load_page(self.page);
        }
        a.set_sidebar_collapsed(self.collapsed);
        a.drawing_tools.set_expanded(self.drawing_tools);
        a.saved_drawing_tools = self.saved_drawing_tools;
        a.mode = mode; a.saved_mode = self.saved_mode;
        a.whiteboard_bg_index = self.whiteboard_bg;
        a.apply_theme(self.theme);
        a.cur_color = self.color; a.cur_pen = self.pen; a.cur_eraser = self.eraser;
        a.wps_debug = self.wps; a.wps_test_mode = self.wps_test;
        a.sidebar_alpha = self.alpha; a.sb_scale = self.scale;
        a.sidebar_expand_minutes = self.sidebar_expand_minutes;
        a.settings_ui.recent.clone_from(&self.recent_colors);
        a.right_click_cursor_on = self.right_click;
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
            self.controls.collapsed = false;
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
