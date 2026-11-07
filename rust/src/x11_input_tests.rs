//! Protocol tests for input shapes and the Mutter remap workaround.
//!
//! Include as a child module of x11.rs. Run with an isolated Xvfb server:
//! xvfb-run -a cargo test x11::input_protocol_tests -- --ignored --test-threads=1
//! These tests exercise real X11 requests. The opt-in below exercises the
//! workaround mechanics on Xvfb, not Mutter's pointer cache or visual behavior.

use super::*;
use crate::backend::{Backend, IRect};
use std::sync::{Mutex, OnceLock};
use x11rb::wrapper::ConnectionExt as _;

// XTest changes the server-wide pointer state, so serialize this module even
// when the test executable is invoked without --test-threads=1.
static SERVER: Mutex<()> = Mutex::new(());
// Xvfb resets when its last client disconnects. Keep an idle connection open
// so the next short-lived test client cannot race that server reset.
static SERVER_KEEPALIVE: OnceLock<RustConnection> = OnceLock::new();

fn connect_to_test_server() -> X11 {
    SERVER_KEEPALIVE.get_or_init(|| x11rb::connect(None).expect("keep isolated Xvfb alive").0);
    let x11 = X11::connect().expect("connect to isolated Xvfb");
    assert!(!x11.is_xwayland, "run protocol tests on isolated Xvfb");
    x11
}

fn platform(refresh: bool) -> X11Plat {
    let x11 = connect_to_test_server();
    let (win, gc) = x11.create_window(0, 0, 320, 240).unwrap();
    let mut plat = X11Plat::new(x11, win, gc).unwrap();
    assert!(!plat.refresh_input_regions);
    plat.refresh_input_regions = refresh;
    plat
}

fn input_rects(plat: &X11Plat) -> Vec<(i16, i16, u16, u16)> {
    shape::get_rectangles(&plat.x11.conn, plat.win, SK::INPUT)
        .unwrap()
        .reply()
        .unwrap()
        .rectangles
        .iter()
        .map(|r| (r.x, r.y, r.width, r.height))
        .collect()
}

fn assert_mapped(plat: &X11Plat) {
    assert_eq!(
        xproto::get_window_attributes(&plat.x11.conn, plat.win)
            .unwrap()
            .reply()
            .unwrap()
            .map_state,
        xproto::MapState::VIEWABLE,
    );
}

fn remap_events(plat: &X11Plat) -> (usize, usize) {
    // A reply orders all preceding requests before the event drain.
    xproto::get_input_focus(&plat.x11.conn).unwrap().reply().unwrap();
    let (mut unmapped, mut mapped) = (0, 0);
    while let Some(event) = plat.x11.conn.poll_for_event().unwrap() {
        match event {
            x11rb::protocol::Event::UnmapNotify(e) if e.window == plat.win => unmapped += 1,
            x11rb::protocol::Event::MapNotify(e) if e.window == plat.win => mapped += 1,
            _ => {}
        }
    }
    (unmapped, mapped)
}

struct HeldButton<'a> {
    x11: &'a X11,
    button: u8,
}

impl<'a> HeldButton<'a> {
    fn press(x11: &'a X11, button: u8) -> Self {
        xtest::fake_input(&x11.conn, xproto::BUTTON_PRESS_EVENT, button, 0, x11.root, 0, 0, 0)
            .unwrap().check().unwrap();
        Self { x11, button }
    }
}

impl Drop for HeldButton<'_> {
    fn drop(&mut self) {
        if let Ok(cookie) = xtest::fake_input(
            &self.x11.conn, xproto::BUTTON_RELEASE_EVENT, self.button,
            0, self.x11.root, 0, 0, 0,
        ) {
            let _ = cookie.check();
        }
    }
}

#[test]
#[ignore = "requires an isolated Xvfb server"]
fn input_shape_round_trips_partial_empty_and_full() {
    let _server = SERVER.lock().unwrap();
    let plat = platform(false);
    plat.set_input_region(&[IRect::new(0, 20, 32, 100), IRect::new(288, 20, 32, 100)], false);
    assert_eq!(input_rects(&plat), vec![(0, 20, 32, 100), (288, 20, 32, 100)]);
    plat.set_input_region(&[], false);
    assert!(input_rects(&plat).is_empty());
    plat.set_input_region(&[], true);
    assert_eq!(input_rects(&plat), vec![(0, 0, 320, 240)]);
    assert!(!plat.input_refresh_pending.get(), "ordinary X11 does not remap");
}

