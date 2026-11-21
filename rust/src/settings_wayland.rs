//! xdg-shell preferences window for native Wayland, including GNOME.
use std::cell::RefCell;
use std::time::{Duration, Instant};

use smithay_client_toolkit::{
    activation::{ActivationHandler, ActivationState, RequestData},
    compositor::{CompositorHandler, CompositorState},
    delegate_activation, delegate_compositor, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::{calloop::EventLoop, calloop_wayland_source::WaylandSource},
    registry::{ProvidesRegistryState, RegistryState},
    seat::{
        pointer::{CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemedPointer, ThemeSpec},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        xdg::{window::{Window, WindowConfigure, WindowDecorations, WindowHandler}, XdgShell},
        WaylandSurface,
    },
    shm::{slot::{Buffer, SlotPool}, Shm, ShmHandler},
};
use tiny_skia::{Pixmap, PixmapPaint, Transform};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch},
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols::xdg::shell::client::xdg_toplevel::ResizeEdge;

use crate::{app::App, settings_window::WindowEvent, ui::{self, Ctx, R},
    ui_theme::{c, alpha}, value_input::Key, Rt};

const BORDER: i32 = 7;
const TITLE: i32 = 40;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decorations {
    Panel,
    TitleBar,
}

impl Decorations {
    fn insets(self) -> (i32, i32) {
        match self {
            Self::Panel => (0, 0),
            Self::TitleBar => (BORDER, TITLE),
        }
    }

    fn content_size(self, (width, height): (i32, i32)) -> (i32, i32) {
        let (border, title) = self.insets();
        ((width - 2 * border).max(1), (height - title - border).max(1))
    }

