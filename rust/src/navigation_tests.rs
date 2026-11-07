use super::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct TestBackend { keys: Rc<Cell<usize>>, direct_keys: bool, foreground: Option<bool>, input: RefCell<Vec<IRect>> }
impl Backend for TestBackend {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn screen_size(&self) -> (i32, i32) { (100, 100) }
    fn present(&self, _: &Pixmap, _: i32, _: i32) {}
    fn set_input_region(&self, rects: &[IRect], _: bool) { *self.input.borrow_mut() = rects.to_vec(); }
    fn fake_key(&self, _: Key) { self.keys.set(self.keys.get() + 1); }
    fn start_presentation(&self) -> bool { self.keys.set(self.keys.get() + 1); true }
    fn can_inject_keys_without_prompt(&self) -> bool { self.direct_keys }
    fn virtual_right_click(&self, _: i32, _: i32) {}
    fn screenshot(&self) -> Option<(Vec<u8>, u32, u32)> { None }
    fn is_presentation_fullscreen(&self) -> bool { false }
    fn presentation_foreground(&self) -> Option<bool> { self.foreground }
    fn has_compositor(&self) -> bool { true }
}

pub(crate) fn fixture(direct_keys: bool) -> (Rt, App, Rc<Cell<usize>>) {
    fixture_with_foreground(direct_keys, None)
}

fn fixture_with_foreground(direct_keys: bool, foreground: Option<bool>) -> (Rt, App, Rc<Cell<usize>>) {
    let keys = Rc::new(Cell::new(0));
    let rt = Rt {
        backend: Box::new(TestBackend { keys: keys.clone(), direct_keys, foreground, input: RefCell::new(Vec::new()) }),
        fonts: Fonts::load(), icon: None, icon_big: None,
        batching: Cell::new(true), dirty: RefCell::new(Vec::new()),
    };
    let mut app = App::new(Pixmap::new(100, 100).unwrap(), 100, 100);
    app.wps_test_mode = true;
    (rt, app, keys)
}

#[test]
fn small_settings_page_buttons_wheel_and_done_use_the_real_input_path() {
    let (rt, mut app, keys) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.mode = 1;
    let mut ip = Ip::default();
    open_settings(&rt, &mut app);
    let panel = ui::settings_rect(&app);
    let next = ui::settings_controls(&app).into_iter()
        .find(|(h, _)| *h == SetHit::SettingsPage(1)).unwrap().1;
    let (x, y) = (next.x + next.w / 2, next.y + next.h / 2);
    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
    assert_eq!(app.settings_page, 0, "activate only after release");
    on_scroll(&rt, &mut app, &ip, x, y, 1.0);
    assert_eq!(app.settings_page, 0, "do not move a pressed target");
    on_release(&rt, &mut app, &mut ip, None, x, y, 1);
    assert_eq!(app.settings_page, 1);
    on_scroll(&rt, &mut app, &ip, 0, 0, 1.0);
    on_scroll(&rt, &mut app, &ip, x, y, f64::NAN);
    assert_eq!(app.settings_page, 1);
    on_press(&rt, &mut app, &mut ip, None, panel.x + 40, panel.y + 30, 4);
    assert_eq!(app.settings_page, 0);
    let done = ui::settings_controls(&app).into_iter().find(|(h, _)| *h == SetHit::Done).unwrap().1;
    let (x, y) = (done.x + done.w / 2, done.y + done.h / 2);
    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
    on_release(&rt, &mut app, &mut ip, None, x, y, 1);
    assert!(app.settings_ui.closing_at > 0);
    app.settings_ui.closing_at -= 121;
    tick_shared(&rt, &mut app, &mut ip, None,
        &WpsUpdate { connected: false, events: Vec::new() }, &mut Timers::new(), false);
    assert!(!app.settings_open);
    assert!(!app.is_drawing && !app.page_has_ink);
    assert!(ip.pending_ui.is_none() && ip.slider.is_none());
    assert_eq!(keys.get(), 0);
}

#[test]
fn disconnected_link_uses_available_direct_keyboard() {
    for direct_keys in [false, true] {
        let (rt, mut app, keys) = fixture(direct_keys);
        app.current_slide = 4;
        go_prev(&rt, &mut app, None);
        assert_eq!(app.current_slide, if direct_keys { 3 } else { 4 });
        go_next(&rt, &mut app, None);
        exit_presentation(&rt, &mut app, None);
        assert_eq!(keys.get(), if direct_keys { 3 } else { 0 });
        assert_eq!(app.current_slide, 4);
        assert_eq!(app.wps_navigation_ready, direct_keys);
    }
}

#[test]
fn disabling_link_does_not_enable_xwayland_portal_input() {
    for direct_keys in [false, true] {
        let (rt, mut app, keys) = fixture(direct_keys);
        app.wps_debug = false;
        go_prev(&rt, &mut app, None);
        go_next(&rt, &mut app, None);
        exit_presentation(&rt, &mut app, None);
        assert_eq!(keys.get(), if direct_keys { 3 } else { 0 });
    }
}

#[test]
fn background_wps_does_not_enter_presentation_or_switch_pages() {
    let (rt, mut app, _) = fixture_with_foreground(false, Some(false));
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    {
        let mut s = bridge.shared.lock().unwrap();
        s.connected = true; s.last_seen = now_ms(); s.real_pos = 3;
        s.page_index = 3; s.page_total = 10; s.can_jump = true; s.deck_epoch = 7;
    }
    let update = WpsUpdate { connected: true, events: vec![WpsEvent::RealPos(3)] };
    tick_shared(&rt, &mut app, &mut Ip::default(), Some(&bridge), &update, &mut Timers::new(), false);
    assert_eq!(app.current_slide, 1, "background page changes must not replace the canvas");
    assert!(!app.wps_can_jump);
    assert!(!app.sidebar_page_visible());
    assert_eq!(app.presentation_action, ui::PresentationAction::Escape);
}

#[test]
fn background_wps_rejects_toolbar_actions_even_before_the_next_tick() {
    let (rt, mut app, keys) = fixture_with_foreground(true, Some(false));
    app.wps_debug = true;
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    {
        let mut state = bridge.shared.lock().unwrap();
        state.connected = true;
        state.last_seen = now_ms();
        state.real_pos = 3;
        state.test_mode = true;
        state.can_jump = true;
        state.page_index = 3;
        state.page_total = 10;
        state.deck_epoch = 7;
    }
    app.wps_connected = true;
    app.wps_navigation_ready = true;
    go_prev(&rt, &mut app, Some(&bridge));
    go_next(&rt, &mut app, Some(&bridge));
    exit_presentation(&rt, &mut app, Some(&bridge));
    app.wps_can_jump = true;
    app.wps_deck_epoch = 7;
    app.wps_page_index = 3;
    app.wps_page_total = 10;
    let mut picker = page_picker::PagePicker::new(3, 10, page_picker::Source::Wps(7));
    picker.set_selected(5);
    app.page_picker = Some(picker);
    commit_page_picker(&rt, &mut app, Some(&bridge));
    assert!(!app.wps_navigation_ready);
    assert_eq!(app.current_slide, 1);
    assert_eq!(keys.get(), 0);
    assert!(bridge.shared.lock().unwrap().queue.is_empty());
}

