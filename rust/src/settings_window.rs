//! Native settings windows, sharing one renderer and input path.
use std::cell::Cell;
use std::rc::Rc;
use tiny_skia::IntRect;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt, CreateGCAux,
    CreateWindowAux, EventMask, Gcontext, PropMode, StackMode, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

use crate::{app::App, ui, x11::{XResult, X11}, Ip, Rt, WpsBridge};
use crate::settings_wayland::WaylandWindow;

pub(crate) enum Window {
    X11(X11Window),
    Wayland(WaylandWindow),
}

pub(crate) enum WindowEvent {
    Close,
    Resized,
    FocusLost,
    Key(crate::value_input::Key),
    Press(i32, i32),
    Release(i32, i32),
    Motion(i32, i32),
    Scroll(i32, i32, f64),
}

impl Window {
    pub(crate) fn open(rt: &Rt, app: &App) -> XResult<Rc<Self>> {
        if !rt.backend.as_any().is::<crate::x11::X11Plat>() {
            match WaylandWindow::open(app) {
                Ok(window) => return Ok(Rc::new(Self::Wayland(window))),
                Err(error) => log::info!("原生设置窗口不可用：{error}"),
            }
        }
        Ok(Rc::new(Self::X11(X11Window::open(app)?)))
    }

    pub(crate) fn raise(&self) {
        match self {
            Self::X11(window) => window.raise(),
            Self::Wayland(window) => window.activate(),
        }
    }

    pub(crate) fn logical_size(&self) -> (i32, i32) {
        match self {
            Self::X11(window) => window.logical_size(),
            Self::Wayland(window) => window.logical_size(),
        }
    }

    pub(crate) fn invalidate(&self) {
        match self {
            Self::X11(window) => window.dirty.set(true),
            Self::Wayland(window) => window.invalidate(),
        }
    }

    pub(crate) fn paint(&self, rt: &Rt, app: &App) {
        match self {
            Self::X11(window) => window.paint(rt, app),
            Self::Wayland(window) => window.paint(rt, app),
        }
    }

    fn events(&self) -> XResult<Vec<WindowEvent>> {
        match self {
            Self::X11(window) => window.events(),
            Self::Wayland(window) => window.events(),
        }
    }
}

pub(crate) struct X11Window {
    x11: X11,
    id: u32,
    gc: Gcontext,
    size: Cell<(u16, u16)>,
    scale: f64,
    protocols: Atom,
    delete: Atom,
    dirty: Cell<bool>,
}

