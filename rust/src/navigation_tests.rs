use super::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct TestBackend {
    keys: Rc<Cell<usize>>,
    document_pixels: Rc<Cell<f64>>,
    direct_keys: bool,
    foreground: Cell<Option<bool>>,
    fullscreen: Cell<bool>,
    input: RefCell<Vec<IRect>>,
}
struct RecordingScroll(Rc<Cell<f64>>);
impl document_scroll::ScrollTarget for RecordingScroll {
    fn scroll_by(&mut self, pixels: f64) -> bool { self.0.set(self.0.get()+pixels); true }
}
impl Backend for TestBackend {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn screen_size(&self) -> (i32, i32) { (100, 100) }
    fn present(&self, _: &Pixmap, _: i32, _: i32) {}
    fn set_input_region(&self, rects: &[IRect], _: bool) { *self.input.borrow_mut() = rects.to_vec(); }
    fn fake_key(&self, _: Key) { self.keys.set(self.keys.get() + 1); }
    fn document_scroll_target(&self) -> Option<Box<dyn document_scroll::ScrollTarget>> {
        Some(Box::new(RecordingScroll(self.document_pixels.clone())))
    }
    fn start_presentation(&self) -> bool { self.keys.set(self.keys.get() + 1); true }
    fn can_inject_keys_without_prompt(&self) -> bool { self.direct_keys }
    fn screenshot(&self) -> Option<(Vec<u8>, u32, u32)> { None }
    fn is_presentation_fullscreen(&self) -> bool { self.fullscreen.get() }
    fn presentation_foreground(&self) -> Option<bool> { self.foreground.get() }
    fn has_compositor(&self) -> bool { true }
}

pub(crate) fn fixture(direct_keys: bool) -> (Rt, App, Rc<Cell<usize>>) {
    fixture_with_foreground(direct_keys, None)
}

fn fixture_with_foreground(direct_keys: bool, foreground: Option<bool>) -> (Rt, App, Rc<Cell<usize>>) {
    let keys = Rc::new(Cell::new(0));
    let rt = Rt {
        backend: Box::new(TestBackend {
            keys: keys.clone(), document_pixels: Rc::default(), direct_keys, foreground: Cell::new(foreground),
            fullscreen: Cell::new(false), input: RefCell::new(Vec::new()),
        }),
        fonts: Fonts::load(), icon: None, icon_big: None,
        batching: Cell::new(true), dirty: RefCell::new(Vec::new()),
    };
    let mut app = App::new(Pixmap::new(100, 100).unwrap(), 100, 100);
    app.wps_test_mode = true;
    (rt, app, keys)
}

#[test]
fn document_scroll_uses_the_foreground_document_and_consumes_the_whole_contact() {
    for right in [false,true] {
        let (rt,mut app,keys)=fixture(true);
        let pixels=rt.backend.as_any().downcast_ref::<TestBackend>().unwrap().document_pixels.clone();
        app.configure_geometry(1280,900,1.25);
        let mut ip=Ip::default();
        let now=Instant::now();
        let old_page=app.current_slide;
        let arrow=ui::sidebar_buttons(&app,right).into_iter().find(|(b,_)|*b==Btn::Next).unwrap().1;
        assert_eq!(ui::control_at(&app,arrow.x+arrow.w/2,arrow.y+arrow.h/2),Some(ui::Control::Sidebar(Btn::Next,right)));
        app.document_scroll_enabled=true;
        tick_document_scroll(&rt,&mut app,&mut ip,now);
        tick_document_scroll(&rt,&mut app,&mut ip,now+Duration::from_millis(220));
        let r=ui::scroll_track_rect(&app,right);
        let x=r.x+r.w/2;
        let (centre,_,_)=ui::scroll_thumb_geometry(&app,right);
        handle_touch(&rt,&mut app,&mut ip,None,7,x,centre,TouchKind::Begin,4.0);
        let start=Instant::now();
        tick_document_scroll(&rt,&mut app,&mut ip,start);
        assert_eq!(pixels.get(),0.0);
        handle_touch(&rt,&mut app,&mut ip,None,7,x,r.y+r.h,TouchKind::Update,4.0);
        on_motion(&rt,&mut app,&mut ip,-1,-1);
        handle_touch(&rt,&mut app,&mut ip,None,8,x,r.y,TouchKind::Begin,4.0);
        handle_touch(&rt,&mut app,&mut ip,None,8,x,r.y,TouchKind::End,4.0);
        tick_document_scroll(&rt,&mut app,&mut ip,start+Duration::from_millis(20));
        let down=pixels.get();
        assert!(down>20.0 && down<=28.1);
        handle_touch(&rt,&mut app,&mut ip,None,7,x,r.y,TouchKind::Update,4.0);
        tick_document_scroll(&rt,&mut app,&mut ip,start+Duration::from_millis(40));
        assert!(pixels.get().abs()<0.1);
        handle_touch(&rt,&mut app,&mut ip,None,7,x,centre,TouchKind::End,4.0);
        let stopped=pixels.get();
        tick_document_scroll(&rt,&mut app,&mut ip,start+Duration::from_secs(1));
        assert_eq!(pixels.get(),stopped);
        assert_eq!(app.scroll_rail.thumb,0.0);
        assert_eq!(app.current_slide,old_page);
        assert!(app.page_picker.is_none() && !app.is_drawing && keys.get()==0);
    }
}

#[test]
fn document_scroll_morphs_for_ppt_and_whiteboard_and_never_converts_a_held_drag_to_a_click() {
    let (rt,mut app,keys)=fixture(true);
    app.configure_geometry(1280,900,1.0);
    let mut ip=Ip::default();
    let start=Instant::now();
    app.document_scroll_enabled=true;
    tick_document_scroll(&rt,&mut app,&mut ip,start);
    tick_document_scroll(&rt,&mut app,&mut ip,start+Duration::from_millis(200));
    let r=ui::scroll_track_rect(&app,false);
    let x=r.x+r.w/2;
    on_press(&rt,&mut app,&mut ip,None,x,r.y+r.h-3,1);
    app.wps_connected=true;
    app.scroll_rail.ppt_foreground=true; // Linked before the first page report.
    assert!(!app.wants_scroll_rail());
    tick_document_scroll(&rt,&mut app,&mut ip,start+Duration::from_millis(210));
    assert!(ip.document_drag.is_none());
    app.wps_page_index=3;app.wps_page_total=15;
    tick_document_scroll(&rt,&mut app,&mut ip,start+Duration::from_millis(285));
    assert!(app.scroll_rail.mix>0.0 && app.scroll_rail.mix<1.0);
    let partial=app.scroll_rail.mix;
    app.scroll_rail.ppt_foreground=false; // WPS is still running behind a document.
    assert!(app.wants_scroll_rail()); // Writer can take focus before PPT clears its cached page count.
    tick_document_scroll(&rt,&mut app,&mut ip,start+Duration::from_millis(285));
    assert_eq!(partial,app.scroll_rail.mix);
    tick_document_scroll(&rt,&mut app,&mut ip,start+Duration::from_millis(500));
    assert_eq!(app.scroll_rail.mix,1.0);
    on_release(&rt,&mut app,&mut ip,None,x,r.y+r.h-3,1);
    assert_eq!(keys.get(),0);
    let focus = &rt.backend.as_any().downcast_ref::<TestBackend>().unwrap().foreground;
    focus.set(Some(true));
    on_press(&rt,&mut app,&mut ip,None,x,r.y+3,1);
    assert!(ip.document_drag.is_none());
    on_release(&rt,&mut app,&mut ip,None,x,r.y+3,1);
    focus.set(Some(false));
    on_press(&rt,&mut app,&mut ip,None,x,r.y+3,1);
    app.toggle_whiteboard();
    let page=app.current_slide;
    tick_document_scroll(&rt,&mut app,&mut ip,start+Duration::from_millis(510));
    tick_document_scroll(&rt,&mut app,&mut ip,start+Duration::from_millis(720));
    app.drawing_tools.advance_at(start+Duration::from_millis(720));
    assert_eq!(app.scroll_rail.mix,0.0);
    let badge=ui::sidebar_page_rect(&app,false);
    on_release(&rt,&mut app,&mut ip,None,badge.x+badge.w/2,badge.y+badge.h/2,1);
    assert!(app.page_picker.is_none() && app.current_slide==page);
    assert_eq!(ui::control_at(&app,badge.x+badge.w/2,badge.y+badge.h/2),Some(ui::Control::PageCount(false)));
    app.toggle_whiteboard();
    assert!(app.wants_scroll_rail());
}

#[test]
fn document_scroll_rate_is_linear_stops_at_centre_and_ignores_sleep_backlog() {
    let now=Instant::now();
    let mut drag=document_scroll::Drag::new(100,100,43.0,0.0,20,now);
    assert_eq!(drag.position(102),0.0);
    assert_eq!(drag.position(123),0.5);
    assert_eq!(drag.position(77),-0.5);
    assert_eq!(drag.position(1000),1.0);
    assert_eq!(drag.distance(0.5,now+Duration::from_millis(20)),14.0);
    assert!(drag.distance(1.0,now+Duration::from_secs(6))<=56.0);
}