#[test]
fn esc_control_does_not_start_a_slideshow_from_a_background_wps_window() {
    let (rt, mut app, keys) = fixture_with_foreground(false, Some(false));
    app.wps_debug = true;
    app.wps_test_mode = false;
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    {
        let mut s = bridge.shared.lock().unwrap();
        s.connected = true;
        s.last_seen = now_ms();
        s.real_pos = -1;
        s.can_start = true;
    }
    exit_presentation(&rt, &mut app, Some(&bridge));
    assert_eq!(keys.get(), 0, "Esc must not inject Shift+F5 when WPS is behind another window");
    assert!(bridge.shared.lock().unwrap().queue.is_empty());
}

#[test]
fn default_wps_mode_uses_shift_f5_to_start_and_escape_to_exit() {
    let (rt, mut app, keys) = fixture(false);
    app.wps_test_mode = false;
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    {
        let mut s = bridge.shared.lock().unwrap();
        s.connected = true; s.last_seen = now_ms(); s.real_pos = -1;
    }
    exit_presentation(&rt, &mut app, Some(&bridge));
    assert_eq!(keys.get(), 1, "default start must inject Shift+F5");
    {
        let mut s = bridge.shared.lock().unwrap();
        s.real_pos = 3;
    }
    exit_presentation(&rt, &mut app, Some(&bridge));
    assert_eq!(keys.get(), 2, "default exit must inject Escape");
}

#[test]
fn linked_navigation_queues_wps_commands_and_waits_for_real_page_state() {
    let (rt, mut app, keys) = fixture(false);
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    {
        let mut s = bridge.shared.lock().unwrap();
        s.connected = true; s.last_seen = now_ms(); s.real_pos = 4;
    }
    app.current_slide = 4;
    go_next(&rt, &mut app, Some(&bridge));
    go_prev(&rt, &mut app, Some(&bridge));
    exit_presentation(&rt, &mut app, Some(&bridge));
    assert_eq!(keys.get(), 0);
    assert_eq!(app.current_slide, 4);
    assert_eq!(bridge.shared.lock().unwrap().queue.iter().map(|(_, c)| c.as_str()).collect::<Vec<_>>(),
               ["NEXT", "PREV", "EXIT"]);
    bridge.shared.lock().unwrap().last_seen -= 4000;
    go_next(&rt, &mut app, Some(&bridge));
    assert_eq!(keys.get(), 0);
    assert_eq!(bridge.shared.lock().unwrap().queue.len(), 3);
}

#[test]
fn whiteboard_pages_remain_local() {
    let (rt, mut app, keys) = fixture(false);
    app.whiteboard = true;
    go_next(&rt, &mut app, None);
    assert_eq!(app.current_slide, 2);
    go_prev(&rt, &mut app, None);
    assert_eq!(app.current_slide, 1);
    exit_presentation(&rt, &mut app, None);
    assert_eq!(keys.get(), 0);
}

#[test]
fn presentation_lifecycle_does_not_erase_visible_whiteboard() {
    for (event, overlay_page) in [(WpsEvent::SlideshowBegin(4), 4), (WpsEvent::SlideshowEnd, 1)] {
        let (rt, mut app, keys) = fixture(false);
        app.current_slide = 7;
        paint::stroke_tapered(&mut app, (20, 20), (80, 20), 6, 6);
        app.toggle_whiteboard();
        paint::stroke_tapered(&mut app, (20, 40), (80, 40), 6, 6);
        go_next(&rt, &mut app, None);
        paint::stroke_tapered(&mut app, (20, 60), (80, 60), 6, 6);
        let second = app.canvas.data().to_vec();
        let mut ip = Ip::default();
        let mut timers = Timers::new();
        let update = WpsUpdate { connected: false, events: vec![event] };
        tick_shared(&rt, &mut app, &mut ip, None, &update, &mut timers, false);
        assert!(app.whiteboard);
        assert_eq!(app.current_slide, 2);
        assert_eq!(app.canvas.data(), second.as_slice());
        // A manual mode switch now clears the whiteboard but keeps the overlay.
        app.toggle_whiteboard();
        assert_eq!(app.current_slide, overlay_page);
        assert!(!app.page_has_ink);
        app.toggle_whiteboard();
        assert!(app.whiteboard);
        assert_eq!(app.current_slide, 1);
        assert!(app.canvas.data().iter().all(|&b| b == 0));
        assert_eq!(keys.get(), 0);
    }
}

#[test]
fn presentation_page_changes_behind_whiteboard_update_the_overlay_on_mode_switch() {
    let (rt, mut app, keys) = fixture(false);
    app.current_slide = 2;
    app.wps_real_pos = 2;
    app.toggle_whiteboard();
    paint::stroke_tapered(&mut app, (20, 40), (80, 40), 6, 6);
    let whiteboard = app.canvas.data().to_vec();
    let update = WpsUpdate { connected: true, events: vec![WpsEvent::RealPos(7)] };
    tick_shared(&rt, &mut app, &mut Ip::default(), None, &update, &mut Timers::new(), false);
    assert_eq!(app.canvas.data(), whiteboard.as_slice());
    assert_eq!(app.current_slide, 1);
    app.toggle_whiteboard();
    assert_eq!(app.current_slide, 7);
    assert!(!app.whiteboard);
    app.toggle_whiteboard();
    assert_eq!(app.current_slide, 1);
    assert!(app.canvas.data().iter().all(|&b| b == 0));
    assert_eq!(keys.get(), 0);
}

#[test]
fn linked_failure_does_not_double_advance_through_keyboard_fallback() {
    let (rt, mut app, keys) = fixture(true);
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    {
        let mut s = bridge.shared.lock().unwrap();
        s.connected = true; s.last_seen = now_ms(); s.real_pos = 4;
        s.command_failed_until = now_ms() + 3000;
    }
    app.current_slide = 4;
    go_next(&rt, &mut app, Some(&bridge));
    assert_eq!(keys.get(), 0);
    assert_eq!(app.current_slide, 4);
    assert!(bridge.shared.lock().unwrap().queue.is_empty());
    bridge.shared.lock().unwrap().last_seen -= 4000;
    go_next(&rt, &mut app, Some(&bridge));
    assert_eq!(keys.get(), 1);
    assert_eq!(app.current_slide, 5);
}