#[test]
#[ignore = "requires an isolated Xvfb server"]
fn numeric_keyboard_grab_survives_a_pending_input_region_refresh() {
    use crate::value_input::Key;
    let _server = SERVER.lock().unwrap();
    let plat = platform(true);
    plat.set_input_region(&[IRect::new(0, 20, 32, 100)], false);
    assert!(plat.grab_text_keyboard());
    assert!(!plat.refresh_input_window(true).unwrap());
    for (symbol, expected) in [(0x31, '1'), (0xffb1, '1'), (0xffae, '.')] {
        let keycode = plat.x11.keysym_to_keycode(symbol);
        assert_ne!(keycode, 0);
        assert_eq!(plat.x11.text_key(keycode, xproto::KeyButMask::MOD2), Some(Key::Character(expected)));
    }
    plat.ungrab_text_keyboard();
    assert!(plat.refresh_input_window(true).unwrap());
}

#[test]
#[ignore = "requires an isolated Xvfb server"]
fn wps_focus_distinguishes_documents_in_the_same_suite_window() {
    let _server = SERVER.lock().unwrap();
    let x11 = connect_to_test_server();
    let (window, _) = x11.create_window(0, 0, 320, 240).unwrap();
    x11.conn.change_property32(xproto::PropMode::REPLACE, x11.root, x11.net_active_window,
        AtomEnum::WINDOW, &[window]).unwrap().check().unwrap();
    x11.conn.change_property8(xproto::PropMode::REPLACE, window, AtomEnum::WM_CLASS,
        AtomEnum::STRING, b"wpsoffice\0wpsoffice\0").unwrap().check().unwrap();
    for (title, expected) in [
        ("课程.dps - WPS Office", true),
        ("笔记.pptx.docx - WPS Office", false),
        ("WPS Office", false),
        ("演示文稿1 - WPS Office", true),
    ] {
        x11.conn.change_property8(xproto::PropMode::REPLACE, window, x11.net_wm_name,
            AtomEnum::STRING, title.as_bytes()).unwrap().check().unwrap();
        assert_eq!(x11.is_wps_presentation_foreground(), expected, "{title}");
    }
    xproto::delete_property(&x11.conn, x11.root, x11.net_active_window).unwrap().check().unwrap();
}

#[test]
#[ignore = "requires an isolated Xvfb server"]
fn invalid_window_shape_updates_report_bad_window() {
    let _server = SERVER.lock().unwrap();
    let plat = platform(false);
    let unused_window = plat.x11.conn.generate_id().unwrap();
    for full in [false, true] {
        let error = plat.x11.set_input_shape(unused_window, &[], full).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<x11rb::errors::ReplyError>(),
            Some(x11rb::errors::ReplyError::X11Error(error))
                if error.error_kind == x11rb::protocol::ErrorKind::Window
        ), "expected BadWindow, got {error:?}");
    }
}

#[test]
#[ignore = "requires an isolated Xvfb server"]
fn unchanged_regions_do_not_schedule_another_remap() {
    let _server = SERVER.lock().unwrap();
    let plat = platform(true);
    let rects = [IRect::new(0, 20, 32, 100)];
    plat.set_input_region(&rects, false);
    assert!(plat.input_refresh_pending.get());
    assert!(plat.refresh_input_window(true).unwrap());
    plat.set_input_region(&rects, false);
    assert!(!plat.input_refresh_pending.get());
    assert!(!plat.refresh_input_window(true).unwrap());

    plat.set_input_region(&[], true);
    assert!(plat.refresh_input_window(true).unwrap());
    assert_eq!(input_rects(&plat), vec![(0, 0, 320, 240)]);
    // Rectangle arguments are irrelevant when the full-window shape is used.
    plat.set_input_region(&rects, true);
    assert!(!plat.input_refresh_pending.get());
    plat.set_input_region(&[], false);
    assert!(plat.input_refresh_pending.get(), "full and empty are distinct");
    assert!(plat.refresh_input_window(true).unwrap());
    assert!(input_rects(&plat).is_empty());
    assert_mapped(&plat);
}