#[test]
fn document_scroll_settings_entry_is_reachable_in_short_and_narrow_windows() {
    for (w,h) in [(320,240),(680,400),(1280,900)] {
        let (rt,mut app,_)=fixture(false);
        app.configure_geometry(w,h,1.0);
        app.settings_open=true;
        let tab=SetHit::Tab(settings_state::Tab::Experimental);
        let nav=ui::settings_controls(&app);
        for (key,r) in nav.iter().filter(|(k,_)| matches!(k,SetHit::Tab(_))) {
            let cancel=nav.iter().find(|(k,_)|*k==SetHit::Quit).unwrap().1;
            assert!(!cancel.contains(r.x+2,r.y+r.h-2));
            assert_eq!(ui::control_at(&app,r.x+r.w/2,r.y+r.h/2),Some(ui::Control::Setting(*key)));
        }
        handle_settings_hit(&rt,&mut app,tab);
        let (_,r)=ui::settings_controls(&app).into_iter().find(|(k,_)|*k==SetHit::DocumentScroll).unwrap();
        let (x,y)=(r.x+r.w-20,r.y+r.h/2);
        let mut ip=Ip::default();
        on_press(&rt,&mut app,&mut ip,None,x,y,1);
        on_release(&rt,&mut app,&mut ip,None,x,y,1);
        assert!(app.document_scroll_enabled);
        #[cfg(unix)]
        {
            // Model a completed read of the desktop setting without enabling
            // accessibility on the machine running this test.
            app.settings_ui.scroll_access = Default::default();
            app.settings_ui.scroll_access.enabled = Some(false);
            let layout = settings_panel::layout(&app);
            app.settings_page = 1 / layout.rows;
            let (_, r) = ui::settings_controls(&app).into_iter()
                .find(|(key, _)| *key == SetHit::EnableScrollAccess).unwrap();
            assert_eq!(ui::control_at(&app, r.x+r.w-42, r.y+r.h/2),
                Some(ui::Control::Setting(SetHit::EnableScrollAccess)));
            app.settings_ui.scroll_access.enabled = Some(true);
            assert!(!ui::settings_controls(&app).iter().any(|(key, _)| *key == SetHit::EnableScrollAccess));
        }
    }
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
fn presentation_end_releases_pen_and_eraser_for_the_editor() {
    for mode in [1, 2] {
        let (rt, mut app, keys) = fixture_with_foreground(true, Some(false));
        app.configure_geometry(800, 600, 1.0);
        app.wps_real_pos = 4;
        app.current_slide = 4;
        switch_to_draw(&rt, &mut app, mode);
        app.pen_popup_visible = mode == 1;
        app.eraser_popup_visible = mode == 2;
        app.is_drawing = true;

        let update = WpsUpdate { connected: true, events: vec![WpsEvent::SlideshowEnd] };
        tick_shared(&rt, &mut app, &mut Ip::default(), None, &update, &mut Timers::new(), false);

        assert_eq!(app.mode, 0);
        assert!(!app.is_drawing && !app.pen_popup_visible && !app.eraser_popup_visible);
        assert!(!point_input_ok(&app, 400, 300));
        let backend = rt.backend.as_any().downcast_ref::<TestBackend>().unwrap();
        let input = backend.input.borrow();
        assert!(!input.is_empty() && input.iter().all(|r| r.x + r.w <= 400 || r.x > 400));
        assert_eq!((app.wps_real_pos, app.current_slide), (-1, 1));
        assert_eq!(keys.get(), 0, "WPS has already handled the physical Escape");
    }
}

#[test]
fn presentation_fullscreen_fallback_requires_an_unlinked_show_and_wps_focus() {
    for (connected, started_in_wps, returned_to_wps, expected_mode) in [
        (false, true, true, 0),  // WPS returned from its show to the editor.
        (false, true, false, 2), // Alt+Tab away from the show.
        (false, false, true, 2), // Switch from an unrelated fullscreen app to WPS.
        (true, true, true, 2),   // The linked show has not sent its end event.
    ] {
        let (rt, mut app, keys) = fixture_with_foreground(true, Some(started_in_wps));
        let backend = rt.backend.as_any().downcast_ref::<TestBackend>().unwrap();
        let update = WpsUpdate { connected, events: Vec::new() };
        let mut timers = Timers::new();
        let mut ip = Ip::default();
        switch_to_draw(&rt, &mut app, 2);
        backend.fullscreen.set(true);
        tick_shared(&rt, &mut app, &mut ip, None, &update, &mut timers, false);
        assert_eq!(app.mode, 2);

        backend.fullscreen.set(false);
        backend.foreground.set(Some(returned_to_wps));
        timers.last_foreground = Instant::now(); // The cached focus still belongs to the old window.
        timers.last_wps -= Duration::from_secs(1);
        tick_shared(&rt, &mut app, &mut ip, None, &update, &mut timers, false);
        assert_eq!(app.mode, expected_mode);
        assert_eq!(keys.get(), 0);
    }
}

#[test]
fn presentation_lifecycle_does_not_erase_visible_whiteboard() {
    for (event, overlay_page) in [(WpsEvent::SlideshowBegin(4), 4), (WpsEvent::SlideshowEnd, 1)] {
        let (rt, mut app, keys) = fixture(false);
        let ended = matches!(event, WpsEvent::SlideshowEnd);
        app.current_slide = 7;
        switch_to_draw(&rt, &mut app, 2);
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
        assert_eq!(app.mode, 1);
        assert_eq!(app.current_slide, 2);
        assert_eq!(app.canvas.data(), second.as_slice());
        // A manual mode switch now clears the whiteboard but keeps the overlay.
        app.toggle_whiteboard();
        assert_eq!(app.current_slide, overlay_page);
        assert_eq!(app.mode, if ended { 0 } else { 2 });
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
    app.configure_geometry(1280, 720, 1.0);
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
fn toolbar_fade_only_runs_during_a_foreground_presentation() {
    let (rt, mut app, _) = fixture_with_foreground(true, Some(true));
    let backend = rt.backend.as_any().downcast_ref::<TestBackend>().unwrap();
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    app.fade_toolbar_during_show = true;
    for (connected, slide, fullscreen, focus, should_fade) in [
        (true, 2, false, Some(true), true),
        (true, 2, false, None, true), // Native Wayland can rely on the add-in.
        (true, 2, true, Some(false), false),
        (true, -1, true, Some(true), false), // An editor with a connected add-in.
        (false, -1, true, Some(true), true),
        (false, -1, true, Some(false), false),
        (false, -1, false, Some(true), false),
    ] {
        backend.fullscreen.set(fullscreen);
        backend.foreground.set(focus);
        {
            let mut state = bridge.shared.lock().unwrap();
            state.connected = connected;
            state.real_pos = slide;
            state.last_seen = now_ms();
        }
        app.presentation_fade.reset();
        app.presentation_fade.advance_at(true, Instant::now() - Duration::from_secs(7));
        let update = WpsUpdate { connected, events: vec![] };
        tick_shared(&rt, &mut app, &mut Ip::default(), Some(&bridge), &update, &mut Timers::new(), false);
        assert_eq!(app.presentation_fade.opacity() < 1.0, should_fade, "connected={connected} slide={slide} focus={focus:?}");
    }

    // Switching directly to another deck emits Begin without an intervening idle tick.
    let mut timers = Timers::new();
    let mut ip = Ip::default();
    {
        let mut state = bridge.shared.lock().unwrap();
        state.connected = true;
        state.real_pos = 3;
        state.last_seen = now_ms();
    }
    app.presentation_fade.advance_at(true, Instant::now() - Duration::from_secs(7));
    let mut update = WpsUpdate { connected: true, events: vec![WpsEvent::RealPos(3)] };
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &update, &mut timers, false);
    assert_eq!(app.presentation_fade.opacity(), 0.2, "a page change alone does not wake the rails");
    update.events = vec![WpsEvent::SlideshowBegin(3)];
    tick_shared(&rt, &mut app, &mut ip, Some(&bridge), &update, &mut timers, false);
    assert_eq!(app.presentation_fade.opacity(), 1.0, "a new show gets the full grace period");
}

#[test]
fn toolbar_fade_keeps_touch_targets_and_executes_the_first_tap() {
    for (right, touch) in [(false, true), (true, false)] {
        let (rt, mut app, mut ip) = ui_fixture();
        app.fade_toolbar_during_show = true;
        let now = Instant::now();
        tick_toolbar_fade(&rt, &mut app, &ip, true, now - Duration::from_secs(7));
        tick_toolbar_fade(&rt, &mut app, &ip, true, now);
        assert_eq!(app.presentation_fade.opacity(), 0.2);
        on_motion(&rt, &mut app, &mut ip, 650, 450);
        on_press(&rt, &mut app, &mut ip, None, 650, 450, 1);
        on_release(&rt, &mut app, &mut ip, None, 650, 450, 1);
        assert_eq!(app.presentation_fade.opacity(), 0.2, "touching the slide should not wake the rails");
        let control = ui::Control::Sidebar(Btn::Pen, right);
        let r = ui::control_rect(&app, control).unwrap();
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        assert_eq!(ui::control_at(&app, x, y), Some(control));
        if touch { handle_touch(&rt, &mut app, &mut ip, None, 1, x, y, TouchKind::Begin, 1.0); }
        else { on_press(&rt, &mut app, &mut ip, None, x, y, 1); }
        assert_eq!(app.presentation_fade.opacity(), 1.0);
        tick_toolbar_fade(&rt, &mut app, &ip, true, now + Duration::from_secs(30));
        tick_toolbar_fade(&rt, &mut app, &ip, true, now + Duration::from_secs(40));
        assert_eq!(app.presentation_fade.opacity(), 1.0, "a stationary press still owns the tool");
        assert_eq!(app.mode, 0);
        if touch { handle_touch(&rt, &mut app, &mut ip, None, 1, x, y, TouchKind::End, 0.0); }
        else { on_release(&rt, &mut app, &mut ip, None, x, y, 1); }
        assert_eq!(app.mode, 1);
        assert!(!app.pen_popup_visible);
    }
}

#[test]
fn toolbar_fade_composites_both_rails_over_the_ink_at_fractional_scale() {
    let (rt, mut app, ip) = ui_fixture();
    app.configure_geometry(820, 720, 1.25);
    app.fade_toolbar_during_show = true;
    let render = |app: &App, r: R| ui::render_region(app, &rt.fonts, None, r.x, r.y, r.w, r.h).unwrap();
    let rails = [false, true].map(|right| ui::sidebar_rect(&app, right).with_paint_margin());
    let normal = rails.map(|r| render(&app, r));
    let now = Instant::now();
    tick_toolbar_fade(&rt, &mut app, &ip, true, now - Duration::from_secs(7));
    tick_toolbar_fade(&rt, &mut app, &ip, true, now);
    let faded = rails.map(|r| render(&app, r));
    for (before, after) in normal.iter().zip(&faded) {
        assert!(before.pixels().iter().any(|p| p.alpha() == 255));
        assert!(before.pixels().iter().zip(after.pixels()).all(|(a,b)| {
            let expected = (f32::from(a.alpha()) * 0.2).round() as u8;
            b.alpha().abs_diff(expected) <= 1
        }));
    }
    // An opaque handwritten stroke beneath the toolbar must keep its own opacity.
    let red = tiny_skia::Color::from_rgba8(210, 30, 35, 255);
    app.canvas.fill(red);
    let under_ink = render(&app, rails[0]);
    let index = faded[0].pixels().iter().position(|p| p.alpha() == 51).unwrap();
    let toolbar = faded[0].pixels()[index];
    let pixel = under_ink.pixels()[index];
    assert_eq!(pixel.alpha(), 255);
    assert!(pixel.red().abs_diff(toolbar.red() + 168) <= 1);
    let center = render(&app, R {x:400, y:340, w:12, h:12});
    assert!(center.pixels().iter().all(|p| *p == red.to_color_u8().premultiply()));
    rt.dirty.borrow_mut().clear();
    tick_toolbar_fade(&rt, &mut app, &ip, true, now + Duration::from_secs(60));
    assert!(rt.dirty.borrow().is_empty());
}

#[test]
fn toolbar_fade_leaves_popups_and_the_whiteboard_fully_visible() {
    let (rt, mut app, mut ip) = ui_fixture();
    app.fade_toolbar_during_show = true;
    let before = Instant::now() - Duration::from_secs(10);
    let now = Instant::now();
    for state in ["pen", "eraser", "pages", "drag", "appearance", "board"] {
        app.pen_popup_visible = state == "pen";
        app.eraser_popup_visible = state == "eraser";
        app.page_picker = (state == "pages").then(|| page_picker::PagePicker::new(2, 9, page_picker::Source::Wps(1)));
        ip.dragging = state == "drag";
        app.settings_open = state == "appearance";
        app.settings_ui.tab = settings_state::Tab::Appearance;
        app.whiteboard = state == "board";
        app.presentation_fade.advance_at(true, before);
        tick_toolbar_fade(&rt, &mut app, &ip, true, now);
        tick_toolbar_fade(&rt, &mut app, &ip, true, now + Duration::from_secs(30));
        assert_eq!(app.presentation_fade.opacity(), 1.0, "{state}");
    }
}

#[test]
fn coordinate_edit_sliders_and_done_drive_the_guide() {
    let (rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    app.coord = Some(crate::app::CoordGuide::new(640, 360,
        crate::app::ConicKind::Ellipse, crate::app::ConicDirection::Horizontal));
    let mut ip = Ip::default();
    let zoom = ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::ZoomSlider)).unwrap();
    on_press(&rt, &mut app, &mut ip, None, zoom.x + zoom.w - 1, zoom.y + 5, 1);
    assert!(app.coord.unwrap().scale > 100.0);
    on_release(&rt, &mut app, &mut ip, None, zoom.x + zoom.w - 1, zoom.y + 5, 1);
    let size = ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::SizeSlider)).unwrap();
    on_press(&rt, &mut app, &mut ip, None, size.x + size.w - 1, size.y + 5, 1);
    assert!(app.coord.unwrap().size > 0.9);
    on_release(&rt, &mut app, &mut ip, None, size.x + size.w - 1, size.y + 5, 1);
    let shape = ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::ShapeSlider)).unwrap();
    on_press(&rt, &mut app, &mut ip, None, shape.x + shape.w - 1, shape.y + 5, 1);
    assert!(app.coord.unwrap().shape > 0.9);
    on_release(&rt, &mut app, &mut ip, None, shape.x + shape.w - 1, shape.y + 5, 1);
    let origin = (app.coord.unwrap().origin_x as i32, app.coord.unwrap().origin_y as i32);
    on_press(&rt, &mut app, &mut ip, None, origin.0, origin.1, 1);
    on_motion(&rt, &mut app, &mut ip, origin.0 + 20, origin.1 + 10);
    on_release(&rt, &mut app, &mut ip, None, origin.0 + 20, origin.1 + 10, 1);
    assert!(app.coord.unwrap().origin_x > 330.0);
    let origin = (app.coord.unwrap().origin_x as i32, app.coord.unwrap().origin_y as i32);
    let (hx,hy) = ui::coord_rotate_handle(&app).unwrap();
    let destination = (origin.0 - (hy as i32 - origin.1), origin.1 + (hx as i32 - origin.0));
    on_press(&rt, &mut app, &mut ip, None, hx as i32, hy as i32, 1);
    on_motion(&rt, &mut app, &mut ip, destination.0, destination.1);
    on_release(&rt, &mut app, &mut ip, None, destination.0, destination.1, 1);
    assert!((app.coord.unwrap().rotation - std::f32::consts::FRAC_PI_2).abs() < 0.2);
    let done = ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::Done)).unwrap();
    on_press(&rt, &mut app, &mut ip, None, done.x + 5, done.y + 5, 1);
    on_release(&rt, &mut app, &mut ip, None, done.x + 5, done.y + 5, 1);
    assert!(!app.coord.unwrap().editing);
    assert_eq!(app.mode, 1, "完成 returns to the pen");
}