#[test]
fn presentation_button_tracks_ready_showing_and_disconnected_states() {
    let (rt, mut app, keys) = fixture(true);
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    {
        let mut s = bridge.shared.lock().unwrap();
        s.connected = true; s.last_seen = now_ms(); s.can_start = true;
    }
    let mut ip = Ip::default(); let mut timers = Timers::new();
    let update = WpsUpdate { connected: true, events: vec![] };
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &update, &mut timers, false);
    assert_eq!(app.presentation_action, ui::PresentationAction::Start);
    // Presentation colours are animating, but the buttons remain available.
    assert!(!ui::sidebar_animating(&app));
    let bounds = [false, true].map(|right| {
        let r = ui::control_rect(&app, ui::Control::Sidebar(Btn::Exit, right)).unwrap();
        assert_eq!(ui::sidebar_button_at(&app, r.x + r.w / 2, r.y + r.h / 2), Some((Btn::Exit, right)));
        r
    });
    exit_presentation(&rt, &mut app, Some(&bridge));
    exit_presentation(&rt, &mut app, Some(&bridge));
    assert_eq!(keys.get(), 0);
    assert_eq!(bridge.shared.lock().unwrap().queue.len(), 1);
    assert_eq!(bridge.shared.lock().unwrap().queue[0].1, "START");
    {
        let mut s = bridge.shared.lock().unwrap(); s.can_start = false; s.real_pos = 1;
    }
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &update, &mut timers, false);
    assert_eq!(app.presentation_action, ui::PresentationAction::End);
    bridge.shared.lock().unwrap().last_seen -= 4000;
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &update, &mut timers, false);
    assert_eq!(app.presentation_action, ui::PresentationAction::Escape);
    for (right, before) in [false, true].into_iter().zip(bounds) {
        assert_eq!(ui::control_rect(&app, ui::Control::Sidebar(Btn::Exit, right)), Some(before));
    }
}

fn ui_fixture() -> (Rt, App, Ip) {
    let (rt, mut app, _) = fixture(false);
    app.configure_geometry(1280, 900, 1.0);
    app.mode = 0; app.collapsed = false;
    (rt, app, Ip::default())
}

#[test]
fn page_wheel_previews_without_changing_whiteboard_and_commits_once() {
    for right in [false, true] {
        let (rt, mut app, mut ip) = ui_fixture();
        app.toggle_whiteboard();
        app.drawing_tools.settle();
        app.canvas.data_mut()[0..4].copy_from_slice(&[255, 0, 0, 255]);
        app.page_has_ink = true;
        let count = ui::sidebar_page_rect(&app, right);
        let (x, y) = (count.x + count.w / 2, count.y + count.h / 2);
        on_press(&rt, &mut app, &mut ip, None, x, y, 1);
        assert!(app.page_picker.is_none(), "the counter activates on release");
        on_release(&rt, &mut app, &mut ip, None, x, y, 1);
        assert!(app.page_picker.is_some());
        assert!(!ip.dragging, "the counter must not drag the toolbar");
        let wheel = ui::page_picker_control_rect(&app, ui::PagePickerHit::Wheel);
        on_scroll(&rt, &mut app, &ip, wheel.x + 30, wheel.y + 110, 2.0);
        assert_eq!(app.current_slide, 1);
        assert_eq!(app.canvas.data()[3], 255);
        let jump = ui::page_picker_control_rect(&app, ui::PagePickerHit::Jump);
        let (x, y) = (jump.x + 20, jump.y + 20);
        on_press(&rt, &mut app, &mut ip, None, x, y, 1);
        on_release(&rt, &mut app, &mut ip, None, x, y, 1);
        assert_eq!(app.current_slide, 3);
        assert!(app.page_picker.is_none());
        assert_eq!(app.whiteboard_cache.get(&1).unwrap().data()[3], 255);
        assert_eq!(app.canvas.data()[3], 0);
    }
}

#[test]
fn page_wheel_owns_touch_until_release_and_cancels_cleanly() {
    let (rt, mut app, mut ip) = ui_fixture();
    app.toggle_whiteboard(); app.drawing_tools.settle();
    open_page_picker(&rt, &mut app, false);
    let wheel = ui::page_picker_control_rect(&app, ui::PagePickerHit::Wheel);
    let (x, y) = (wheel.x + 50, wheel.y + wheel.h / 2);
    handle_touch(&rt, &mut app, &mut ip, None, 1, x, y, TouchKind::Begin, 1.0);
    handle_touch(&rt, &mut app, &mut ip, None, 2, x, y, TouchKind::Begin, 1.0);
    handle_touch(&rt, &mut app, &mut ip, None, 2, x, y, TouchKind::End, 0.0);
    assert!(app.page_picker.as_ref().unwrap().dragging());
    handle_touch(&rt, &mut app, &mut ip, None, 1, x + 500, y - 70, TouchKind::Update, 1.0);
    on_release(&rt, &mut app, &mut ip, None, x, y, 3);
    assert!(app.page_picker.as_ref().unwrap().dragging());
    handle_touch(&rt, &mut app, &mut ip, None, 1, x + 500, y - 70, TouchKind::End, 0.0);
    assert!(!app.page_picker.as_ref().unwrap().dragging());
    assert_eq!(app.current_slide, 1);
    cancel_input(&rt, &mut app, &mut ip);
    close_all_popups(&rt, &mut app);
    assert!(!over_ui(&app, x, y));
}

#[test]
fn stale_wps_document_closes_the_wheel_without_sending_a_jump() {
    let (rt, mut app, mut ip) = ui_fixture();
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    {
        let mut s = bridge.shared.lock().unwrap();
        s.connected = true; s.last_seen = now_ms(); s.page_index = 3; s.page_total = 20;
        s.can_jump = true; s.deck_epoch = 10;
    }
    let update = WpsUpdate { connected: true, events: vec![] };
    let mut timers = Timers::new();
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &update, &mut timers, false);
    open_page_picker(&rt, &mut app, true);
    assert!(app.page_picker.is_some());
    bridge.shared.lock().unwrap().deck_epoch = 11;
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &update, &mut timers, false);
    assert!(app.page_picker.is_none());
    assert!(bridge.shared.lock().unwrap().queue.is_empty());
}

fn button_center(app: &App, btn: Btn) -> (i32, i32) {
    let r = ui::sidebar_buttons(app, false).into_iter().find(|(b, _)| *b == btn).unwrap().1;
    (r.x + r.w / 2, r.y + r.h / 2)
}

