//! xdg-shell preferences window for native Wayland, including GNOME.
use std::cell::RefCell;
use std::time::Duration;

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
use wayland_protocols::xdg::shell::client::xdg_toplevel::ResizeEdge;

use crate::{app::App, settings_window::WindowEvent, ui::{self, Ctx, R}, ui_theme::{c, alpha}, Rt};

const BORDER: i32 = 7;
const TITLE: i32 = 40;
const MIN_WIDTH: u32 = (360 + BORDER * 2) as u32;
const MIN_HEIGHT: u32 = (340 + TITLE + BORDER) as u32;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(crate) struct WaylandWindow {
    transport: RefCell<Transport>,
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
    scale: i32,
    configured: bool,
    dirty: bool,
    maximized: bool,
}

impl WaylandWindow {
    pub(crate) fn open(app: &App) -> Result<Self> {
        let connection = Connection::connect_to_env()?;
        let (globals, event_queue) = registry_queue_init(&connection)?;
        let queue = event_queue.handle();
        let event_loop = EventLoop::try_new()?;
        WaylandSource::new(connection.clone(), event_queue).insert(event_loop.handle())?;
        let compositor = CompositorState::bind(&globals, &queue)?;
        let shell = XdgShell::bind(&globals, &queue)?;
        let shm = Shm::bind(&globals, &queue)?;
        let pool = SlotPool::new(4096, &shm)?;
        let window = shell.create_window(
            compositor.create_surface(&queue), WindowDecorations::ClientOnly, &queue,
        );
        window.set_title("Sidera · 偏好设置");
        window.set_app_id("sidera");
        window.set_min_size(Some((MIN_WIDTH, MIN_HEIGHT)));
        window.commit();
        connection.flush()?;
        let state = State {
            window, compositor, registry: RegistryState::new(&globals),
            seats: SeatState::new(&globals, &queue), outputs: OutputState::new(&globals, &queue),
            shm, activation: ActivationState::bind(&globals, &queue).ok(),
            devices: Vec::new(), touch: None, events: Vec::new(), pool, buffers: [None, None],
            size: ((860 + BORDER * 2).min(app.screen_w - 24).max(MIN_WIDTH as i32),
                (620 + TITLE + BORDER).min(app.screen_h - 24).max(MIN_HEIGHT as i32)),
            scale: 1, configured: false, dirty: true, maximized: false,
        };
        let result = Self { transport: RefCell::new(Transport { event_loop, state, queue, connection }) };
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
        let (width, height) = self.transport.borrow().state.size;
        ((width - 2 * BORDER).max(1), (height - TITLE - BORDER).max(1))
    }

    pub(crate) fn invalidate(&self) {
        self.transport.borrow_mut().state.dirty = true;
    }

    pub(crate) fn events(&self) -> Result<Vec<WindowEvent>> {
        let mut transport = self.transport.borrow_mut();
        let Transport { event_loop, state, .. } = &mut *transport;
        event_loop.dispatch(Duration::ZERO, state)?;
        Ok(std::mem::take(&mut state.events))
    }