#[test]
fn whiteboard_cursor_and_edit_share_the_sidebar_button() {
    let (_rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    assert!(ui::control_enabled(&app, ui::Control::Sidebar(Btn::Cursor, false)));
    switch_to_cursor(&_rt, &mut app);
    assert_eq!(app.mode, 0);
    assert!(app.coord.is_none());
    app.set_coord(crate::app::ConicKind::Ellipse, crate::app::ConicDirection::Horizontal);
    assert!(app.coord.unwrap().editing);
    assert_eq!(app.mode, 0);
    switch_to_draw(&_rt, &mut app, 1);
    assert!(!app.coord.unwrap().editing);
    assert_eq!(app.mode, 1);
    switch_to_cursor(&_rt, &mut app);
    assert!(app.coord.unwrap().editing);
    assert_eq!(app.mode, 0);
}

#[test]
fn switching_shapes_keeps_the_guide_transform() {
    let (_rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    app.set_coord(crate::app::ConicKind::Ellipse, crate::app::ConicDirection::Horizontal);
    if let Some(coord) = app.coord.as_mut() {
        coord.origin_x = 111.0; coord.origin_y = 222.0;
        coord.scale = 77.0; coord.size = 0.8; coord.shape = 0.2; coord.rotation = 0.3;
    }
    app.set_coord(crate::app::ConicKind::Parabola, crate::app::ConicDirection::Up);
    let coord = app.coord.unwrap();
    assert_eq!((coord.origin_x, coord.origin_y), (111.0, 222.0));
    assert_eq!((coord.scale, coord.size, coord.shape, coord.rotation), (77.0, 0.8, 0.2, 0.3));
    assert_eq!(coord.conic, crate::app::ConicKind::Parabola);
}

#[test]
fn whiteboard_guides_belong_to_their_page_and_clear_keeps_them() {
    let (_rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    app.set_coord(crate::app::ConicKind::Ellipse, crate::app::ConicDirection::Horizontal);
    app.save_coord_page();
    app.save_current_page();
    app.current_slide = 2;
    app.load_page(2);
    assert!(app.coord.is_none(), "a new page starts without a guide");
    assert_eq!(app.mode, 1, "leaving edit mode restores the pen");
    app.set_coord(crate::app::ConicKind::Parabola, crate::app::ConicDirection::Up);
    app.save_coord_page();
    app.save_current_page();
    app.current_slide = 1;
    app.load_page(1);
    let guide = app.coord.unwrap();
    assert_eq!(guide.conic, crate::app::ConicKind::Ellipse);
    assert!(!guide.editing);
    app.clear_current_strokes();
    assert!(app.coord.is_some(), "clearing ink must not delete the guide");
    app.toggle_whiteboard();
    assert!(app.coord.is_none() && app.coord_pages.is_empty());
}

#[test]
fn whiteboard_delete_button_removes_the_current_page_guide() {
    let (rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    app.set_coord(crate::app::ConicKind::Ellipse, crate::app::ConicDirection::Horizontal);
    let delete = ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::Delete)).unwrap();
    let mut ip = Ip::default();
    on_press(&rt, &mut app, &mut ip, None, delete.x + 5, delete.y + 5, 1);
    on_release(&rt, &mut app, &mut ip, None, delete.x + 5, delete.y + 5, 1);
    assert!(app.coord.is_none());
    assert!(!app.coord_pages.contains_key(&1));
    assert_eq!(app.mode, 1);
}

#[test]
fn selection_lasso_moves_whiteboard_ink_and_can_commit() {
    let (rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    {
        let data = app.canvas.data_mut();
        let i = (100 * 640 + 100) * 4;
        data[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
    }
    app.page_has_ink = true;
    let mut ip = Ip::default();
    let select = ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::Select)).unwrap();
    on_press(&rt, &mut app, &mut ip, None, select.x + 4, select.y + 4, 1);
    on_release(&rt, &mut app, &mut ip, None, select.x + 4, select.y + 4, 1);
    assert!(app.select_mode);
    on_press(&rt, &mut app, &mut ip, None, 80, 80, 1);
    on_motion(&rt, &mut app, &mut ip, 120, 120);
    on_release(&rt, &mut app, &mut ip, None, 120, 120, 1);
    assert!(app.floating.is_some(), "marquee must lift the selected ink");
    assert_eq!(app.canvas.data()[(100 * 640 + 100) * 4 + 3], 0);
    on_press(&rt, &mut app, &mut ip, None, 100, 100, 1);
    on_motion(&rt, &mut app, &mut ip, 130, 110);
    on_release(&rt, &mut app, &mut ip, None, 130, 110, 1);
    let done = ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::SelDone)).unwrap();
    on_press(&rt, &mut app, &mut ip, None, done.x + 5, done.y + 5, 1);
    on_release(&rt, &mut app, &mut ip, None, done.x + 5, done.y + 5, 1);
    assert!(app.floating.is_none());
    assert_eq!(app.canvas.data()[(110 * 640 + 130) * 4 + 3], 255);
}