#[test]
fn both_clear_controls_keep_restore_reachable_and_whiteboard_ready() {
    for whiteboard in [false, true] {
        for popup_clear in [false, true] {
            let (rt, mut app, mut ip) = ui_fixture();
            assert!(!app.drawing_tools.expanded());
            assert!(ui::sidebar_buttons(&app, false).iter().any(|(b, _)| *b == Btn::Pen));
            assert!(!ui::sidebar_buttons(&app, false).iter().any(|(b, _)| *b == Btn::Clear));
            if whiteboard { handle_btn(&rt, &mut app, None, Btn::Whiteboard, false); }
            else { handle_btn(&rt, &mut app, None, Btn::Pen, false); }
            assert_eq!(app.mode, 1);
            assert!(app.drawing_tools.expanded());
            assert!(!app.pen_popup_visible, "first pen selection must not also open its settings");
            app.drawing_tools.settle();
            paint::stroke_tapered(&mut app, (640, 200), (710, 200), 6, 6);
            app.save_current_page();
            if popup_clear {
                handle_btn(&rt, &mut app, None, Btn::Eraser, false);
                handle_btn(&rt, &mut app, None, Btn::Eraser, false);
                let r = ui::control_rect(&app, ui::Control::Popup(PopupHit::ClearAll)).unwrap();
                on_press(&rt, &mut app, &mut ip, None, r.x + r.w / 2, r.y + r.h / 2, 1);
                on_release(&rt, &mut app, &mut ip, None, r.x + r.w / 2, r.y + r.h / 2, 1);
            } else { handle_btn(&rt, &mut app, None, Btn::Clear, false); }
            assert!(!app.page_has_ink && !app.is_drawing);
            assert!(app.can_restore_cleared(), "clearing must leave one restore snapshot");
            resolve_drawing_tools_auto_update(&rt, &mut app);
            assert!(app.drawing_tools.expanded(), "restore must stay reachable after clearing");
            assert!(!app.pen_popup_visible && !app.eraser_popup_visible);
            assert_eq!(app.whiteboard, whiteboard);
            assert_eq!(app.mode, if whiteboard { 1 } else { 0 });
            app.drawing_tools.settle();
            assert!(ui::sidebar_buttons(&app, false).iter().any(|(b, _)| *b == Btn::Eraser),
                "the clear restore action lives beside eraser/undo");
            assert!(ui::sidebar_buttons(&app, false).iter().any(|(b, _)| *b == Btn::Next));
            app.load_page(app.current_slide);
            assert!(!app.page_has_ink, "cleared content must not return from its cache");
            if whiteboard {
                go_next(&rt, &mut app, None);
                resolve_drawing_tools_auto_update(&rt, &mut app);
                assert!(app.drawing_tools.expanded(), "blank whiteboard pages remain ready to write");
            } else {
                handle_btn(&rt, &mut app, None, Btn::Pen, false);
                assert!(app.drawing_tools.expanded());
                assert!(!app.pen_popup_visible, "a folded active pen opens its tool group first");
            }
        }
    }
}

#[test]
fn drawer_expansion_stays_on_screen_and_preserves_its_preferred_anchor() {
    for scale in [0.6, 1.0, 1.4] {
        let (rt, mut app, mut ip) = ui_fixture();
        app.sb_scale = scale;
        app.sb_y = app.screen_h - app.sb_height();
        let compact = ui::sidebar_rect(&app, false);
        let start = Instant::now();
        app.drawing_tools.set_expanded_at(true, start);
        for ms in [0, 40, 80, 120, 180, 220] {
            app.drawing_tools.advance_at(start + Duration::from_millis(ms));
            for right in [false, true] {
                let sb = ui::sidebar_rect(&app, right);
                assert_eq!(app.sb_y, compact.y, "keep the preferred anchor for shrinking again");
                assert!(sb.y >= 0 && sb.y <= compact.y);
                assert_eq!(sb.y + sb.h, app.screen_h, "expanding tools must rise into the screen");
                let region = ui::sidebar_input_rect(&app, right);
                assert!(region.y <= sb.y && region.y + region.h >= sb.y + sb.h);
                for (_, r) in ui::sidebar_buttons(&app, right) {
                    assert!(r.y >= sb.y && r.y + r.h <= sb.y + sb.h);
                }
                let pages = ui::sidebar_page_rect(&app, right);
                assert!(pages.y >= sb.y && pages.y + pages.h <= sb.y + sb.h);
            }
            if app.drawing_tools.animating() {
                let (x, y) = button_center(&app, Btn::Next);
                on_press(&rt, &mut app, &mut ip, None, x, y, 1);
                assert!(ip.pending_ui.is_none() && !ip.dragging && !app.is_drawing);
            }
        }
        let open = ui::sidebar_rect(&app, false);
        app.drawing_tools.set_expanded_at(false, start + Duration::from_millis(250));
        app.drawing_tools.advance_at(start + Duration::from_millis(470));
        let closed = ui::sidebar_rect(&app, false);
        assert_eq!(closed, compact);
        assert!(open.h > closed.h);
        assert_eq!(ui::sidebar_input_rect(&app, false), closed);
    }
}

#[test]
fn cursor_collapses_only_when_no_actual_ink_remains() {
    for remaining in ["blank", "ink", "erased", "undone", "pending"] {
        let (rt, mut app, _) = fixture(false);
        switch_to_draw(&rt, &mut app, 1);
        if matches!(remaining, "ink" | "erased" | "undone") {
            app.begin_undo();
            paint::stroke_tapered(&mut app, (20, 20), (80, 20), 6, 6);
        }
        if remaining == "erased" {
            paint::stroke_segment(&mut app, (10, 20), (90, 20), true, 40);
        } else if remaining == "undone" {
            app.undo_last();
        } else if remaining == "pending" {
            app.is_drawing = true;
            app.stroke_pts = vec![(20.0, 20.0, 6.0), (80.0, 20.0, 6.0)];
        }
        switch_to_cursor(&rt, &mut app);
        resolve_drawing_tools_auto_update(&rt, &mut app);
        assert_eq!(app.drawing_tools.expanded(), matches!(remaining, "ink" | "pending"), "{remaining}");
        assert_eq!(app.mode, 0);
        assert!(!app.is_drawing);
    }
}

#[test]
fn paging_checks_destination_ink_and_preserves_active_drawing_modes() {
    for mode in [0, 1, 2] {
        for target_ink in [false, true] {
            let (rt, mut app, _) = fixture(true);
            switch_to_draw(&rt, &mut app, 1);
            paint::stroke_tapered(&mut app, (20, 20), (80, 20), 6, 6);
            let target = if target_ink { app.canvas.clone() } else { Pixmap::new(100, 100).unwrap() };
            app.slide_cache.insert(2, target); // Empty caches must not be mistaken for ink.
            if mode == 0 { switch_to_cursor(&rt, &mut app); }
            else { switch_to_draw(&rt, &mut app, mode); }
            resolve_drawing_tools_auto_update(&rt, &mut app);
            assert!(app.drawing_tools.expanded());
            go_next(&rt, &mut app, None);
            resolve_drawing_tools_auto_update(&rt, &mut app);
            assert_eq!(app.current_slide, 2);
            assert_eq!(app.drawing_tools.expanded(), mode != 0 || target_ink);
            go_prev(&rt, &mut app, None);
            resolve_drawing_tools_auto_update(&rt, &mut app);
            assert!(app.canvas.pixels().iter().any(|p| p.alpha() != 0));
            assert!(app.drawing_tools.expanded(), "returning to ink reopens the drawing tools");
            assert_eq!(app.mode, mode, "restoring tools must not change the selected mode");
        }
    }
}