    pub(crate) fn paint(&self, rt: &Rt, app: &App) {
        // Drawing the common UI consults logical_size(), so release the
        // transport borrow before calling the renderer.
        let (size, scale, buffer) = {
            let mut transport = self.transport.borrow_mut();
            let state = &mut transport.state;
            if !state.configured || !state.dirty { return; }
            let Some(buffer) = state.available_buffer() else { return };
            (state.size, state.scale, buffer)
        };
        let (width, height) = size;
        let (body_w, body_h) = self.logical_size();
        let Some(body) = ui::render_settings_window(app, &rt.fonts, rt.icon.as_ref(),
            (body_w * scale) as u32, (body_h * scale) as u32, scale as f32) else { return };
        let Some(mut frame) = Pixmap::new((width * scale) as u32, (height * scale) as u32) else { return };
        let palette = app.palette();
        frame.fill(c(palette.surface));
        frame.draw_pixmap(BORDER * scale, TITLE * scale, body.as_ref(),
            &PixmapPaint::default(), Transform::identity(), None);
        let mut ctx = Ctx { palette, pm: &mut frame, tr: Transform::from_scale(scale as f32, scale as f32) };
        ctx.text(&rt.fonts, "Sidera · 偏好设置", 16.0, 25.0, 14.0, c(palette.text));
        let close = close_rect(width);
        ctx.fill_round(close, 5.0, alpha(palette.border, 100));
        let x = (close.x + close.w / 2) as f32;
        let y = (close.y + close.h / 2) as f32;
        ctx.stroke_line((x - 4.0, y - 4.0), (x + 4.0, y + 4.0), c(palette.text), 1.5);
        ctx.stroke_line((x - 4.0, y + 4.0), (x + 4.0, y - 4.0), c(palette.text), 1.5);
        let mut transport = self.transport.borrow_mut();
        if let Err(error) = transport.state.present(&frame, buffer) {
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
            let (buffer, _) = self.pool.create_buffer(width, height, width * 4, wl_shm::Format::Xrgb8888)?;
            self.buffers[index] = Some(buffer);
        }
        let buffer = self.buffers[index].as_ref().unwrap();
        let Some(bytes) = buffer.canvas(&mut self.pool) else { return Ok(()) };
        for (dst, src) in bytes.chunks_exact_mut(4).zip(frame.data().chunks_exact(4)) {
            dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
        }
        let surface = self.window.wl_surface();
        if surface.version() >= 3 {
            surface.set_buffer_scale(self.scale);
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
        if let Some(edge) = self.resize_edge(x, y) {
            self.window.resize(seat, serial, edge);
        } else if close_rect(self.size.0).contains(x, y) {
            self.events.push(WindowEvent::Close);
        } else if y < TITLE {
            self.window.move_(seat, serial);
        } else {
            self.events.push(WindowEvent::Press(x - BORDER, y - TITLE));
            return true;
        }
        false
    }
}

impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.events.push(WindowEvent::Close);
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window,
        configure: WindowConfigure, _: u32) {
        let (width, height) = self.size;
        let size = (configure.new_size.0.map_or(width, |w| w.get() as i32),
            configure.new_size.1.map_or(height, |h| h.get() as i32));
        if crate::app::valid_geometry(size.0, size.1, f64::from(self.scale)) {
            if size != self.size {
                self.size = size;
                self.events.push(WindowEvent::Resized);
            }
            self.maximized = configure.is_maximized() || configure.is_fullscreen();
            self.configured = true;
            self.dirty = true;
        }
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        let factor = if surface.version() >= 3 { factor.clamp(1, 4) } else { 1 };
        if crate::app::valid_geometry(self.size.0, self.size.1, f64::from(factor)) {
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
            let (local_x, local_y) = (x - BORDER, y - TITLE);
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
                PointerEventKind::Axis { vertical, .. } if y >= TITLE => {
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
                    state.touch = Some((touch.clone(), id, x - BORDER, y - TITLE));
                }
            }
            wl_touch::Event::Motion { id, x, y, .. } => {
                if let Some((active, contact, cx, cy)) = &mut state.touch {
                    if active == touch && *contact == id {
                        (*cx, *cy) = (x as i32 - BORDER, y as i32 - TITLE);
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
                state.events.push(WindowEvent::FocusLost);
            }
            wl_keyboard::Event::Modifiers { mods_depressed, mods_latched, mods_locked, .. } => {
                device.modifiers = mods_depressed | mods_latched | (mods_locked & 2);
            }
            wl_keyboard::Event::Key { key, state: WEnum::Value(wl_keyboard::KeyState::Pressed), .. }
                if device.focused => {
                if let Some(key) = crate::wayland::text_key(key, device.modifiers) {
                    state.events.push(WindowEvent::Key(key));
                }
            }
            _ => {}
        }
    }
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