#[test]
fn floating_selection_commits_when_tools_change_or_the_page_is_cleared() {
    let (rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    let ink = {
        let data = app.canvas.data_mut();
        let i = (100 * 640 + 100) * 4;
        data[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
        i
    };
    app.page_has_ink = true;
    let lasso = [(80.0, 80.0), (120.0, 80.0), (120.0, 120.0), (80.0, 120.0)];
    app.floating = crate::selection::lift(&mut app, &lasso);
    assert!(app.floating.is_some());
    switch_to_draw(&rt, &mut app, 1);
    assert!(app.floating.is_none());
    assert_eq!(app.canvas.data()[ink + 3], 255, "the lifted ink must be committed");
    app.floating = crate::selection::lift(&mut app, &lasso);
    app.floating.as_mut().unwrap().y += 20;
    clear_drawing(&rt, &mut app);
    assert!(app.floating.is_none());
    handle_btn(&rt, &mut app, None, Btn::Restore, false);
    assert_eq!(app.canvas.data()[ink + 3], 0);
    assert_eq!(app.canvas.data()[ink + 20 * 640 * 4 + 3], 255, "restore the ink at its moved position");
}

#[test]
fn whiteboard_selection_works_through_the_touch_path() {
    let (rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    {
        let data = app.canvas.data_mut();
        let i = (100 * 640 + 100) * 4;
        data[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
    }
    app.page_has_ink = true;
    let mut ip = Ip::default();
    let select = ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::Select)).unwrap();
    handle_touch(&rt, &mut app, &mut ip, None, 7, select.x + 5, select.y + 5, TouchKind::Begin, 1.0);
    handle_touch(&rt, &mut app, &mut ip, None, 7, select.x + 5, select.y + 5, TouchKind::End, 0.0);
    assert!(app.select_mode);
    handle_touch(&rt, &mut app, &mut ip, None, 8, 80, 80, TouchKind::Begin, 1.0);
    handle_touch(&rt, &mut app, &mut ip, None, 8, 120, 120, TouchKind::Update, 1.0);
    handle_touch(&rt, &mut app, &mut ip, None, 8, 120, 120, TouchKind::End, 0.0);
    assert!(app.floating.is_some(), "the touch marquee must lift ink");
    assert_eq!(app.canvas.data()[(100 * 640 + 100) * 4 + 3], 0);
}

#[test]
fn whiteboard_cursor_touch_does_not_draw() {
    let (rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    switch_to_cursor(&rt, &mut app);
    assert_eq!(app.mode, 0);
    let mut ip = Ip::default();
    handle_touch(&rt, &mut app, &mut ip, None, 9, 100, 100, TouchKind::Begin, 1.0);
    handle_touch(&rt, &mut app, &mut ip, None, 9, 160, 140, TouchKind::Update, 1.0);
    handle_touch(&rt, &mut app, &mut ip, None, 9, 160, 140, TouchKind::End, 0.0);
    assert!(app.canvas.data().iter().all(|byte| *byte == 0), "cursor touch must not paint");
}

#[test]
fn thumbnail_decoder_accepts_only_private_temp_files() {
    let dir = std::env::temp_dir().join("sidera-thumbs");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("test-{}.png", std::process::id()));
    let image = image::RgbaImage::from_raw(2, 2, vec![255u8; 16]).unwrap();
    image.save(&path).unwrap();
    let pm = super::load_thumbnail(path.to_str().unwrap()).unwrap();
    assert_eq!((pm.width(), pm.height()), (2, 2));
    let _ = std::fs::remove_file(&path);
    assert!(super::load_thumbnail("/etc/passwd").is_none());
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

fn rendered_page_preview(rt: &Rt, app: &App) -> Pixmap {
    let r = ui::page_picker_thumb_rect(app);
    ui::render_region(app, &rt.fonts, None, r.x, r.y, r.w, r.h).unwrap()
}

#[test]
fn page_wheel_shows_live_saved_and_empty_board_pages_without_navigating() {
    use tiny_skia::Color;
    let (rt, mut app, _) = ui_fixture();
    app.configure_geometry(960, 540, 1.25);
    app.toggle_whiteboard();
    let red = Color::from_rgba8(210, 40, 35, 255);
    let blue = Color::from_rgba8(30, 80, 210, 255);
    app.canvas.fill(red);
    app.page_has_ink = true;
    go_next(&rt, &mut app, None);
    app.canvas.fill(blue);
    app.page_has_ink = true;
    // WPS page numbers may coincide with local ones.
    let mut slide = Pixmap::new(16, 9).unwrap();
    slide.fill(red);
    app.thumbs.insert((0, 3), slide);
    open_page_picker(&rt, &mut app, true);
    app.open_anim = 0;
    let center = |app: &App| {
        let pm = rendered_page_preview(&rt, app);
        pm.pixel(pm.width() / 2, pm.height() / 2).unwrap()
    };
    assert_eq!(center(&app), blue.to_color_u8().premultiply());
    open_value_input(&rt, &mut app, value_input::Target::Page);
    value_input_key(&rt, &mut app, value_input::Key::Character('1'));
    value_input_key(&rt, &mut app, value_input::Key::Enter);
    assert_eq!(center(&app), red.to_color_u8().premultiply());
    app.page_picker.as_mut().unwrap().set_selected(3);
    for background in [0, 1] {
        app.whiteboard_bg_index = background;
        assert_eq!(center(&app), app.whiteboard_bg_color().to_color_u8().premultiply());
    }
    assert_eq!(app.current_slide, 2);
    assert!(!app.whiteboard_cache.contains_key(&2));
    assert!(app.thumb_waiting.is_none() && app.thumb_target.is_none());
    close_all_popups(&rt, &mut app);
    assert_eq!(app.canvas.pixel(0, 0).unwrap(), blue.to_color_u8().premultiply());

    // Clearing and restoring live ink must also refresh an already-open preview.
    open_page_picker(&rt, &mut app, false);
    app.open_anim = 0;
    assert_eq!(center(&app), blue.to_color_u8().premultiply());
    app.remember_before_clear();
    app.clear_current_strokes();
    assert_eq!(center(&app), app.whiteboard_bg_color().to_color_u8().premultiply());
    assert!(app.restore_cleared());
    assert_eq!(center(&app), blue.to_color_u8().premultiply());
}

#[test]
fn page_wheel_composes_guides_and_floating_ink_in_page_coordinates() {
    let (rt, mut app, _) = ui_fixture();
    app.configure_geometry(640, 360, 1.5);
    app.toggle_whiteboard();
    app.whiteboard_bg_index = 1;
    let mut guide = app::CoordGuide::new(640, 360, app::ConicKind::Ellipse, app::ConicDirection::Horizontal);
    guide.editing = false;
    guide.rotation = 0.4;
    app.coord = Some(guide);
    open_page_picker(&rt, &mut app, false);
    app.open_anim = 0;
    let live_guide = rendered_page_preview(&rt, &app);
    close_all_popups(&rt, &mut app);
    go_next(&rt, &mut app, None);
    assert!(!app.whiteboard_cache.contains_key(&1), "a guide-only page has no ink snapshot");

    let red = tiny_skia::Color::from_rgba8(220, 20, 30, 255);
    crate::paint::pen_segment(&mut app.canvas, red, (120, 180), (150, 180), 24,
        tiny_skia::Transform::from_scale(1.5, 1.5));
    app.page_has_ink = true;
    app.floating = crate::selection::lift(&mut app, &[(80., 140.), (190., 140.), (190., 220.), (80., 220.)]);
    assert!(app.floating.is_some());
    crate::selection::move_to_pointer(&mut app, (0, 0), 400, 140);
    let lifted_canvas = app.canvas.data().to_vec();
    open_page_picker(&rt, &mut app, false);
    app.open_anim = 0;
    let live = rendered_page_preview(&rt, &app);
    let reds: Vec<_> = live.pixels().iter().enumerate()
        .filter(|(_, p)| p.red() > 180 && p.green() < 60 && p.blue() < 60)
        .map(|(i, _)| i % live.width() as usize).collect();
    assert!(!reds.is_empty() && reds.iter().all(|&x| x > live.width() as usize / 2));
    app.page_picker.as_mut().unwrap().set_selected(1);
    let saved_guide = rendered_page_preview(&rt, &app);
    assert!(saved_guide.data() == live_guide.data());
    app.page_picker.as_mut().unwrap().set_selected(3);
    let blank = rendered_page_preview(&rt, &app);
    assert!(saved_guide.pixels().iter().zip(blank.pixels()).filter(|(a, b)| a != b).count() > 100);
    close_all_popups(&rt, &mut app);
    assert_eq!(app.current_slide, 2);
    assert!(app.floating.is_some() && app.canvas.data() == lifted_canvas);
}

#[test]
fn page_wheel_fits_whiteboard_preview_and_touch_buttons_on_short_outputs() {
    let (rt, mut app, _) = ui_fixture();
    app.toggle_whiteboard();
    for (w, h) in [(640, 320), (720, 480), (480, 780)] {
        app.configure_geometry(w, h, 1.0);
        for right in [false, true] {
            open_page_picker(&rt, &mut app, right);
            let panel = ui::page_picker_rect(&app);
            let thumb = ui::page_picker_thumb_rect(&app);
            let wheel = ui::page_picker_control_rect(&app, ui::PagePickerHit::Wheel);
            assert!(panel.x >= 0 && panel.y >= 0 && panel.x + panel.w <= w && panel.y + panel.h <= h);
            assert!(thumb.h >= 44 && thumb.x + thumb.w <= wheel.x && wheel.h >= 88);
            for hit in [ui::PagePickerHit::Cancel, ui::PagePickerHit::Jump] {
                let button = ui::page_picker_control_rect(&app, hit);
                assert!(wheel.y + wheel.h <= button.y && button.y + button.h <= panel.y + panel.h);
                assert_eq!(ui::page_picker_hit(&app, button.x + button.w / 2, button.y + button.h / 2), Some(hit));
            }
        }
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
fn page_wheel_keeps_editor_jumps_without_rendering_or_requesting_thumbnails() {
    let (rt, mut app, mut ip) = ui_fixture();
    rt.begin_batch();
    let bridge = WpsBridge::from_listener(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap());
    {
        let mut s = bridge.shared.lock().unwrap();
        s.connected = true; s.last_seen = now_ms(); s.page_index = 3; s.page_total = 20;
        s.can_jump = true; s.deck_epoch = 10;
    }
    let update = WpsUpdate { connected: true, events: vec![] };
    let mut timers = Timers::new();
    let mut tick = |a: &mut App| tick_shared(&rt, a, &mut ip, Some(&bridge), &update, &mut timers, false);
    tick(&mut app);
    open_page_picker(&rt, &mut app, false);
    let edit = ui::page_picker_rect(&app);
    assert_eq!(ui::page_picker_thumb_rect(&app).h, 0);
    assert!(ui::page_picker_control_rect(&app, ui::PagePickerHit::Wheel).h >= 88);
    bridge.shared.lock().unwrap().real_pos = 3;
    tick(&mut app);
    assert!(ui::page_picker_thumb_rect(&app).h > 0 && ui::page_picker_rect(&app).w > edit.w);
    bridge.shared.lock().unwrap().real_pos = -1;
    app.thumb_target = None;
    tick(&mut app);
    assert_eq!(ui::page_picker_rect(&app), edit);
    assert!(app.thumb_target.is_none() && app.thumb_waiting.is_none());
    app.page_picker.as_mut().unwrap().set_selected(8);
    commit_page_picker(&rt, &mut app, Some(&bridge));
    let state = bridge.shared.lock().unwrap();
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.queue[0].1, "GOTO 8 10");
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
fn tool_track_previews_on_drag_and_commits_only_when_the_owner_releases() {
    for right in [false, true] {
        for board in [false, true] {
            for touch in [false, true] {
                let (rt, mut app, mut ip) = ui_fixture();
                app.sb_y = 60;
                if board { app.toggle_whiteboard(); switch_to_cursor(&rt, &mut app); }
                app.drawing_tools.set_expanded(true);
                app.drawing_tools.settle();
                app.tool_selector.advance_at(Instant::now() + Duration::from_secs(1));
                let center = |button| {
                    let r = ui::control_rect(&app, ui::Control::Sidebar(button, right)).unwrap();
                    (r.x + r.w / 2, r.y + r.h / 2)
                };
                let (x, cursor) = center(Btn::Cursor);
                let (_, pen) = center(Btn::Pen);
                let (_, eraser) = center(Btn::Eraser);
                let event = |app: &mut App, ip: &mut Ip, y, kind| {
                    if touch { handle_touch(&rt, app, ip, None, 4, x, y, kind, 0.0); }
                    else { match kind {
                        TouchKind::Begin => on_press(&rt, app, ip, None, x, y, 1),
                        TouchKind::Update => on_motion(&rt, app, ip, x, y),
                        TouchKind::End => on_release(&rt, app, ip, None, x, y, 1),
                    }}
                };
                event(&mut app, &mut ip, cursor, TouchKind::Begin);
                rt.dirty.borrow_mut().clear();
                event(&mut app, &mut ip, (pen + eraser) / 2, TouchKind::Update);
                assert_eq!(app.mode, 0);
                assert!((app.tool_selector.position(app.mode) - 1.5).abs() < 0.03);
                for side in [false, true] {
                    assert!(rt.dirty.borrow().contains(&ui::tool_track_rect(&app, side).with_paint_margin()));
                }
                if touch {
                    for kind in [TouchKind::Begin, TouchKind::End] {
                        handle_touch(&rt, &mut app, &mut ip, None, 9, x, pen, kind, 0.0);
                    }
                    assert_eq!(app.mode, 0);
                }
                event(&mut app, &mut ip, eraser, TouchKind::End);
                assert_eq!(app.mode, 2);
                assert!(!app.eraser_popup_visible && !app.is_drawing && !app.page_has_ink);
                // The auxiliary drawer can still be opening. The fixed segments stay live.
                event(&mut app, &mut ip, eraser, TouchKind::Begin);
                event(&mut app, &mut ip, eraser, TouchKind::End);
                assert!(app.eraser_popup_visible && app.popup_on_right == right);
                event(&mut app, &mut ip, eraser, TouchKind::Begin);
                event(&mut app, &mut ip, pen, TouchKind::End);
                assert_eq!(app.mode, 1, "a coalesced release can select the next segment");
                assert!(!app.eraser_popup_visible && !app.pen_popup_visible);
                event(&mut app, &mut ip, pen, TouchKind::Begin);
                event(&mut app, &mut ip, eraser, TouchKind::Update);
                event(&mut app, &mut ip, pen, TouchKind::End);
                assert_eq!(app.mode, 1);
                assert!(!app.pen_popup_visible, "dragging back does not count as tapping the pen again");
            }
        }
    }
}

#[test]
fn leaving_the_track_or_ending_the_slideshow_discards_a_pending_mode_change() {
    let (rt, mut app, mut ip) = ui_fixture();
    app.sb_y = 60;
    app.drawing_tools.set_expanded(true);
    app.drawing_tools.settle();
    let (x, pen) = button_center(&app, Btn::Pen);
    let (_, eraser) = button_center(&app, Btn::Eraser);
    let (_, next) = button_center(&app, Btn::Next);
    for (release_x, release_y) in [(x, next), (app.screen_w / 2, eraser), (app.screen_w - x, eraser)] {
        on_press(&rt, &mut app, &mut ip, None, x, pen, 1);
        on_motion(&rt, &mut app, &mut ip, x, eraser);
        on_release(&rt, &mut app, &mut ip, None, release_x, release_y, 1);
        assert_eq!((app.mode, app.current_slide), (0, 1));
        assert_eq!(app.tool_selector.position(app.mode), 0.0);
        assert!(!app.page_has_ink && !app.is_drawing);
    }
    for mode in [0, 1] {
        app.set_tool_mode(mode);
        on_press(&rt, &mut app, &mut ip, None, x, pen, 1);
        on_motion(&rt, &mut app, &mut ip, x, eraser);
        finish_presentation(&rt, &mut app);
        on_release(&rt, &mut app, &mut ip, None, x, eraser, 1);
        assert_eq!(app.mode, 0);
        assert!(!app.eraser_popup_visible);
    }
}

#[test]
fn each_sidebar_key_keeps_a_distinct_48_by_48_touch_target() {
    let (_, mut app, _) = ui_fixture();
    app.wps_connected = true;
    app.wps_page_index = 6;
    app.wps_page_total = 24;
    app.wps_can_jump = true;
    app.wps_deck_epoch = 1;
    app.sb_y = 60;
    for scale in [0.6, 1.0, 1.4] {
        app.sb_scale = scale;
        for open in [false, true] {
            app.drawing_tools.set_expanded(open);
            app.drawing_tools.settle();
            for right in [false, true] {
                let keys = ui::sidebar_buttons(&app, right).into_iter()
                    .map(|(button, r)| (ui::Control::Sidebar(button, right), r))
                    .chain([(ui::Control::PageCount(right), ui::sidebar_page_rect(&app, right))]);
                for (key, r) in keys {
                    let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
                    for x in [cx - 24, cx + 23] {
                        for y in [cy - 24, cy + 23] {
                            assert_eq!(ui::control_at(&app, x, y), Some(key), "{key:?} at scale {scale}");
                        }
                    }
                }
            }
        }
    }
    for p in [ui_theme::LIGHT, ui_theme::DARK] {
        for face in [p.paper, p.surface] {
            assert!(ui_theme::contrast(p.control_border(), face) >= 3.0);
        }
    }
}

#[test]
fn a_short_touchscreen_can_scroll_to_the_last_action_without_triggering_clear() {
    let (rt, mut app, mut ip) = ui_fixture();
    app.configure_geometry(640, 360, 1.0);
    app.sb_y = 77;
    app.drawing_tools.set_expanded(true);
    app.drawing_tools.settle();
    paint::stroke_tapered(&mut app, (300, 80), (360, 100), 5, 5);
    let ink = app.canvas.data().to_vec();
    let (x, clear_y) = button_center(&app, Btn::Clear);
    handle_touch(&rt, &mut app, &mut ip, None, 3, x, clear_y, TouchKind::Begin, 0.0);
    handle_touch(&rt, &mut app, &mut ip, None, 3, x, 20, TouchKind::End, 0.0);
    assert!(ip.pending_ui.is_none() && !ip.dragging);
    assert_eq!(app.canvas.data(), ink);
    assert!(ui::sidebar_rect(&app, false).y < 0);
    on_scroll(&rt, &mut app, &ip, x, 100, 10.0);
    let rail = ui::sidebar_rect(&app, false);
    assert_eq!(rail.y + rail.h, app.screen_h);
    for right in [false, true] {
        let photo = ui::Control::Sidebar(Btn::Screenshot, right);
        let key = ui::control_rect(&app, photo).unwrap();
        assert_eq!(ui::control_at(&app, key.x + key.w / 2, key.y + key.h / 2), Some(photo));
    }
    assert_eq!(app.sb_y, 77, "scrolling does not change the normal docking position");
}

#[test]
fn pen_palette_uses_the_same_cells_for_paint_feedback_and_picking_on_either_edge() {
    let (_, mut app, _) = ui_fixture();
    app.pen_popup_visible = true;
    for (w, h, display_scale, rail_scale) in [(1280, 720, 1.0, 1.0), (800, 600, 1.25, 0.6),
        (640, 360, 1.5, 1.4), (320, 200, 2.0, 1.0)] {
        app.configure_geometry(w, h, display_scale);
        app.sb_scale = rail_scale;
        for right in [false, true] {
            app.popup_on_right = right;
            for anchor in [0, h] {
                app.sb_y = anchor;
                let panel = ui::pen_popup_rect(&app);
                assert!(panel.x >= 0 && panel.x + panel.w <= w && panel.y >= 0 && panel.y + panel.h <= h);
                let controls = (0..app::COLORS.len()).map(PopupHit::Color)
                    .chain((0..app::PEN_SIZES.len()).map(PopupHit::PenSize));
                for hit in controls {
                    let key = ui::Control::Popup(hit);
                    let cell = ui::control_rect(&app, key).unwrap();
                    for x in [cell.x, cell.x + cell.w - 1] {
                        for y in [cell.y, cell.y + cell.h - 1] {
                            assert!(panel.contains(x, y));
                            assert_eq!(ui::control_at(&app, x, y), Some(key));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn eraser_palette_touch_selects_each_width_and_clears_only_on_release() {
    for side in [false, true] {
        let (rt, mut app, mut ip) = ui_fixture();
        rt.begin_batch();
        app.mode = 2;
        app.eraser_popup_visible = true;
        app.popup_on_right = side;
        for i in 0..app::ERASER_SIZES.len() {
            let hit = ui::Control::Popup(PopupHit::EraserSize(i));
            let r = ui::control_rect(&app, hit).unwrap();
            assert!(r.w >= 48 && r.h >= 48);
            let (x, y) = (r.x + 1, r.y + r.h - 1);
            assert_eq!(ui::control_at(&app, x, y), Some(hit));
            handle_touch(&rt, &mut app, &mut ip, None, 1, x, y, TouchKind::Begin, 0.0);
            handle_touch(&rt, &mut app, &mut ip, None, 1, x, y, TouchKind::End, 0.0);
            assert_eq!(app.eraser_width(), app::ERASER_SIZES[i]);
            assert!(app.eraser_popup_visible);
            assert!(rt.dirty.borrow().iter().all(|r| r.w < app.screen_w));
        }
        app.canvas.pixels_mut()[0] = tiny_skia::ColorU8::from_rgba(0, 0, 0, 255).premultiply();
        app.page_has_ink = true;
        let r = ui::control_rect(&app, ui::Control::Popup(PopupHit::ClearAll)).unwrap();
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        handle_touch(&rt, &mut app, &mut ip, None, 1, x, y, TouchKind::Begin, 0.0);
        assert!(app.page_has_ink);
        handle_touch(&rt, &mut app, &mut ip, None, 1, x, y, TouchKind::End, 0.0);
        assert!(app.can_restore_cleared() && !app.eraser_popup_visible);
    }
}

#[test]
fn rapid_pen_retap_opens_the_palette_during_drawer_expansion() {
    for right in [false, true] {
        for touch in [false, true] {
            for (at_bottom, leave, drift) in [(false, false, 0), (false, false, 7), (false, true, 0), (true, false, 0)] {
                let (rt, mut app, mut ip) = ui_fixture();
                if at_bottom { app.sb_y = app.screen_h - app.sb_height(); }
                let pen = ui::control_rect(&app, ui::Control::Sidebar(Btn::Pen, right)).unwrap();
                let (x, y) = (pen.x + pen.w / 2, pen.y + pen.h / 2);
                let event = |app: &mut App, ip: &mut Ip, kind, (x, y)| {
                    if touch { handle_touch(&rt, app, ip, None, 1, x, y, kind, 0.0); }
                    else { match kind {
                        TouchKind::Begin => on_press(&rt, app, ip, None, x, y, 1),
                        TouchKind::End => on_release(&rt, app, ip, None, x, y, 1),
                        TouchKind::Update => on_motion(&rt, app, ip, x, y),
                    }}
                };
                event(&mut app, &mut ip, TouchKind::Begin, (x, y));
                event(&mut app, &mut ip, TouchKind::End, (x, y));
                assert_eq!(app.mode, 1);
                assert!(app.drawing_tools.animating() && !app.pen_popup_visible);
                app.drawing_tools.advance_at(Instant::now() + Duration::from_millis(80));
                event(&mut app, &mut ip, TouchKind::Begin, (x, y));
                if leave { on_motion(&rt, &mut app, &mut ip, -1, -1); }
                event(&mut app, &mut ip, TouchKind::Update, (x, y + drift));
                event(&mut app, &mut ip, TouchKind::End, (x, y + drift));
                assert!(app.pen_popup_visible && app.popup_on_right == right,
                    "right={right} touch={touch} at_bottom={at_bottom} leave={leave} drift={drift} mode={}", app.mode);
                let hit = ui::Control::Popup(PopupHit::Color(9));
                let cell = ui::control_rect(&app, hit).unwrap();
                let swatch = (cell.x + cell.w / 2, cell.y + cell.h / 2);
                event(&mut app, &mut ip, TouchKind::Begin, swatch);
                app.drawing_tools.settle();
                assert_eq!(ui::control_rect(&app, hit), Some(cell));
                event(&mut app, &mut ip, TouchKind::End, swatch);
                assert_eq!(app.cur_color, 9);
                assert!(app.pen_popup_visible && !app.is_drawing && !app.page_has_ink);
            }
        }
    }
}

#[test]
fn a_presenter_can_pick_a_black_fine_pen_from_the_gutters_then_return_to_cursor() {
    for right in [false, true] {
        for touch in [false, true] {
            let (rt, mut app, mut ip) = ui_fixture();
            rt.begin_batch();
            app.cur_pen = app::PEN_SIZES.len() - 1;
            let tap = |app: &mut App, ip: &mut Ip, x, y| {
                if touch {
                    handle_touch(&rt, app, ip, None, 1, x, y, TouchKind::Begin, 0.0);
                    handle_touch(&rt, app, ip, None, 1, x, y, TouchKind::End, 0.0);
                } else {
                    on_press(&rt, app, ip, None, x, y, 1);
                    on_release(&rt, app, ip, None, x, y, 1);
                }
            };
            let pen = ui::control_rect(&app, ui::Control::Sidebar(Btn::Pen, right)).unwrap();
            let (x, y) = (pen.x - 2, pen.y + pen.h / 2);
            tap(&mut app, &mut ip, x, y);
            assert_eq!(app.mode, 1);
            app.drawing_tools.settle();
            tap(&mut app, &mut ip, x, y);
            assert!(app.pen_popup_visible && app.popup_on_right == right);
            for hit in [PopupHit::Color(9), PopupHit::PenSize(0)] {
                let cell = ui::control_rect(&app, ui::Control::Popup(hit)).unwrap();
                rt.dirty.borrow_mut().clear();
                tap(&mut app, &mut ip, cell.x + 1, cell.y + 1);
                assert!(rt.dirty.borrow().iter().all(|r| r.w < app.screen_w));
                if matches!(hit, PopupHit::Color(_)) {
                    for side in [false, true] {
                        let tip = ui::control_rect(&app, ui::Control::Sidebar(Btn::Pen, side)).unwrap();
                        assert!(rt.dirty.borrow().contains(&tip.with_paint_margin()));
                    }
                }
            }
            assert_eq!(app.pen_width(), 2);
            assert_eq!(app::COLORS[app.cur_color], (0, 0, 0));
            assert!(app.pen_popup_visible && !app.page_has_ink);
            let ended = WpsUpdate { connected: true, events: vec![WpsEvent::SlideshowEnd] };
            tick_shared(&rt, &mut app, &mut ip, None, &ended, &mut Timers::new(), false);
            assert_eq!(app.mode, 0);
            assert!(!app.pen_popup_visible);
        }
    }
}

#[test]
fn a_delayed_palette_frame_reaches_full_opacity_and_releases_the_animation_timer() {
    let (rt, mut app, mut ip) = ui_fixture();
    rt.begin_batch();
    app.mode = 1;
    app.pen_popup_visible = true;
    app.open_anim = now_ms() - ui::POPUP_FADE_MS - 1000;
    let panel = ui::pen_popup_rect(&app).with_paint_margin();
    let update = WpsUpdate { connected: false, events: vec![] };
    let mut timers = Timers::new();
    tick_shared(&rt, &mut app, &mut ip, None, &update, &mut timers, false);
    assert_eq!(app.open_anim, 0);
    assert!(rt.dirty.borrow().contains(&panel));
    rt.dirty.borrow_mut().clear();
    tick_shared(&rt, &mut app, &mut ip, None, &update, &mut timers, false);
    assert!(!rt.dirty.borrow().contains(&panel));
}

#[test]
fn both_clear_controls_keep_restore_reachable_and_whiteboard_ready() {
    for whiteboard in [false, true] {
        for popup_clear in [false, true] {
            let (rt, mut app, mut ip) = ui_fixture();
            assert!(!app.drawing_tools.expanded());
            let shown = |app: &App, b| ui::control_rect(app, ui::Control::Sidebar(b, false)).is_some();
            assert!([Btn::Cursor, Btn::Pen].iter().all(|&b| shown(&app, b)));
            assert!([Btn::Eraser, Btn::Clear, Btn::Undo, Btn::Restore].iter().all(|&b| !shown(&app, b)));
            if whiteboard { handle_btn(&rt, &mut app, None, Btn::Whiteboard, false); }
            else { handle_btn(&rt, &mut app, None, Btn::Pen, false); }
            assert_eq!(app.mode, 1);
            assert!(app.drawing_tools.expanded());
            assert!(!app.pen_popup_visible, "first pen selection must not also open its settings");
            app.drawing_tools.settle();
            assert!([Btn::Eraser, Btn::Clear, Btn::Undo].iter().all(|&b| shown(&app, b)));
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
            assert_eq!(app.drawing_tools.expanded(), whiteboard);
            assert!(!app.pen_popup_visible && !app.eraser_popup_visible);
            assert_eq!(app.whiteboard, whiteboard);
            assert_eq!(app.mode, if whiteboard { 1 } else { 0 });
            app.drawing_tools.settle();
            assert!(shown(&app, Btn::Restore) && shown(&app, Btn::Next));
            assert_eq!(shown(&app, Btn::Eraser), whiteboard);
            assert_eq!(shown(&app, Btn::Undo), whiteboard);
            app.load_page(app.current_slide);
            assert!(!app.page_has_ink, "cleared content must not return from its cache");
            if whiteboard {
                switch_to_cursor(&rt, &mut app);
                assert!(shown(&app, Btn::Restore) && !shown(&app, Btn::Eraser) && !shown(&app, Btn::Undo));
                switch_to_draw(&rt, &mut app, 1);
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
            assert!(app.drawing_tools.expanded(), "leave the navigation keys in place during the dwell");
            resolve_drawing_tools_at(&rt, &mut app, Instant::now() + Duration::from_secs(1));
            assert_eq!(app.drawing_tools.expanded(), mode != 0 || target_ink);
            go_prev(&rt, &mut app, None);
            resolve_drawing_tools_at(&rt, &mut app, Instant::now() + Duration::from_secs(1));
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
            assert!(!app.drawing_tools.expanded());
            resolve_drawing_tools_at(&rt, &mut app, Instant::now() + Duration::from_secs(1));
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
fn rapid_cursor_paging_keeps_the_key_under_the_finger_and_ignores_skipped_ink() {
    for right in [false, true] {
        let (rt, mut app, _) = fixture(true);
        app.configure_geometry(1280, 900, 1.0);
        let mut ip = Ip::default();
        app.sb_y = 0;
        rt.begin_batch();
        paint::stroke_tapered(&mut app, (100, 100), (130, 100), 5, 5);
        app.slide_cache.insert(2, app.canvas.clone());
        app.clear_canvas();
        let next = ui::Control::Sidebar(Btn::Next, right);
        let key = ui::control_rect(&app, next).unwrap();
        let (x, y) = (key.x + key.w / 2, key.y + key.h / 2);
        let touch = |app: &mut App, ip: &mut Ip, kind| handle_touch(&rt, app, ip, None, 1, x, y, kind, 0.0);
        touch(&mut app, &mut ip, TouchKind::Begin);
        touch(&mut app, &mut ip, TouchKind::End);
        assert_eq!(app.current_slide, 2);
        assert!(app.page_has_ink);
        touch(&mut app, &mut ip, TouchKind::Begin);
        resolve_drawing_tools_at(&rt, &mut app, Instant::now() + Duration::from_secs(2));
        assert!(!app.drawing_tools.expanded());
        assert_eq!(ui::control_rect(&app, next), Some(key));
        touch(&mut app, &mut ip, TouchKind::End);
        assert_eq!(app.current_slide, 3);
        resolve_drawing_tools_at(&rt, &mut app, Instant::now() + Duration::from_secs(2));
        assert!(!app.drawing_tools.expanded());
        go_prev(&rt, &mut app, None);
        handle_btn(&rt, &mut app, None, Btn::Pen, right);
        assert_eq!(app.mode, 1);
        assert!(app.drawing_tools.expanded(), "selecting the pen bypasses page dwell");
    }
}

#[test]
fn undoing_the_last_mark_in_cursor_mode_waits_before_folding() {
    let (rt, mut app, _) = fixture(true);
    switch_to_draw(&rt, &mut app, 1);
    app.begin_undo();
    paint::stroke_tapered(&mut app, (20, 20), (80, 20), 6, 6);
    switch_to_cursor(&rt, &mut app);
    resolve_drawing_tools_auto_update(&rt, &mut app);
    handle_btn(&rt, &mut app, None, Btn::Undo, false);
    resolve_drawing_tools_auto_update(&rt, &mut app);
    assert!(app.drawing_tools.expanded());
    resolve_drawing_tools_at(&rt, &mut app, Instant::now() + Duration::from_secs(1));
    assert!(!app.drawing_tools.expanded() && !app.page_has_ink && !app.can_restore_cleared());
}

#[test]
fn compact_restore_belongs_to_the_cleared_page_and_works_from_either_rail() {
    for right in [false, true] {
        let (rt, mut app, _) = fixture(true);
        app.configure_geometry(1280, 900, 1.0);
        let mut ip = Ip::default();
        rt.begin_batch();
        switch_to_draw(&rt, &mut app, 1);
        paint::stroke_tapered(&mut app, (100, 100), (130, 100), 5, 5);
        let ink = app.canvas.clone();
        clear_drawing(&rt, &mut app);
        let recover = ui::Control::Sidebar(Btn::Restore, right);
        assert!(ui::control_rect(&app, recover).is_some());
        go_next(&rt, &mut app, None);
        assert!(!ui::control_enabled(&app, recover) && ui::control_rect(&app, recover).is_none());
        resolve_drawing_tools_at(&rt, &mut app, Instant::now() + Duration::from_secs(1));
        app.drawing_tools.settle();
        go_prev(&rt, &mut app, None);
        resolve_drawing_tools_at(&rt, &mut app, Instant::now() + Duration::from_secs(1));
        app.drawing_tools.settle();
        let key = ui::control_rect(&app, recover).unwrap();
        let pen = ui::control_rect(&app, ui::Control::Sidebar(Btn::Pen, right)).unwrap();
        assert_eq!(key.y, pen.y + pen.h + 6);
        assert!(ui::control_rect(&app, ui::Control::Sidebar(Btn::Eraser, right)).is_none());
        let (x, y) = (key.x + key.w / 2, key.y + key.h / 2);
        assert_eq!(ui::control_at(&app, x, y), Some(recover));
        on_press(&rt, &mut app, &mut ip, None, x, y, 1);
        on_release(&rt, &mut app, &mut ip, None, x, y, 1);
        assert_eq!(app.canvas.data(), ink.data());
        assert_eq!(app.mode, 0);
        assert!(app.drawing_tools.expanded() && !app.can_restore_cleared());
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
    assert!(app.drawing_tools.expanded());
    resolve_drawing_tools_at(&rt, &mut app, Instant::now() + Duration::from_secs(1));
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
    resolve_drawing_tools_at(&rt, &mut app, Instant::now() + Duration::from_secs(1));
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

#[cfg(unix)]
#[test]
fn settings_default_to_the_sharp_overlay_path_on_every_backend() {
    // 3.0/3.1 的设置面板画在覆盖层上，几何直接落在设备像素；受管窗口会被
    // KWin/UKUI 按全局缩放位图放大而发虚，所以只在显式要求时才打开。
    assert!(!super::use_native_settings(false, false));
    assert!(super::use_native_settings(false, true));
    assert!(!super::use_native_settings(true, false));
    assert!(!super::use_native_settings(true, true));
}

#[test]
fn seewo_avoidance_blocks_expanding_and_restores_it() {
    let (rt, mut app, _) = ui_fixture();
    app.collapsed = true;
    app.avoid_active = true;
    expand_sidebars(&rt, &mut app);
    assert!(app.collapsed, "avoid mode must refuse to open the toolbar");
    apply_input_shape(&rt, &app);
    let backend = rt.backend.as_any().downcast_ref::<TestBackend>().unwrap();
    assert!(backend.input.borrow().is_empty(), "avoid mode must not capture input");
    app.avoid_active = false;
    expand_sidebars(&rt, &mut app);
    assert!(!app.collapsed);
}

#[test]
fn expanding_a_tucked_sidebar_reopens_drawing_tools_when_ink_exists() {
    let (rt, mut app, _) = ui_fixture();
    app.canvas.data_mut()[0..4].copy_from_slice(&[255, 0, 0, 255]);
    app.page_has_ink = true;
    app.mode = 1;
    app.drawing_tools.set_expanded(true);
    collapse_sidebars(&rt, &mut app);
    assert!(app.collapsed);
    assert!(!app.drawing_tools.expanded(), "tucking the sidebar folds the tools");
    expand_sidebars(&rt, &mut app);
    assert!(app.drawing_tools.expanded(), "ink must reopen the tools after expanding");

    let (rt, mut app, _) = ui_fixture();
    app.canvas.data_mut().fill(0);
    app.page_has_ink = false;
    app.mode = 1;
    app.drawing_tools.set_expanded(true);
    collapse_sidebars(&rt, &mut app);
    expand_sidebars(&rt, &mut app);
    assert!(!app.drawing_tools.expanded(), "a blank page keeps the tools folded");
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
fn collapsed_tab_repaints_just_the_two_faces_until_auto_expand() {
    let (rt, mut app, _) = ui_fixture();
    app.set_sidebar_collapsed(true);
    app.sidebar_anim_start = None;
    let start = app.sidebar_collapsed_since.unwrap();
    let half = start + Duration::from_secs(60);
    rt.begin_batch();
    tick_sidebar_countdown(&rt, &mut app, half);
    assert_eq!(app.sidebar_countdown, 128);
    assert_eq!(rt.dirty.borrow().as_slice(), &[false, true].map(|right| ui::sidebar_tab_face(&app, right).with_paint_margin()));
    rt.begin_batch();
    tick_sidebar_countdown(&rt, &mut app, half + Duration::from_millis(1));
    assert!(rt.dirty.borrow().is_empty());
    tick_sidebar_countdown(&rt, &mut app, start + Duration::from_secs(120));
    assert!(!app.collapsed && app.sidebar_countdown == 0);
}

#[test]
fn slim_collapsed_tab_keeps_its_edge_touch_target() {
    for right in [false, true] {
        let (rt, mut app, mut ip) = ui_fixture();
        for scale in [0.6, 1.0, 1.4] {
            app.sb_scale = scale;
            app.set_sidebar_collapsed(true);
            app.sidebar_anim_start = None;
            let hit = ui::sidebar_rect(&app, right);
            let face = ui::sidebar_tab_face(&app, right);
            assert!(hit.w >= 48 && face.h > 2 * face.w);
            assert!(face.x > hit.x && face.x + face.w < hit.x + hit.w);
            let x = if right { hit.x + 1 } else { hit.x + hit.w - 2 };
            let y = hit.y + hit.h / 2;
            assert!(!face.contains(x, y));
            assert_eq!(ui::control_at(&app, x, y), Some(ui::Control::Sidebar(Btn::Collapse, right)));
            for kind in [TouchKind::Begin, TouchKind::End] {
                handle_touch(&rt, &mut app, &mut ip, None, 1, x, y, kind, 1.0);
            }
            assert!(!app.collapsed);
        }
    }
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
fn outside_click_dismisses_cursor_popups() {
    let (rt, mut app, mut ip) = ui_fixture();
    app.pen_popup_visible = true;
    apply_input_shape(&rt, &app);
    assert!(dismiss_capture(&app));
    let panel = ui::pen_popup_rect(&app);
    let (x, y) = (panel.x + panel.w + 8, panel.y + panel.h / 2);
    assert!(!over_ui(&app, x, y));
    on_press(&rt, &mut app, &mut ip, None, x, y, 1);
    on_release(&rt, &mut app, &mut ip, None, x, y, 1);
    assert!(!app.pen_popup_visible);
    assert!(!app.is_drawing && !app.page_has_ink);
    assert!(!dismiss_capture(&app));
}

#[test]
fn whiteboard_bottom_bar_keeps_only_guides_and_selection() {
    let (_rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    let select = ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::Select)).unwrap();
    assert_eq!(ui::control_at(&app, select.x + 4, select.y + 4),
        Some(ui::Control::Whiteboard(ui::WhiteboardControl::Select)));
    assert!(ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::Empty)).is_some());
}

#[test]
fn whiteboard_sidebar_has_no_collapse_button_and_enters_expanded() {
    let (_rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.collapsed = true;
    app.toggle_whiteboard();
    assert!(!app.collapsed, "whiteboard must be expanded so its controls are reachable");
    let buttons: Vec<ui::Btn> = [false, true].into_iter()
        .flat_map(|right| ui::sidebar_buttons(&app, right).into_iter().map(|(b, _)| b))
        .collect();
    assert!(!buttons.contains(&ui::Btn::Collapse));
    app.toggle_whiteboard();
    let buttons: Vec<ui::Btn> = [false, true].into_iter()
        .flat_map(|right| ui::sidebar_buttons(&app, right).into_iter().map(|(b, _)| b))
        .collect();
    assert!(buttons.contains(&ui::Btn::Collapse));
}

fn tap_board(rt: &Rt, app: &mut App, ip: &mut Ip, hit: ui::WhiteboardControl) {
    let r = ui::control_rect(app, ui::Control::Whiteboard(hit)).expect("visible whiteboard control");
    let (x,y) = (r.x+r.w/2,r.y+r.h/2);
    assert_eq!(ui::control_at(app,x,y),Some(ui::Control::Whiteboard(hit)));
    on_press(rt,app,ip,None,x,y,1);
    on_release(rt,app,ip,None,x,y,1);
}

#[test]
fn compact_geometry_values_validate_and_undo_in_order() {
    use ui::WhiteboardControl as W;
    use app::{ConicKind as K, ConicDirection as D};
    use value_input::Key;
    let (rt,mut a,_) = fixture(false);
    a.configure_geometry(320,240,1.25);
    a.toggle_whiteboard();
    let mut ip = Ip::default();
    tap_board(&rt,&mut a,&mut ip,W::Ellipse);
    tap_board(&rt,&mut a,&mut ip,W::Direction(K::Ellipse,D::Vertical));
    let initial = a.coord.unwrap();
    tap_board(&rt,&mut a,&mut ip,W::ParameterTab(WbSlider::Shape));
    tap_board(&rt,&mut a,&mut ip,W::Value(WbSlider::Shape));
    value_input_key(&rt,&mut a,Key::Character('9'));
    value_input_key(&rt,&mut a,Key::Enter);
    assert!(!a.settings_ui.editor.as_ref().unwrap().error.is_empty());
    assert_eq!(a.coord.unwrap(),initial);
    value_input_key(&rt,&mut a,Key::SelectAll);
    for ch in "0.72".chars() { value_input_key(&rt,&mut a,Key::Character(ch)); }
    value_input_key(&rt,&mut a,Key::Enter);
    assert!(a.settings_ui.editor.is_none());
    let edited = a.coord.unwrap();
    let (major, minor) = conic::ellipse_axes(edited.shape,edited.size);
    assert!((minor/major-0.72).abs()<0.001);
    tap_board(&rt,&mut a,&mut ip,W::Done);
    tap_board(&rt,&mut a,&mut ip,W::Ellipse);
    assert!(a.coord.unwrap().editing && a.conic_menu.is_none());
    tap_board(&rt,&mut a,&mut ip,W::Parabola);
    tap_board(&rt,&mut a,&mut ip,W::Direction(K::Parabola,D::Left));
    tap_board(&rt,&mut a,&mut ip,W::Delete);
    a.undo_last();
    assert_eq!(a.coord.unwrap().direction,D::Left);
    a.undo_last();
    assert_eq!(a.coord.unwrap().shape,edited.shape);
    assert_eq!(a.coord.unwrap().conic,K::Ellipse);
    a.undo_last();
    assert_eq!(a.coord.unwrap().shape,initial.shape);
    a.undo_last();
    assert!(a.coord.is_none());
}

#[test]
fn empty_selection_retries_and_a_cancelled_touch_menu_leaves_no_ink() {
    use ui::WhiteboardControl as W;
    let (rt,mut a,_) = fixture(false);
    a.configure_geometry(640,360,1.);
    a.toggle_whiteboard();
    let mut ip = Ip::default();
    tap_board(&rt,&mut a,&mut ip,W::Hyperbola);
    handle_touch(&rt,&mut a,&mut ip,None,3,140,65,TouchKind::Begin,1.);
    handle_touch(&rt,&mut a,&mut ip,None,3,140,65,TouchKind::End,0.);
    assert!(a.conic_menu.is_none() && a.coord.is_none() && !a.page_has_ink);
    tap_board(&rt,&mut a,&mut ip,W::Select);
    on_press(&rt,&mut a,&mut ip,None,100,80,1);
    on_release(&rt,&mut a,&mut ip,None,140,110,1);
    assert!(a.select_mode && a.selection_missed && a.floating.is_none());
    tap_board(&rt,&mut a,&mut ip,W::SelCancel);
    assert!(!a.select_mode && !a.page_has_ink);
}

#[test]
fn guide_drag_releases_once_and_can_be_recentred_without_changing_scale() {
    use ui::WhiteboardControl as W;
    let (rt,mut a,_) = fixture(false);
    a.configure_geometry(800,600,1.);
    a.toggle_whiteboard();
    let mut ip = Ip::default();
    tap_board(&rt,&mut a,&mut ip,W::Empty);
    let before = a.coord.unwrap();
    let (x,y) = (before.origin_x as i32,before.origin_y as i32);
    on_press(&rt,&mut a,&mut ip,None,x,y,1);
    on_motion(&rt,&mut a,&mut ip,x+12,y+10);
    on_release(&rt,&mut a,&mut ip,None,x+12,y+10,3);
    on_release(&rt,&mut a,&mut ip,None,x+41,y+17,1);
    assert_eq!(a.coord.unwrap().origin_x,before.origin_x+41.);
    tap_board(&rt,&mut a,&mut ip,W::ResetView);
    assert_eq!(a.coord.unwrap(),before);
    a.undo_last();
    assert_eq!(a.coord.unwrap().origin_y,before.origin_y+17.);
    a.undo_last();
    assert_eq!(a.coord.unwrap(),before);
}

#[test]
fn whiteboard_shape_menu_creates_the_selected_conic_in_edit_mode() {
    let (rt, mut app, _) = fixture(false);
    app.configure_geometry(640, 360, 1.0);
    app.toggle_whiteboard();
    let mut ip = Ip::default();
    let ellipse = ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::Ellipse)).unwrap();
    on_press(&rt, &mut app, &mut ip, None, ellipse.x + 8, ellipse.y + 8, 1);
    on_release(&rt, &mut app, &mut ip, None, ellipse.x + 8, ellipse.y + 8, 1);
    assert_eq!(app.conic_menu, Some(crate::app::ConicKind::Ellipse));
    assert!(app.coord.is_none());
    let option = ui::control_rect(&app, ui::Control::Whiteboard(ui::WhiteboardControl::Direction(
        crate::app::ConicKind::Ellipse, crate::app::ConicDirection::Vertical))).unwrap();
    on_press(&rt, &mut app, &mut ip, None, option.x + 8, option.y + 8, 1);
    on_release(&rt, &mut app, &mut ip, None, option.x + 8, option.y + 8, 1);
    let coord = app.coord.expect("ellipse creates a coordinate guide");
    assert_eq!(coord.conic, crate::app::ConicKind::Ellipse);
    assert_eq!(coord.direction, crate::app::ConicDirection::Vertical);
    assert!(coord.editing);
    assert_eq!(app.conic_menu, None);
}