#[test]
fn folded_cursor_tools_follow_actual_ink_without_untucking_the_sidebar() {
    for tucked in [false, true] {
        for content in ["ink", "erased", "empty"] {
            let (rt, mut app, _) = fixture(true);
            if content != "empty" {
                paint::stroke_tapered(&mut app, (20, 20), (80, 20), 6, 6);
                if content == "erased" {
                    paint::stroke_segment(&mut app, (10, 20), (90, 20), true, 40);
                }
            }
            app.slide_cache.insert(2, app.canvas.clone());
            app.clear_canvas();
            if tucked { collapse_sidebars(&rt, &mut app); }
            assert!(!app.drawing_tools.expanded());
            go_next(&rt, &mut app, None);
            resolve_drawing_tools_auto_update(&rt, &mut app);
            assert_eq!(app.current_slide, 2);
            assert_eq!(app.mode, 0);
            assert_eq!(app.collapsed, tucked);
            assert_eq!(app.drawing_tools.expanded(), content == "ink" && !tucked);
            assert_eq!(app.canvas.pixels().iter().any(|p| p.alpha() != 0), content == "ink");
            // Mouse movement/idle checks cannot override the manual whole-bar tuck.
            resolve_drawing_tools_auto_update(&rt, &mut app);
            assert_eq!(app.drawing_tools.expanded(), content == "ink" && !tucked);
        }
    }
}

#[test]
fn wps_waits_for_real_page_before_collapsing_and_whiteboard_restores_tools() {
    let (rt, mut app, _) = fixture(false);
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    {
        let mut s = bridge.shared.lock().unwrap();
        s.connected = true; s.last_seen = now_ms(); s.real_pos = 1;
    }
    app.wps_real_pos = 1;
    switch_to_draw(&rt, &mut app, 1);
    paint::stroke_tapered(&mut app, (20, 20), (80, 20), 6, 6);
    switch_to_cursor(&rt, &mut app);
    resolve_drawing_tools_auto_update(&rt, &mut app);
    app.toggle_whiteboard();
    clear_drawing(&rt, &mut app);
    assert!(app.drawing_tools.expanded());
    app.toggle_whiteboard();
    resolve_drawing_tools_auto_update(&rt, &mut app);
    assert!(app.drawing_tools.expanded(), "restore the pre-whiteboard tools when ink remains");
    go_next(&rt, &mut app, Some(&bridge));
    let mut ip = Ip::default();
    let mut timers = Timers::new();
    timers.last_wps = Instant::now();
    let same_page = WpsUpdate { connected: true, events: vec![WpsEvent::RealPos(1)] };
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &same_page, &mut timers, false);
    resolve_drawing_tools_auto_update(&rt, &mut app);
    assert!(app.drawing_tools.expanded(), "a queued command or animation step is not a page change");
    bridge.shared.lock().unwrap().real_pos = 2;
    let next_page = WpsUpdate { connected: true, events: vec![WpsEvent::RealPos(2)] };
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &next_page, &mut timers, false);
    resolve_drawing_tools_auto_update(&rt, &mut app);
    assert_eq!(app.current_slide, 2);
    assert!(!app.drawing_tools.expanded());
    app.toggle_whiteboard();
    assert!(app.drawing_tools.expanded());
    app.toggle_whiteboard();
    resolve_drawing_tools_auto_update(&rt, &mut app);
    assert!(!app.drawing_tools.expanded());
    go_prev(&rt, &mut app, Some(&bridge));
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &next_page, &mut timers, false);
    resolve_drawing_tools_auto_update(&rt, &mut app);
    assert!(!app.drawing_tools.expanded(), "queued navigation must wait for restored page content");
    bridge.shared.lock().unwrap().real_pos = 1;
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &same_page, &mut timers, false);
    resolve_drawing_tools_auto_update(&rt, &mut app);
    assert_eq!(app.current_slide, 1);
    assert!(app.canvas.pixels().iter().any(|p| p.alpha() != 0));
    assert!(app.drawing_tools.expanded());
    assert_eq!(app.mode, 0);
}

#[test]
fn separate_sidebar_damage_does_not_repaint_the_full_width() {
    let left = R { x: 0, y: 100, w: 60, h: 500 };
    let right = R { x: 1220, ..left };
    let small = R { x: 8, y: 150, w: 34, h: 34 };
    let regions = merge_redraw_regions(vec![left, right, small]);
    assert_eq!(regions.len(), 2);
    assert!(regions.contains(&left) && regions.contains(&right));
    let full = R { x: 0, y: 0, w: 1280, h: 900 };
    assert_eq!(merge_redraw_regions(vec![left, right, full]), vec![full]);
}

#[test]
fn page_indicator_uses_fresh_wps_data_without_changing_local_annotations() {
    let (rt, mut app, mut ip) = ui_fixture();
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    app.current_slide = 8; // Keyboard navigation's estimate must never appear as document metadata.
    assert_eq!(ui::sidebar_page_values(&app), ("—".into(), "—".into()));
    assert!(!app.sidebar_page_visible());
    let hidden_height = app.sb_height();
    let buttons = ui::sidebar_buttons(&app, false);
    let prev = buttons.iter().find(|(b, _)| *b == Btn::Prev).unwrap().1;
    let next = buttons.iter().find(|(b, _)| *b == Btn::Next).unwrap().1;
    assert_eq!(next.y, prev.y + prev.h + 6, "hidden counters must not reserve an empty row");
    {
        let mut s = bridge.shared.lock().unwrap();
        s.connected = true; s.last_seen = now_ms(); s.page_index = 6; s.page_total = 24;
    }
    let mut timers = Timers::new();
    let update = WpsUpdate { connected: true, events: vec![] };
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &update, &mut timers, false);
    assert_eq!(ui::sidebar_page_values(&app), ("6".into(), "24".into()));
    assert!(app.sidebar_page_visible());
    assert_eq!(app.sb_height(), hidden_height + app.sb_page_h() + 6);
    assert_eq!(app.current_slide, 8);
    app.toggle_whiteboard();
    go_next(&rt, &mut app, Some(&bridge));
    assert_eq!(ui::sidebar_page_values(&app), ("2".into(), "10".into()));
    assert!(app.sidebar_page_visible());
    app.toggle_whiteboard();
    bridge.shared.lock().unwrap().last_seen -= 4000;
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &update, &mut timers, false);
    assert_eq!(ui::sidebar_page_values(&app), ("—".into(), "—".into()));
    assert!(!app.sidebar_page_visible());
    assert_eq!(app.current_slide, 8);
    app.wps_debug = false;
    app.wps_page_index = 6; app.wps_page_total = 24;
    assert_eq!(ui::sidebar_page_values(&app), ("—".into(), "—".into()));
    assert!(!app.sidebar_page_visible());
}