#[test]
#[ignore = "requires an isolated Xvfb server"]
fn active_input_defers_without_unmapping() {
    let _server = SERVER.lock().unwrap();
    let plat = platform(true);
    plat.set_input_region(&[IRect::new(0, 20, 32, 100)], false);
    remap_events(&plat);
    assert!(!plat.refresh_input_window(false).unwrap());
    assert!(plat.input_refresh_pending.get());
    assert_mapped(&plat);
    assert_eq!(remap_events(&plat), (0, 0));
    assert!(plat.refresh_input_window(true).unwrap());
    assert_eq!(remap_events(&plat), (1, 1));
}

#[test]
#[ignore = "requires an isolated Xvfb server; injects server-wide input"]
fn held_buttons_defer_until_release_and_remap_preserves_shape() {
    let _server = SERVER.lock().unwrap();
    let plat = platform(true);
    for button in 1..=5 {
        let rects = [IRect::new(0, 20 + i32::from(button), 32, 100)];
        plat.set_input_region(&rects, false);
        let expected = input_rects(&plat);
        remap_events(&plat);
        let held = HeldButton::press(&plat.x11, button);
        let pointer = xproto::query_pointer(&plat.x11.conn, plat.x11.root)
            .unwrap().reply().unwrap();
        assert_ne!(u16::from(pointer.mask) & (1 << (7 + button)), 0);
        assert!(!plat.refresh_input_window(true).unwrap(), "button {button} is held");
        assert!(plat.input_refresh_pending.get());
        assert_mapped(&plat);
        assert_eq!(remap_events(&plat), (0, 0));
        drop(held);
        assert!(plat.refresh_input_window(true).unwrap());
        assert!(!plat.input_refresh_pending.get());
        assert_mapped(&plat);
        assert_eq!(input_rects(&plat), expected);
        assert_eq!(remap_events(&plat), (1, 1));
    }
}

#[test]
#[ignore = "requires an isolated Xvfb server"]
fn failed_shape_update_is_not_cached_or_scheduled() {
    let _server = SERVER.lock().unwrap();
    let plat = platform(true);
    xproto::destroy_window(&plat.x11.conn, plat.win).unwrap().check().unwrap();
    plat.set_input_region(&[IRect::new(0, 20, 32, 100)], false);
    assert!(plat.input_region.borrow().is_none());
    assert!(!plat.input_refresh_pending.get());
}

// Advertise a minimal EWMH identity only on the isolated test server. The
// XWAYLAND flag is overridden after connecting: this exercises the production
// constructor's selection path without claiming that Xvfb emulates Mutter.
fn split_platform() -> X11Plat {
    let mut x11 = connect_to_test_server();
    assert_eq!(x11.argb_depth, 32);
    let (win, gc) = x11.create_overlay().unwrap();
    let check = xproto::intern_atom(&x11.conn, false, b"_NET_SUPPORTING_WM_CHECK")
        .unwrap().reply().unwrap().atom;
    let name = xproto::intern_atom(&x11.conn, false, b"_NET_WM_NAME")
        .unwrap().reply().unwrap().atom;
    let utf8 = xproto::intern_atom(&x11.conn, false, b"UTF8_STRING")
        .unwrap().reply().unwrap().atom;
    for target in [x11.root, win] {
        x11.conn.change_property32(
            xproto::PropMode::REPLACE, target, check, AtomEnum::WINDOW, &[win],
        ).unwrap().check().unwrap();
    }
    x11.conn.change_property8(
        xproto::PropMode::REPLACE, win, name, utf8, b"GNOME Shell",
    ).unwrap().check().unwrap();
    x11.is_xwayland = true;
    let plat = X11Plat::new(x11, win, gc).unwrap();
    assert!(plat.refresh_input_regions);
    assert_ne!(plat.win, plat.render_win);
    plat
}

fn map_transitions(x11: &X11) -> Vec<(bool, Window)> {
    xproto::get_input_focus(&x11.conn).unwrap().reply().unwrap();
    let mut transitions = Vec::new();
    while let Some(event) = x11.conn.poll_for_event().unwrap() {
        match event {
            x11rb::protocol::Event::UnmapNotify(e) => transitions.push((false, e.window)),
            x11rb::protocol::Event::MapNotify(e) => transitions.push((true, e.window)),
            _ => {}
        }
    }
    transitions
}