impl X11Window {
    fn open(app: &App) -> XResult<Self> {
        let x11 = X11::connect()?;
        let scale = x11.detect_scale();
        let pixels = |logical: i32| (f64::from(logical) * scale).round() as i32;
        let width = pixels(860).min((x11.width - 40).max(1)).max(1);
        let height = pixels(620).min((x11.height - 72).max(1)).max(1);
        let id = x11.conn.generate_id()?;
        let gc = x11.conn.generate_id()?;
        let atom = |name: &[u8]| -> XResult<Atom> {
            Ok(x11.conn.intern_atom(false, name)?.reply()?.atom)
        };
        let protocols = atom(b"WM_PROTOCOLS")?;
        let delete = atom(b"WM_DELETE_WINDOW")?;
        let net_name = atom(b"_NET_WM_NAME")?;
        let utf8 = atom(b"UTF8_STRING")?;
        x11.conn.create_window(
            x11.argb_depth, id, x11.root,
            ((x11.width - width) / 2) as i16, ((x11.height - height) / 2) as i16,
            width as u16, height as u16, 0, WindowClass::INPUT_OUTPUT, x11.argb_visual,
            &CreateWindowAux::new()
                .colormap(x11.cmap)
                .background_pixel(0xff000000 | app.palette().paper)
                .border_pixel(0)
                .event_mask(EventMask::EXPOSURE | EventMask::STRUCTURE_NOTIFY
                    | EventMask::FOCUS_CHANGE | EventMask::KEY_PRESS | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION | EventMask::LEAVE_WINDOW),
        )?.check()?;
        x11.conn.create_gc(gc, id, &CreateGCAux::new())?.check()?;
        x11.conn.change_property8(PropMode::REPLACE, id, AtomEnum::WM_NAME,
            AtomEnum::STRING, b"Sidera Settings")?.check()?;
        x11.conn.change_property8(PropMode::REPLACE, id, net_name, utf8,
            "Sidera · 偏好设置".as_bytes())?.check()?;
        x11.conn.change_property8(PropMode::REPLACE, id, AtomEnum::WM_CLASS,
            AtomEnum::STRING, b"sidera\0Sidera\0")?.check()?;
        x11.conn.change_property32(PropMode::REPLACE, id, protocols, AtomEnum::ATOM, &[delete])?.check()?;
        // ICCCM PMinSize. Settings reflow at this size, including the palette and dialogs.
        let mut hints = [0; 18];
        hints[0] = 1 << 4;
        hints[5] = pixels(360).min(width) as u32;
        hints[6] = pixels(340).min(height) as u32;
        x11.conn.change_property32(PropMode::REPLACE, id, AtomEnum::WM_NORMAL_HINTS,
            AtomEnum::WM_SIZE_HINTS, &hints)?.check()?;
        x11.conn.map_window(id)?.check()?;
        x11.conn.flush()?;
        Ok(Self {
            x11, id, gc, size: Cell::new((width as u16, height as u16)), scale,
            protocols, delete, dirty: Cell::new(true),
        })
    }