    fn content_point(self, x: i32, y: i32) -> (i32, i32) {
        let (border, title) = self.insets();
        (x - border, y - title)
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(crate) struct WaylandWindow {
    transport: RefCell<Transport>,
    body: RefCell<Option<Pixmap>>,
    frame: RefCell<Option<Pixmap>>,
}

struct Repeat {
    keycode: u32,
    key: Key,
    next: Instant,
    interval: Duration,
}

struct Transport {
    event_loop: EventLoop<'static, State>,
    state: State,
    queue: QueueHandle<State>,
    connection: Connection,
}

struct Devices {
    seat: wl_seat::WlSeat,
    pointer: Option<ThemedPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    touch: Option<wl_touch::WlTouch>,
    focused: bool,
    modifiers: u32,
}

struct State {
    window: Window,
    compositor: CompositorState,
    registry: RegistryState,
    seats: SeatState,
    outputs: OutputState,
    shm: Shm,
    activation: Option<ActivationState>,
    devices: Vec<Devices>,
    touch: Option<(wl_touch::WlTouch, i32, i32, i32)>,
    events: Vec<WindowEvent>,
    pool: SlotPool,
    buffers: [Option<Buffer>; 2],
    size: (i32, i32),
    decorations: Decorations,
    panel_drag_region: Option<R>,
    scale: f64,
    configured: bool,
    dirty: bool,
    maximized: bool,
    // Keep the protocol object for the lifetime of the window.
    _fractional: Option<WpFractionalScaleV1>,
    received_preferred_scale: bool,
    viewport: Option<WpViewport>,
    activation_retry: bool,
    repeat_rate: i32,
    repeat_delay: i32,
    repeat: Option<Repeat>,
}

impl WaylandWindow {
    pub(crate) fn open(app: &App, decorations: Decorations) -> Result<Self> {
        let connection = Connection::connect_to_env()?;
        let (globals, event_queue) = registry_queue_init(&connection)?;
        let queue = event_queue.handle();
        let event_loop = EventLoop::try_new()?;
        WaylandSource::new(connection.clone(), event_queue).insert(event_loop.handle())?;
        let compositor = CompositorState::bind(&globals, &queue)?;
        let shell = XdgShell::bind(&globals, &queue)?;
        let shm = Shm::bind(&globals, &queue)?;
        let pool = SlotPool::new(4096, &shm)?;
        let surface = compositor.create_surface(&queue);
        let viewport = globals.bind::<WpViewporter, _, _>(&queue, 1..=1, ()).ok()
            .map(|viewporter| viewporter.get_viewport(&surface, &queue, ()));
        let fractional = viewport.as_ref().and_then(|_| {
            let manager = globals.bind::<WpFractionalScaleManagerV1, _, _>(&queue, 1..=1, ()).ok()?;
            Some(manager.get_fractional_scale(&surface, &queue, ()))
        });
        let scale = crate::wayland::render_scale(app.scale, viewport.is_some(), surface.version());
        let window = shell.create_window(surface, WindowDecorations::ClientOnly, &queue);
        window.set_title("Sidera · 偏好设置");
        window.set_app_id("sidera");
        let (border, title) = decorations.insets();
        let min_width = 360 + 2 * border;
        let min_height = 340 + title + border;
        window.set_min_size(Some((min_width as u32, min_height as u32)));
        window.commit();
        connection.flush()?;
        let state = State {
            window, compositor, registry: RegistryState::new(&globals),
            seats: SeatState::new(&globals, &queue), outputs: OutputState::new(&globals, &queue),
            shm, activation: ActivationState::bind(&globals, &queue).ok(),
            devices: Vec::new(), touch: None, events: Vec::new(), pool, buffers: [None, None],
            size: ((860 + 2 * border).min(app.screen_w - 24).max(min_width),
                (620 + title + border).min(app.screen_h - 24).max(min_height)),
            decorations, panel_drag_region: None,
            scale, configured: false, dirty: true, maximized: false,
            _fractional: fractional, received_preferred_scale: false, viewport, activation_retry: false,
            repeat_rate: 25, repeat_delay: 600, repeat: None,
        };
        let result = Self {
            transport: RefCell::new(Transport { event_loop, state, queue, connection }),
            body: RefCell::new(None), frame: RefCell::new(None),
        };
        result.activate();
        Ok(result)
    }

    pub(crate) fn activate(&self) {
        let transport = self.transport.borrow();
        if let Some(activation) = &transport.state.activation {
            activation.request_token(&transport.queue, RequestData {
                seat_and_serial: None,
                surface: Some(transport.state.window.wl_surface().clone()),
                app_id: Some("sidera".into()),
            });
            let _ = transport.connection.flush();
        }
    }

    pub(crate) fn logical_size(&self) -> (i32, i32) {
        let transport = self.transport.borrow();
        transport.state.decorations.content_size(transport.state.size)
    }

    pub(crate) fn invalidate(&self) {
        self.transport.borrow_mut().state.dirty = true;
    }

    pub(crate) fn events(&self, app: &App) -> Result<Vec<WindowEvent>> {
        // Layout reads logical_size() from this window.
        let drag_region = crate::settings_panel::window_drag_region(app);
        let mut transport = self.transport.borrow_mut();
        let Transport { event_loop, state, .. } = &mut *transport;
        state.panel_drag_region = drag_region;
        event_loop.dispatch(Duration::ZERO, state)?;
        let now = Instant::now();
        while let Some(repeat) = state.repeat.as_mut() {
            if now < repeat.next { break; }
            state.events.push(WindowEvent::Key(repeat.key));
            repeat.next += repeat.interval;
            // Never replay a long backlog after a stall.
            if repeat.next <= now { repeat.next = now + repeat.interval; }
        }
        Ok(std::mem::take(&mut state.events))
    }

    pub(crate) fn paint(&self, rt: &Rt, app: &App) {
        // Drawing the common UI consults logical_size(), so release the
        // transport borrow before calling the renderer.
        let (size, decorations, render_scale, buffer) = {
            let mut transport = self.transport.borrow_mut();
            let state = &mut transport.state;
            if !state.configured || !state.dirty { return; }
            let Some(buffer) = state.available_buffer() else { return };
            (state.size, state.decorations, state.scale, buffer)
        };
        let (width, height) = size;
        let (content_w, content_h) = decorations.content_size(size);
        let pixels = |logical: i32| (f64::from(logical) * render_scale).round().max(1.0) as u32;
        let (body_w, body_h) = (pixels(content_w), pixels(content_h));
        let mut body_cache = self.body.borrow_mut();
        if body_cache.as_ref().is_none_or(|body| body.width() != body_w || body.height() != body_h) {
            *body_cache = Pixmap::new(body_w, body_h);
        }
        let Some(body) = body_cache.as_mut() else { return };
        if decorations == Decorations::Panel {
            ui::draw_settings_surface(app, &rt.fonts, rt.icon.as_ref(), body, render_scale as f32, tiny_skia::Color::TRANSPARENT);
            self.present(body, buffer);
            return;
        }
        ui::draw_settings_window(app, &rt.fonts, rt.icon.as_ref(), body, render_scale as f32);
        let (frame_w, frame_h) = (pixels(width), pixels(height));
        let mut frame_cache = self.frame.borrow_mut();
        if frame_cache.as_ref().is_none_or(|frame| frame.width() != frame_w || frame.height() != frame_h) {
            *frame_cache = Pixmap::new(frame_w, frame_h);
        }
        let Some(frame) = frame_cache.as_mut() else { return };
        let palette = app.palette();
        frame.fill(c(palette.surface));
        let border = (BORDER as f64 * render_scale).round() as i32;
        let title = (TITLE as f64 * render_scale).round() as i32;
        frame.draw_pixmap(border, title, body.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        let mut ctx = Ctx { palette, pm: frame, tr: Transform::from_scale(render_scale as f32, render_scale as f32) };
        ctx.text(&rt.fonts.for_settings(), "Sidera · 偏好设置", 16.0, 25.0, 14.0, c(palette.text));
        let close = close_rect(width);
        ctx.fill_round(close, 5.0, alpha(palette.border, 100));
        let x = (close.x + close.w / 2) as f32;
        let y = (close.y + close.h / 2) as f32;
        ctx.stroke_line((x - 4.0, y - 4.0), (x + 4.0, y + 4.0), c(palette.text), 1.5);
        ctx.stroke_line((x - 4.0, y + 4.0), (x + 4.0, y - 4.0), c(palette.text), 1.5);
        self.present(frame, buffer);
    }

    fn present(&self, frame: &Pixmap, buffer: usize) {
        let mut transport = self.transport.borrow_mut();
        if let Err(error) = transport.state.present(frame, buffer) {
            log::warn!("无法绘制 Wayland 设置窗口：{error}");
            transport.state.events.push(WindowEvent::Close);
        }
        let _ = transport.connection.flush();
    }
}

fn close_rect(width: i32) -> R {
    R { x: width - BORDER - 32, y: 8, w: 28, h: 24 }
}

impl State {
    fn available_buffer(&mut self) -> Option<usize> {
        self.buffers.iter().position(|slot|
            slot.as_ref().is_none_or(|buffer| buffer.canvas(&mut self.pool).is_some()))
    }

    fn present(&mut self, frame: &Pixmap, index: usize) -> Result<()> {
        let width = frame.width() as i32;
        let height = frame.height() as i32;
        if self.buffers[index].as_ref().is_none_or(|buffer|
            buffer.height() != height || buffer.stride() != width * 4) {
            self.buffers[index] = None;
            let (buffer, _) = self.pool.create_buffer(width, height, width * 4, wl_shm::Format::Argb8888)?;
            self.buffers[index] = Some(buffer);
        }
        let buffer = self.buffers[index].as_ref().unwrap();
        let Some(bytes) = buffer.canvas(&mut self.pool) else { return Ok(()) };
        for (dst, src) in bytes.chunks_exact_mut(4).zip(frame.data().chunks_exact(4)) {
            dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
        }
        let surface = self.window.wl_surface();
        if let Some(viewport) = self.viewport.as_ref() {
            viewport.set_destination(self.size.0, self.size.1);
            if surface.version() >= 3 { surface.set_buffer_scale(1); }
        } else if surface.version() >= 3 {
            surface.set_buffer_scale(self.scale.round().max(1.0) as i32);
        }
        buffer.attach_to(surface)?;
        surface.damage(0, 0, self.size.0, self.size.1);
        self.window.commit();
        self.dirty = false;
        Ok(())
    }

    fn resize_edge(&self, x: i32, y: i32) -> Option<ResizeEdge> {
        if self.maximized { return None; }
        let (width, height) = self.size;
        let horizontal = if x < BORDER { -1 } else if x >= width - BORDER { 1 } else { 0 };
        let vertical = if y < BORDER { -1 } else if y >= height - BORDER { 1 } else { 0 };
        Some(match (horizontal, vertical) {
            (-1, -1) => ResizeEdge::TopLeft,
            (1, -1) => ResizeEdge::TopRight,
            (-1, 1) => ResizeEdge::BottomLeft,
            (1, 1) => ResizeEdge::BottomRight,
            (-1, _) => ResizeEdge::Left,
            (1, _) => ResizeEdge::Right,
            (_, -1) => ResizeEdge::Top,
            (_, 1) => ResizeEdge::Bottom,
            _ => return None,
        })
    }

    /// Return true when the press belongs to the shared settings contents.
    fn press(&mut self, seat: &wl_seat::WlSeat, serial: u32, x: i32, y: i32) -> bool {
        let titled = self.decorations == Decorations::TitleBar;
        if let Some(edge) = self.resize_edge(x, y) {
            self.window.resize(seat, serial, edge);
        } else if titled && close_rect(self.size.0).contains(x, y) {
            self.events.push(WindowEvent::Close);
        } else if (titled && y < TITLE) || (!titled && self.panel_drag_region.is_some_and(|r| r.contains(x, y))) {
            self.window.move_(seat, serial);
        } else {
            let (cx, cy) = self.decorations.content_point(x, y);
            self.events.push(WindowEvent::Press(cx, cy));
            return true;
        }
        false
    }
}

impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.events.push(WindowEvent::Close);
    }

    fn configure(&mut self, _: &Connection, queue: &QueueHandle<Self>, _: &Window,
        configure: WindowConfigure, _: u32) {
        let (width, height) = self.size;
        let size = (configure.new_size.0.map_or(width, |w| w.get() as i32),
            configure.new_size.1.map_or(height, |h| h.get() as i32));
        if crate::app::valid_geometry(size.0, size.1, self.scale) {
            if size != self.size {
                self.size = size;
                self.events.push(WindowEvent::Resized);
            }
            self.maximized = configure.is_maximized() || configure.is_fullscreen();
            self.configured = true;
            self.dirty = true;
            // The first activation token can be requested before the surface is
            // mapped; ask again once the compositor has configured it.
            if !self.activation_retry {
                self.activation_retry = true;
                if let Some(activation) = &self.activation {
                    activation.request_token(queue, RequestData {
                        seat_and_serial: None,
                        surface: Some(self.window.wl_surface().clone()),
                        app_id: Some("sidera".into()),
                    });
                }
            }
        }
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        // Use output scale while waiting for the surface's first preference.
        if self.received_preferred_scale { return; }
        let factor = crate::wayland::render_scale(f64::from(factor), self.viewport.is_some(), surface.version());
        if crate::app::valid_geometry(self.size.0, self.size.1, factor) {
            self.scale = factor;
            self.dirty = true;
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState { &mut self.outputs }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState { &mut self.seats }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if !self.devices.iter().any(|device| device.seat == seat) {
            self.devices.push(Devices { seat, pointer: None, keyboard: None, touch: None, focused: false, modifiers: 0 });
        }
    }
    fn new_capability(&mut self, connection: &Connection, queue: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        self.new_seat(connection, queue, seat.clone());
        let Some(device) = self.devices.iter_mut().find(|device| device.seat == seat) else { return };
        match capability {
            Capability::Pointer if device.pointer.is_none() => {
                device.pointer = self.seats.get_pointer_with_theme(queue, &seat, self.shm.wl_shm(),
                    self.compositor.create_surface(queue), ThemeSpec::System).ok();
            }
            Capability::Keyboard if device.keyboard.is_none() => device.keyboard = Some(seat.get_keyboard(queue, ())),
            Capability::Touch if device.touch.is_none() => device.touch = Some(seat.get_touch(queue, ())),
            _ => {}
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if let Some(device) = self.devices.iter_mut().find(|device| device.seat == seat) {
            match capability {
                Capability::Pointer => { device.pointer.take(); }
                Capability::Keyboard => {
                    if let Some(keyboard) = device.keyboard.take().filter(|keyboard| keyboard.version() >= 3) { keyboard.release(); }
                    device.focused = false;
                    device.modifiers = 0;
                }
                Capability::Touch => {
                    if let Some(touch) = device.touch.take() {
                        if touch.version() >= 3 { touch.release(); }
                        if self.touch.as_ref().is_some_and(|(active, ..)| *active == touch) { self.touch = None; }
                    }
                }
                _ => {}
            }
            self.events.push(WindowEvent::FocusLost);
        }
    }
    fn remove_seat(&mut self, connection: &Connection, queue: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        for capability in [Capability::Pointer, Capability::Keyboard, Capability::Touch] {
            self.remove_capability(connection, queue, seat.clone(), capability);
        }
        self.devices.retain(|device| device.seat != seat);
    }
}

impl PointerHandler for State {
    fn pointer_frame(&mut self, conn: &Connection, _: &QueueHandle<Self>,
        pointer: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        let Some(index) = self.devices.iter().position(|device|
            device.pointer.as_ref().is_some_and(|themed| themed.pointer() == pointer)) else { return };
        let seat = self.devices[index].seat.clone();
        for event in events {
            if &event.surface != self.window.wl_surface() { continue; }
            let (x, y) = (event.position.0 as i32, event.position.1 as i32);
            let (local_x, local_y) = self.decorations.content_point(x, y);
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    let cursor = match self.resize_edge(x, y) {
                        Some(ResizeEdge::TopLeft) => CursorIcon::NwResize,
                        Some(ResizeEdge::TopRight) => CursorIcon::NeResize,
                        Some(ResizeEdge::BottomLeft) => CursorIcon::SwResize,
                        Some(ResizeEdge::BottomRight) => CursorIcon::SeResize,
                        Some(ResizeEdge::Left | ResizeEdge::Right) => CursorIcon::EwResize,
                        Some(_) => CursorIcon::NsResize,
                        None => CursorIcon::Default,
                    };
                    if let Some(pointer) = &self.devices[index].pointer { let _ = pointer.set_cursor(conn, cursor); }
                    self.events.push(WindowEvent::Motion(local_x, local_y));
                }
                PointerEventKind::Leave { .. } => self.events.push(WindowEvent::Motion(-1, -1)),
                PointerEventKind::Press { button: 0x110, serial, .. } => { self.press(&seat, serial, x, y); }
                PointerEventKind::Release { button: 0x110, .. } => self.events.push(WindowEvent::Release(local_x, local_y)),
                PointerEventKind::Axis { vertical, .. } if local_y >= 0 => {
                    let pages = if vertical.discrete != 0 { f64::from(vertical.discrete) } else { vertical.absolute / 40.0 };
                    self.events.push(WindowEvent::Scroll(local_x, local_y, pages));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for State {
    fn event(state: &mut Self, touch: &wl_touch::WlTouch, event: wl_touch::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_touch::Event::Down { serial, surface, id, x, y, .. }
                if state.touch.is_none() && &surface == state.window.wl_surface() => {
                let Some(seat) = state.devices.iter().find(|device| device.touch.as_ref() == Some(touch))
                    .map(|device| device.seat.clone()) else { return };
                let (x, y) = (x as i32, y as i32);
                if state.press(&seat, serial, x, y) {
                    let (cx, cy) = state.decorations.content_point(x, y);
                    state.touch = Some((touch.clone(), id, cx, cy));
                }
            }
            wl_touch::Event::Motion { id, x, y, .. } => {
                if let Some((active, contact, cx, cy)) = &mut state.touch {
                    if active == touch && *contact == id {
                        (*cx, *cy) = state.decorations.content_point(x as i32, y as i32);
                        state.events.push(WindowEvent::Motion(*cx, *cy));
                    }
                }
            }
            wl_touch::Event::Up { id, .. } if state.touch.as_ref().is_some_and(|(active, contact, ..)|
                active == touch && *contact == id) => {
                let (_, _, x, y) = state.touch.take().unwrap();
                state.events.push(WindowEvent::Release(x, y));
            }
            wl_touch::Event::Cancel if state.touch.as_ref().is_some_and(|(active, ..)| active == touch) => {
                state.touch = None;
                state.events.push(WindowEvent::FocusLost);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(state: &mut Self, keyboard: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let Some(device) = state.devices.iter_mut().find(|device| device.keyboard.as_ref() == Some(keyboard))
            else { return };
        match event {
            wl_keyboard::Event::Enter { surface, .. } => device.focused = &surface == state.window.wl_surface(),
            wl_keyboard::Event::Leave { .. } => {
                device.focused = false;
                device.modifiers = 0;
                state.repeat = None;
                state.events.push(WindowEvent::FocusLost);
            }
            wl_keyboard::Event::Modifiers { mods_depressed, mods_latched, mods_locked, .. } => {
                device.modifiers = mods_depressed | mods_latched | (mods_locked & 2);
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                state.repeat_rate = rate;
                state.repeat_delay = delay;
            }
            wl_keyboard::Event::Key { key: code, state: WEnum::Value(wl_keyboard::KeyState::Pressed), .. }
                if device.focused => {
                if let Some(key) = crate::wayland::text_key(code, device.modifiers) {
                    state.events.push(WindowEvent::Key(key));
                    state.repeat = (state.repeat_rate > 0).then(|| {
                        let delay = Duration::from_millis(state.repeat_delay.max(0) as u64);
                        Repeat {
                            keycode: code, key, next: Instant::now() + delay,
                            interval: Duration::from_secs_f64(1.0 / state.repeat_rate as f64),
                        }
                    });
                } else {
                    state.repeat = None;
                }
            }
            wl_keyboard::Event::Key { key: code, state: WEnum::Value(wl_keyboard::KeyState::Released), .. } => {
                if state.repeat.as_ref().is_some_and(|repeat| repeat.keycode == code) {
                    state.repeat = None;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WpFractionalScaleManagerV1, ()> for State {
    fn event(_: &mut Self, _: &WpFractionalScaleManagerV1, _: <WpFractionalScaleManagerV1 as Proxy>::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpFractionalScaleV1, ()> for State {
    fn event(state: &mut Self, _: &WpFractionalScaleV1, event: wp_fractional_scale_v1::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            let factor = (scale as f64 / 120.0).clamp(1.0, 4.0);
            if crate::app::valid_geometry(state.size.0, state.size.1, factor) {
                state.received_preferred_scale = true;
                state.scale = factor;
                state.dirty = true;
            }
        }
    }
}

impl Dispatch<WpViewporter, ()> for State {
    fn event(_: &mut Self, _: &WpViewporter, _: <WpViewporter as Proxy>::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpViewport, ()> for State {
    fn event(_: &mut Self, _: &WpViewport, _: <WpViewport as Proxy>::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl ActivationHandler for State {
    type RequestData = RequestData;
    fn new_token(&mut self, token: String, _: &RequestData) {
        if let Some(activation) = &self.activation {
            activation.activate::<Self>(self.window.wl_surface(), token);
        }
    }
}
impl ShmHandler for State { fn shm_state(&mut self) -> &mut Shm { &mut self.shm } }
impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState { &mut self.registry }
    smithay_client_toolkit::registry_handlers![OutputState, SeatState];
}
delegate_activation!(State);
delegate_compositor!(State);
delegate_output!(State);
delegate_pointer!(State);
delegate_registry!(State);
delegate_seat!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);
delegate_xdg_window!(State);