#[test]
#[ignore = "requires an isolated Xvfb server; changes the root EWMH properties"]
fn mutter_split_visual_stays_mapped_and_never_accepts_input() {
    let _server = SERVER.lock().unwrap();
    let plat = split_platform();
    let role = xproto::intern_atom(&plat.x11.conn, true, b"_SIDERA_OVERLAY_ROLE")
        .unwrap().reply().unwrap().atom;
    for (win, expected) in [(plat.win, b"input".as_slice()), (plat.render_win, b"visual".as_slice())] {
        assert_eq!(xproto::get_property(
            &plat.x11.conn, false, win, role, AtomEnum::STRING, 0, 16,
        ).unwrap().reply().unwrap().value, expected);
        let geometry = xproto::get_geometry(&plat.x11.conn, win)
            .unwrap().reply().unwrap();
        assert_eq!((geometry.x, geometry.y), (0, 0));
        assert_eq!((geometry.width as i32, geometry.height as i32), (plat.x11.width, plat.x11.height));
    }
    map_transitions(&plat.x11);
    for full in [true, false, true, false] {
        plat.set_input_region(&[IRect::new(0, 20, 32, 100)], full);
        assert!(plat.refresh_input_window(true).unwrap());
        assert_eq!(map_transitions(&plat.x11), vec![(false, plat.win), (true, plat.win)]);
        assert_eq!(xproto::get_window_attributes(&plat.x11.conn, plat.render_win)
            .unwrap().reply().unwrap().map_state, xproto::MapState::VIEWABLE);
        assert!(shape::get_rectangles(&plat.x11.conn, plat.render_win, SK::INPUT)
            .unwrap().reply().unwrap().rectangles.is_empty());
        assert_mapped(&plat);
        let expected = if full {
            vec![(0, 0, plat.x11.width as u16, plat.x11.height as u16)]
        } else {
            vec![(0, 20, 32, 100)]
        };
        assert_eq!(input_rects(&plat), expected);
        let pixels = xproto::get_image(
            &plat.x11.conn, ImageFormat::Z_PIXMAP, plat.win,
            0, 0, 1, 1, u32::MAX,
        ).unwrap().reply().unwrap();
        assert!(pixels.data.iter().all(|byte| *byte == 0), "input surface stays transparent");
    }
}

#[test]
#[ignore = "requires an isolated Xvfb server; changes the root EWMH properties"]
fn mutter_split_raise_keeps_both_surfaces_above_other_windows() {
    let _server = SERVER.lock().unwrap();
    let plat = split_platform();
    let (other, _) = plat.x11.create_window(10, 10, 16, 16).unwrap();
    let before = xproto::query_tree(&plat.x11.conn, plat.x11.root)
        .unwrap().reply().unwrap().children;
    assert_eq!(before.last(), Some(&other));
    map_transitions(&plat.x11);
    plat.raise_overlays().unwrap();
    let after = xproto::query_tree(&plat.x11.conn, plat.x11.root)
        .unwrap().reply().unwrap().children;
    assert_eq!(&after[after.len() - 2..], &[plat.render_win, plat.win]);
    assert!(map_transitions(&plat.x11).is_empty(), "raising never remaps the visual surface");
}

#[test]
#[ignore = "requires an isolated Xvfb server; changes the root EWMH properties"]
fn mutter_split_present_draws_to_the_visual_surface() {
    let _server = SERVER.lock().unwrap();
    let plat = split_platform();
    // Xvfb has no compositor: temporarily put the visual window on top so its
    // pixels are not obscured. This checks routing, not compositor retention.
    xproto::configure_window(
        &plat.x11.conn, plat.render_win,
        &ConfigureWindowAux::new().stack_mode(xproto::StackMode::ABOVE),
    ).unwrap().check().unwrap();
    let mut pixels = tiny_skia::Pixmap::new(16, 16).unwrap();
    pixels.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
    plat.present(&pixels, 8, 8);
    let image = xproto::get_image(
        &plat.x11.conn, ImageFormat::Z_PIXMAP, plat.render_win,
        8, 8, 16, 16, u32::MAX,
    ).unwrap().reply().unwrap();
    assert_eq!(image.depth, 32);
    assert_eq!(image.data.len(), 16 * 16 * 4);
    assert!(image.data.chunks_exact(4).all(|pixel| pixel == [0, 0, 255, 255]));
    plat.raise_overlays().unwrap();
}