#[test]
fn drawing_outside_tool_popups_repaints_their_entire_old_bounds() {
    for right in [false, true] {
        for mode in [1, 2] {
            for touch in [false, true] {
                let (rt, mut app, mut ip) = ui_fixture();
                app.mode = mode;
                app.popup_on_right = right;
                app.pen_popup_visible = mode == 1;
                app.eraser_popup_visible = mode == 2;
                let panel = if mode == 1 { ui::pen_popup_rect(&app) } else { ui::eraser_popup_rect(&app) };
                let (x, y) = (640, 100); // Far from either panel and both sidebars.
                assert!(!over_ui(&app, x, y));
                rt.begin_batch();
                if touch {
                    handle_touch(&rt, &mut app, &mut ip, None, 42, x, y, TouchKind::Begin, 0.0);
                    assert_eq!(app.t_prev_pos.get(&42), Some(&(x, y)));
                } else {
                    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
                    assert!(app.is_drawing);
                }
                assert!(!app.pen_popup_visible && !app.eraser_popup_visible);
                assert!(rt.dirty.borrow().iter().any(|r|
                    r.x <= panel.x && r.y <= panel.y &&
                    r.x + r.w >= panel.x + panel.w && r.y + r.h >= panel.y + panel.h),
                    "the stroke's small dirty area must not leave old panel pixels visible");
            }
        }
    }
}

#[test]
fn pressing_tool_popup_background_does_not_dismiss_it_or_draw() {
    for mode in [1, 2] {
        for touch in [false, true] {
            let (rt, mut app, mut ip) = ui_fixture();
            app.mode = mode;
            app.pen_popup_visible = mode == 1;
            app.eraser_popup_visible = mode == 2;
            let panel = if mode == 1 { ui::pen_popup_rect(&app) } else { ui::eraser_popup_rect(&app) };
            let (x, y) = (panel.x + 8, panel.y + 8); // Panel padding, outside controls.
            if touch {
                handle_touch(&rt, &mut app, &mut ip, None, 42, x, y, TouchKind::Begin, 0.0);
            } else {
                on_press(&rt, &mut app, &mut ip, None, x, y, 1);
            }
            assert_eq!(app.pen_popup_visible, mode == 1);
            assert_eq!(app.eraser_popup_visible, mode == 2);
            assert!(!app.is_drawing && app.t_prev_pos.is_empty() && !app.page_has_ink);
        }
    }
}

#[test]
fn collapsed_edge_tabs_stay_hidden_until_the_user_expands_them() {
    for right in [false, true] {
        for touch in [false, true] {
            let (rt, mut app, mut ip) = ui_fixture();
            let expanded = ui::sidebar_rect(&app, right);
            collapse_sidebars(&rt, &mut app);
            app.sidebar_anim_start = Some(Instant::now() - Duration::from_secs(121));
            let mut timers = Timers::new();
            tick_shared(&rt, &mut app, &mut ip, None,
                &WpsUpdate { connected: false, events: vec![] }, &mut timers, false);
            assert!(app.collapsed, "idle ticks must never reopen a tucked toolbar");
            assert!(app.sidebar_anim_start.is_none());
            let tab = ui::sidebar_rect(&app, right);
            assert_eq!(if right { tab.x + tab.w } else { tab.x }, if right { app.screen_w } else { 0 });
            assert!(tab.w < expanded.w && tab.h < expanded.h);
            assert_eq!(ui::sidebar_input_rect(&app, right), tab);
            assert!(!over_ui(&app, expanded.x + expanded.w / 2, expanded.y + expanded.h - 8),
                "the hidden part of the toolbar must not block the underlying window");
            let (x, y) = (tab.x + tab.w / 2, tab.y + tab.h / 2);
            if touch {
                handle_touch(&rt, &mut app, &mut ip, None, 42, x, y, TouchKind::Begin, 0.0);
                handle_touch(&rt, &mut app, &mut ip, None, 42, x, y, TouchKind::End, 0.0);
            } else {
                on_press(&rt, &mut app, &mut ip, None, x, y, 1);
                on_release(&rt, &mut app, &mut ip, None, x, y, 1);
            }
            assert!(!app.collapsed);
            assert_eq!(ui::sidebar_rect(&app, right), expanded);
        }
    }
}

#[test]
fn pointer_hover_is_idle_and_buttons_activate_only_on_matching_release() {
    let (rt, mut app, mut ip) = ui_fixture();
    let (x, y) = button_center(&app, Btn::Pen);
    rt.begin_batch();
    on_motion(&rt, &mut app, &mut ip, x, y);
    assert!(app.press.is_none());
    assert!(rt.dirty.borrow().is_empty());
    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
    assert_eq!(app.mode, 0);
    assert!(app.press.unwrap().active);
    on_release(&rt, &mut app, &mut ip, None, x, y, 3);
    assert_eq!(app.mode, 0);
    assert!(ip.pending_ui.is_some());
    on_release(&rt, &mut app, &mut ip, None, x, y, 1);
    assert_eq!(app.mode, 1);
    assert!(app.press.unwrap().until.is_some());
}

#[test]
fn sliding_outside_or_canceling_does_not_activate_a_button() {
    let (rt, mut app, mut ip) = ui_fixture();
    let (x, y) = button_center(&app, Btn::Pen);
    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
    on_motion(&rt, &mut app, &mut ip, 600, 600);
    assert!(!app.press.unwrap().active);
    on_release(&rt, &mut app, &mut ip, None, 600, 600, 1);
    assert_eq!(app.mode, 0);
    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
    cancel_input(&rt, &mut app, &mut ip);
    on_release(&rt, &mut app, &mut ip, None, x, y, 1);
    assert_eq!(app.mode, 0);
    assert!(app.press.is_none());
}

#[test]
fn only_the_owning_finger_can_activate_or_release_ui() {
    let (rt, mut app, mut ip) = ui_fixture();
    let (x, y) = button_center(&app, Btn::Pen);
    handle_touch(&rt, &mut app, &mut ip, None, 1, x, y, TouchKind::Begin, 0.0);
    handle_touch(&rt, &mut app, &mut ip, None, 2, x, y, TouchKind::Begin, 0.0);
    handle_touch(&rt, &mut app, &mut ip, None, 2, x, y, TouchKind::End, 0.0);
    assert_eq!(app.mode, 0);
    assert_eq!(ip.ui_touch, Some(1));
    handle_touch(&rt, &mut app, &mut ip, None, 1, x, y, TouchKind::End, 0.0);
    assert_eq!(app.mode, 1);
    assert!(!app.pen_popup_visible, "a duplicate activation would open the pen popup");
    assert!(ip.ui_touch.is_none());
}