    fn raise(&self) {
        let _ = self.x11.conn.configure_window(self.id, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let event = ClientMessageEvent::new(32, self.id, self.x11.net_active_window, [1, 0, 0, 0, 0]);
        let _ = self.x11.conn.send_event(false, self.x11.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event);
        let _ = self.x11.conn.flush();
    }

    fn logical(&self, pixels: i16) -> i32 {
        (f64::from(pixels) / self.scale).floor() as i32
    }

    fn logical_size(&self) -> (i32, i32) {
        let (width, height) = self.size.get();
        let logical = |size| (f64::from(size) / self.scale).round().max(1.0) as i32;
        (logical(width), logical(height))
    }

    fn paint(&self, rt: &Rt, app: &App) {
        if !self.dirty.replace(false) {
            return;
        }
        let (width, height) = self.size.get();
        if !crate::app::valid_geometry(i32::from(width), i32::from(height), 1.0) {
            return;
        }
        let Some(frame) = ui::render_settings_window(
            app, &rt.fonts, rt.icon.as_ref(), width.into(), height.into(), self.scale as f32,
        ) else { return };
        // Respect the server's request limit when the window is maximized.
        let limit = self.x11.conn.maximum_request_bytes().saturating_sub(64);
        let rows = (limit / (usize::from(width) * 4)).max(1);
        if rows >= usize::from(height) {
            let _ = self.x11.put_pixmap(self.id, self.gc, &frame, 0, 0);
            return;
        }
        for y in (0..u32::from(height)).step_by(rows) {
            let lines = (u32::from(height) - y).min(rows as u32);
            let rect = IntRect::from_xywh(0, y as i32, width.into(), lines).unwrap();
            if let Some(strip) = frame.clone_rect(rect) {
                let _ = self.x11.put_pixmap(self.id, self.gc, &strip, 0, y as i32);
            }
        }
    }

    fn events(&self) -> XResult<Vec<WindowEvent>> {
        use WindowEvent::*;
        let mut events = Vec::new();
        for _ in 0..128 {
            let Some(event) = self.x11.conn.poll_for_event()? else { break };
            let event = match event {
                Event::ClientMessage(event) if event.type_ == self.protocols
                    && event.data.as_data32()[0] == self.delete => Close,
                Event::DestroyNotify(_) => Close,
                Event::ConfigureNotify(event) => {
                    let size = (event.width, event.height);
                    if self.size.replace(size) == size { continue; }
                    self.dirty.set(true);
                    Resized
                }
                Event::Expose(_) => { self.dirty.set(true); continue; }
                Event::FocusOut(_) => FocusLost,
                Event::KeyPress(event) => {
                    let Some(key) = self.x11.text_key(event.detail, event.state) else { continue };
                    Key(key)
                }
                Event::ButtonPress(event) => {
                    let (x, y) = (self.logical(event.event_x), self.logical(event.event_y));
                    match event.detail {
                        1 => Press(x, y),
                        4 => Scroll(x, y, -1.0),
                        5 => Scroll(x, y, 1.0),
                        _ => continue,
                    }
                }
                Event::ButtonRelease(event) if event.detail == 1 =>
                    Release(self.logical(event.event_x), self.logical(event.event_y)),
                Event::MotionNotify(event) =>
                    Motion(self.logical(event.event_x), self.logical(event.event_y)),
                Event::LeaveNotify(_) => Motion(-1, -1),
                _ => continue,
            };
            let closing = matches!(event, Close);
            events.push(event);
            if closing { break; }
        }
        Ok(events)
    }
}

fn finish_gesture(app: &mut App, ip: &mut Ip) {
    if ip.slider.take().is_some() {
        crate::app::save_settings(app);
    }
    ip.pending_ui = None;
    ip.panel_scroll = None;
    ip.picker_drag = None;
    app.press = None;
}

pub(crate) fn close(rt: &Rt, app: &mut App, ip: &mut Ip) {
    if app.settings_ui.window.take().is_none() {
        return;
    }
    finish_gesture(app, ip);
    app.settings_open = false;
    app.process_block_open = false;
    app.process_input_active = false;
    app.confirm_quit = false;
    app.show_diag = false;
    app.settings_ui.editor = None;
    app.settings_ui.picker = None;
    app.settings_ui.dialog = None;
    app.settings_ui.closing_at = 0;
    app.settings_ui.overlay_closing_at = 0;
    crate::apply_input_shape(rt, app);
    rt.redraw_full(app);
}

/// Returns true when a settings window supplied input to this display's App.
pub(crate) fn poll(rt: &Rt, app: &mut App, ip: &mut Ip, wps: Option<&WpsBridge>) -> bool {
    let Some(window) = app.settings_ui.window.clone() else { return false };
    let events = match window.events() {
        Ok(events) => events,
        Err(error) => {
            log::warn!("设置窗口连接已断开：{error}");
            close(rt, app, ip);
            return false;
        }
    };
    let mut input = false;
    for event in events {
        match event {
            WindowEvent::Close => {
                close(rt, app, ip);
                break;
            }
            WindowEvent::Resized => {
                finish_gesture(app, ip);
                if app.settings_ui.editor.as_ref().is_some_and(|editor|
                    crate::value_input::target_rect(app, editor.target).is_none()) {
                    app.settings_ui.editor = None;
                }
                if app.process_input_active && !ui::process_block_controls(app).iter()
                    .any(|(hit, _)| *hit == ui::ProcessHit::Input) {
                    app.process_input_active = false;
                }
                window.invalidate();
            }
            WindowEvent::FocusLost => {
                finish_gesture(app, ip);
                window.invalidate();
            }
            WindowEvent::Key(key) => input |= crate::text_input_key(rt, app, key),
            WindowEvent::Press(x, y) => {
                crate::on_press(rt, app, ip, wps, x, y, 1);
                input = true;
            }
            WindowEvent::Release(x, y) => {
                crate::on_release(rt, app, ip, wps, x, y, 1);
                input = true;
            }
            WindowEvent::Motion(x, y) => {
                crate::on_motion(rt, app, ip, x, y);
                input = true;
            }
            WindowEvent::Scroll(x, y, pages) => crate::on_scroll(rt, app, ip, x, y, pages),
        }
    }
    // The frame flush paints once after input, animation and theme updates.
    input
}