#[test]
fn sidebar_drag_ignores_other_buttons_and_ends_on_its_own_release() {
    let (rt, mut app, mut ip) = ui_fixture();
    app.sb_y = 180;
    let sb = ui::sidebar_rect(&app, false);
    let (x, y) = (sb.x + 1, sb.y + 20); // Background, outside the buttons.
    for button in [2, 3, 4, 5] {
        on_press(&rt, &mut app, &mut ip, None, x, y, button);
        on_motion(&rt, &mut app, &mut ip, x, y - 60);
        on_release(&rt, &mut app, &mut ip, None, x, y - 60, button);
        assert!(!ip.dragging);
        assert_eq!(app.sb_y, 180);
        assert!(app.sidebar_drag.is_none());
    }
    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
    assert!(app.sidebar_drag.is_some_and(|drag| !drag.right));
    on_motion(&rt, &mut app, &mut ip, x, y - 60);
    assert_eq!(app.sb_y, 120);
    assert!(app.sidebar_drag.is_some());
    let (bx, by) = button_center(&app, Btn::Pen);
    on_press(&rt, &mut app, &mut ip, None, bx, by, 1);
    on_press(&rt, &mut app, &mut ip, None, bx, by, 3);
    assert!(ip.pending_ui.is_none(), "another press cannot consume the drag's release");
    on_release(&rt, &mut app, &mut ip, None, x, y - 60, 3);
    assert!(ip.dragging);
    assert!(app.sidebar_drag.is_some());
    on_release(&rt, &mut app, &mut ip, None, x, y - 60, 1);
    assert!(!ip.dragging);
    assert!(app.sidebar_drag.is_none());
    on_motion(&rt, &mut app, &mut ip, x, y - 90);
    assert_eq!(app.sb_y, 120, "moving after release cannot move the toolbar");
    assert_eq!(app.mode, 0);
}

#[test]
fn both_grips_keep_dragging_outside_the_toolbar_and_stop_at_screen_edges() {
    for right in [false, true] {
        for bottom in [false, true] {
            for touch in [false, true] {
                let (rt, mut app, mut ip) = ui_fixture();
                app.sb_y = 180;
                let sb = ui::sidebar_rect(&app, right);
                let x = sb.x + sb.w / 2;
                let y = if bottom { sb.y + sb.h - 5 } else { sb.y + 6 };
                let target_y = if bottom { 2 } else { app.screen_h - 2 };
                let outside_x = app.screen_w / 2;
                assert!(ui::control_at(&app, x, y).is_none(), "grips must not activate nearby buttons");
                if touch {
                    handle_touch(&rt, &mut app, &mut ip, None, 42, x, y, TouchKind::Begin, 0.0);
                    handle_touch(&rt, &mut app, &mut ip, None, 42, outside_x, target_y, TouchKind::Update, 0.0);
                } else {
                    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
                    on_motion(&rt, &mut app, &mut ip, outside_x, target_y);
                }
                let expected = if bottom { 0 } else { app.screen_h - sb.h };
                assert!(ip.dragging && ip.pending_ui.is_none() && !app.is_drawing);
                assert_eq!(app.sb_y, expected);
                assert_eq!(ui::sidebar_rect(&app, right).y, expected);
                assert!(expected >= 0 && expected + sb.h <= app.screen_h);
                assert!(app.sidebar_drag.is_some_and(|drag| drag.right == right));
                if touch {
                    handle_touch(&rt, &mut app, &mut ip, None, 42, outside_x, target_y, TouchKind::End, 0.0);
                } else { on_release(&rt, &mut app, &mut ip, None, outside_x, target_y, 1); }
                assert!(!ip.dragging);
                assert!(app.sidebar_drag.is_none());
                let backend = rt.backend.as_any().downcast_ref::<TestBackend>().unwrap();
                let input = backend.input.borrow();
                assert_eq!(input.len(), 2);
                assert!(input.iter().all(|r| r.x >= 0 && r.y >= 0
                    && r.x + r.w <= app.screen_w && r.y + r.h <= app.screen_h));
                assert!(input.iter().all(|r| outside_x < r.x || outside_x >= r.x + r.w),
                    "released dragging must not leave a screen-wide input region");
                drop(input);
                on_motion(&rt, &mut app, &mut ip, x, target_y + 20);
                assert_eq!(app.sb_y, expected, "moving after release must not continue the drag");
                apply_slider(&mut app, ui::SliderId::Scale, 0.83);
                assert_eq!(app.sb_y, expected);
                collapse_sidebars(&rt, &mut app);
                expand_sidebars(&rt, &mut app);
                assert_eq!(app.sb_y, expected, "scale and collapse changes must not snap back");
                let visible = ui::sidebar_rect(&app, right);
                assert!(visible.y >= 0 && visible.y + visible.h <= app.screen_h);
            }
        }
    }
}

#[test]
fn cancelling_a_drag_clears_feedback_and_hover_does_not_restore_it() {
    let (rt, mut app, mut ip) = ui_fixture();
    let sb = ui::sidebar_rect(&app, true);
    let (x, y) = (sb.x + sb.w / 2, sb.y + 6);
    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
    assert!(app.sidebar_drag.is_some());
    on_motion(&rt, &mut app, &mut ip, x, y + 30);
    let position = app.sb_y;
    cancel_input(&rt, &mut app, &mut ip);
    assert!(!ip.dragging && app.sidebar_drag.is_none());
    rt.dirty.borrow_mut().clear();
    on_motion(&rt, &mut app, &mut ip, x, y + 60);
    assert_eq!(app.sb_y, position);
    assert!(app.sidebar_drag.is_none() && rt.dirty.borrow().is_empty());
}

#[test]
fn drag_reverses_immediately_at_an_edge_and_uses_the_release_position() {
    for collapsed in [false, true] {
        let (rt, mut app, mut ip) = ui_fixture();
        app.collapsed = collapsed;
        app.sb_y = 180;
        let sb = ui::sidebar_rect(&app, false);
        let press_y = if collapsed { sb.y + sb.h / 2 } else { sb.y + sb.h - 5 };
        on_press(&rt, &mut app, &mut ip, None, sb.x + sb.w / 2, press_y, 1);
        assert!(ip.dragging && ip.pending_ui.is_none(), "the tab centre must start dragging too");
        let outside_x = app.screen_w / 2;
        on_motion(&rt, &mut app, &mut ip, outside_x, -100);
        assert_eq!(app.sb_y, 0);
        on_motion(&rt, &mut app, &mut ip, outside_x, -97);
        assert_eq!(app.sb_y, 3, "no accumulated dead travel past the boundary");
        on_release(&rt, &mut app, &mut ip, None, outside_x, -91, 1);
        assert_eq!(app.sb_y, 9, "the release may contain the last pointer movement");
        assert!(!ip.dragging && app.sidebar_drag.is_none());
        assert_eq!(app.collapsed, collapsed, "dragging a tab must not expand it");
    }
}

#[test]
fn resized_or_expanded_toolbars_fit_without_losing_the_dragged_anchor() {
    let (_, mut app, _) = ui_fixture();
    app.sb_y = app.screen_h - app.collapsed_h();
    let anchor = app.sb_y;
    for scale in [0.6, 1.0, 1.4] {
        for collapsed in [false, true] {
            for linked in [false, true] {
                app.sb_scale = scale;
                app.collapsed = collapsed;
                app.wps_connected = linked;
                app.wps_page_index = 2;
                app.wps_page_total = 12;
                app.drawing_tools.set_expanded(true);
                app.drawing_tools.settle();
                for right in [false, true] {
                    let sb = ui::sidebar_rect(&app, right);
                    assert!(sb.y >= 0 && sb.y + sb.h <= app.screen_h);
                }
                assert_eq!(app.sb_y, anchor);
            }
        }
    }
}

#[test]
fn startup_intercepts_only_its_ui_and_waits_until_reveal_before_activating_tools() {
    let (rt, mut app, mut ip) = ui_fixture();
    let start = Instant::now();
    app.launch = launch::Launch::begin("GNOME".into(), true, true, start);
    apply_input_shape(&rt, &app);
    let backend = rt.backend.as_any().downcast_ref::<TestBackend>().unwrap();
    assert_eq!(backend.input.borrow().len(), 1, "only the card is visible initially");
    app.launch.report_link(launch::LinkState::Disabled, start);
    app.launch.advance_at(start + Duration::from_millis(1400));
    let (x, y) = button_center(&app, Btn::Pen);
    assert!(ui::control_at(&app, x, y).is_none());
    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
    on_release(&rt, &mut app, &mut ip, None, x, y, 1);
    assert_eq!(app.mode, 0);
    assert!(!ip.dragging && ip.pending_ui.is_none());
    app.launch.advance_at(start + Duration::from_secs(2));
    apply_input_shape(&rt, &app);
    assert_eq!(backend.input.borrow().len(), 2, "the card input region is removed");
    assert_eq!(ui::control_at(&app, x, y), Some(ui::Control::Sidebar(Btn::Pen, false)));
}

#[test]
fn a_finger_held_through_startup_cannot_turn_into_a_stroke() {
    let (rt, mut app, mut ip) = ui_fixture();
    let start = Instant::now();
    app.launch = launch::Launch::begin("GNOME".into(), true, true, start);
    let r = ui::launch_rect(&app);
    let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
    handle_touch(&rt, &mut app, &mut ip, None, 42, x, y, TouchKind::Begin, 0.0);
    assert!(ip.touch_over_ui.get(&42).copied().unwrap_or(false));
    app.launch.finish();
    app.mode = 1;
    handle_touch(&rt, &mut app, &mut ip, None, 42, x + 100, y, TouchKind::Update, 0.0);
    handle_touch(&rt, &mut app, &mut ip, None, 42, x + 100, y, TouchKind::End, 0.0);
    assert!(!app.page_has_ink && app.t_prev_pos.is_empty());
    assert!(ip.touch_ids.is_empty() && ip.touch_over_ui.is_empty());
}

#[test]
fn startup_reveal_keeps_its_center_and_fits_the_screen_at_every_frame() {
    let (_, mut app, _) = ui_fixture();
    for scale in [0.6, 1.0, 1.4] {
        app.sb_scale = scale;
        app.sb_y = app.screen_h;
        let start = Instant::now();
        app.launch = launch::Launch::begin("X11".into(), true, false, start);
        app.launch.report_link(launch::LinkState::Waiting, start);
        let mut height = 0;
        for ms in (1240..=1640).step_by(20) {
            app.launch.advance_at(start + Duration::from_millis(ms));
            for right in [false, true] {
                let target = ui::sidebar_rect(&app, right);
                let visible = ui::sidebar_launch_rect(&app, right);
                assert!(visible.y >= 0 && visible.y + visible.h <= app.screen_h);
                assert!((2 * visible.y + visible.h - 2 * target.y - target.h).abs() <= 1);
                assert!(visible.h >= height);
                height = visible.h;
            }
        }
    }
}
#[test]
fn more_button_toggles_and_outside_click_dismisses_cursor_popups() {
    let (rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.mode = 0;
    app.more_settings_entry = true;
    let mut ip = Ip::default();
    let more = ui::sidebar_buttons(&app, false)
        .into_iter()
        .find(|(button, _)| *button == Btn::More)
        .unwrap()
        .1;
    let (x, y) = (more.x + more.w / 2, more.y + more.h / 2);
    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
    on_release(&rt, &mut app, &mut ip, None, x, y, 1);
    assert!(app.more_menu_open);
    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
    on_release(&rt, &mut app, &mut ip, None, x, y, 1);
    assert!(
        !app.more_menu_open,
        "clicking the More button again must close the menu"
    );

    app.pen_popup_visible = true;
    app.more_menu_open = false;
    apply_input_shape(&rt, &app);
    assert!(dismiss_capture(&app));
    on_press(&rt, &mut app, &mut ip, None, 320, 300, 1);
    on_release(&rt, &mut app, &mut ip, None, 320, 300, 1);
    assert!(!app.pen_popup_visible);
    assert!(!app.is_drawing && !app.page_has_ink);
    assert!(!dismiss_capture(&app));
}

#[test]
fn whiteboard_dot_controls_toggle_and_apply_the_size_slider() {
    let (rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    let mut ip = Ip::default();
    let toggle = ui::whiteboard_dots_toggle_rect(&app);
    on_press(
        &rt,
        &mut app,
        &mut ip,
        None,
        toggle.x + 20,
        toggle.y + 20,
        1,
    );
    on_release(
        &rt,
        &mut app,
        &mut ip,
        None,
        toggle.x + 20,
        toggle.y + 20,
        1,
    );
    assert!(app.whiteboard_dots);
    let slider = ui::whiteboard_dot_slider_rect(&app);
    on_press(
        &rt,
        &mut app,
        &mut ip,
        None,
        slider.x + slider.w - 1,
        slider.y + 10,
        1,
    );
    assert_eq!(app.whiteboard_dot_size, 10);
    assert!(ip.dot_slider);
    on_release(
        &rt,
        &mut app,
        &mut ip,
        None,
        slider.x + slider.w - 1,
        slider.y + 10,
        1,
    );
    assert!(!ip.dot_slider);
    app.whiteboard_dots = false;
    assert_eq!(
        ui::whiteboard_control_hit(&app, slider.x + 4, slider.y + 4),
        Some(ui::WhiteboardControl::DotSize)
    );
    on_press(&rt, &mut app, &mut ip, None, slider.x + 4, slider.y + 4, 1);
    assert!(!ip.dot_slider);
    assert_eq!(app.whiteboard_dot_size, 10, "a disabled slider must retain its value");
}
