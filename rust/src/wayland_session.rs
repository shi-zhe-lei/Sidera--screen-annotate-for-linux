// Per-output surfaces and per-seat devices for the layer-shell backend.
use std::collections::HashMap;
use wayland_client::protocol::wl_keyboard;
use crate::desktop::Desktop;

struct OutputSurface {
    id: String,
    output: wl_output::WlOutput,
    surface: wl_surface::WlSurface,
    fractional: Option<WpFractionalScaleV1>,
}
impl Drop for OutputSurface {
    fn drop(&mut self) { if let Some(scale) = self.fractional.take() { scale.destroy(); } }
}
impl Drop for WlPlat {
    fn drop(&mut self) { if let Some(viewport) = self.viewport.borrow_mut().take() { viewport.destroy(); } }
}
#[derive(Clone)]
struct Contact { id: i32, output: String, x: i32, y: i32 }
struct TouchAction { contact: Contact, kind: TouchKind }
struct SeatDevices {
    seat: wl_seat::WlSeat,
    pointer: Option<ThemedPointer>,
    touch: Option<wl_touch::WlTouch>,
    keyboard: Option<ZwpVirtualKeyboardV1>,
    text_keyboard: Option<wl_keyboard::WlKeyboard>,
    mods: u32,
    virtual_pointers: HashMap<String, ZwlrVirtualPointerV1>,
    contacts: HashMap<i32, Contact>,
    actions: Vec<TouchAction>,
    shapes: HashMap<i32, f64>,
}
impl SeatDevices {
    fn new(seat: wl_seat::WlSeat) -> Self {
        Self { seat, pointer: None, touch: None, keyboard: None, text_keyboard: None, mods: 0,
            virtual_pointers: HashMap::new(), contacts: HashMap::new(), actions: Vec::new(), shapes: HashMap::new() }
    }
}
impl Drop for SeatDevices {
    fn drop(&mut self) {
        if let Some(touch) = self.touch.take() { if touch.version() >= 3 { touch.release(); } }
        if let Some(keyboard) = self.keyboard.take() { keyboard.destroy(); }
        if let Some(keyboard) = self.text_keyboard.take() { if keyboard.version() >= 3 { keyboard.release(); } }
        for (_, pointer) in self.virtual_pointers.drain() { pointer.destroy(); }
    }
}
struct WlState {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    conn: Connection,
    compositor: CompositorState,
    layer_shell: LayerShell,
    vk_manager: Option<ZwpVirtualKeyboardManagerV1>,
    vp_manager: Option<ZwlrVirtualPointerManagerV1>,
    fs_manager: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
    vk: Rc<RefCell<Option<ZwpVirtualKeyboardV1>>>,
    uinput: Rc<RefCell<Option<crate::uinput::KeyInjector>>>,
    surfaces: Vec<OutputSurface>,
    seats: Vec<SeatDevices>,
    next_touch: i32,
    desktop: Desktop,
}
impl WlState {
    fn surface_id(&self, surface: &wl_surface::WlSurface) -> Option<String> {
        self.surfaces.iter().find(|s| &s.surface == surface).map(|s| s.id.clone())
    }
    fn apply_scale(&mut self, id: &str, scale: f64) {
        let Some(output) = self.desktop.outputs.get(id) else { return };
        let platform = output.rt.backend.as_any().downcast_ref::<WlPlat>().unwrap();
        platform.set_scale(scale);
        let (w,h) = platform.logical.get(); let actual = platform.scale.get();
        if w > 0 && h > 0 { self.desktop.configure(id,w,h,actual); }
    }
    fn create_output(&mut self, output: wl_output::WlOutput, qh: &QueueHandle<Self>) {
        if self.surfaces.len() >= 16 || self.surfaces.iter().any(|s| s.output == output) { return; }
        let info = self.output_state.info(&output);
        let mut id = info.as_ref().and_then(|i| i.name.clone()).unwrap_or_else(|| format!("wl-{}",output.id().protocol_id()));
        if self.desktop.outputs.contains_key(&id) { id = format!("{id}-{}",output.id().protocol_id()); }
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(qh,surface.clone(),Layer::Overlay,Some("sidera"),Some(&output));
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0,0);
        let viewport = self.viewporter.as_ref().map(|v|v.get_viewport(&surface,qh,()));
        let fractional = self.fs_manager.as_ref().filter(|_|viewport.is_some()).map(|m|m.get_fractional_scale(&surface,qh,()));
        let scale = render_scale(info.map_or(1.0, |i|i.scale_factor as f64),viewport.is_some(),surface.version());
        let platform = WlPlat { compositor: self.compositor.clone(), layer: layer.clone(),conn:self.conn.clone(),qh:qh.clone(),
            frames:RefCell::new(ShmFrames::new(&self.shm)),logical:Cell::new((0,0)),dims:Cell::new((0,0)),scale:Cell::new(scale),
            viewport:RefCell::new(viewport),uinput:self.uinput.clone(),back:RefCell::new(Pixmap::new(1,1).unwrap()),
            vk:self.vk.clone(),pointer:RefCell::new(None),pending_click:Cell::new(None),dirty:Cell::new(None),input_dirty:Cell::new(false) };
        self.desktop.add(id.clone(),Box::new(platform));
        self.surfaces.push(OutputSurface {id,output,surface,fractional});
        // No input or pixels before the compositor acknowledges the layer size.
        layer.commit();
    }
    fn remove_output(&mut self, id: &str) {
        self.desktop.remove(id);
        self.surfaces.retain(|surface|surface.id != id);
        for seat in &mut self.seats {
            seat.contacts.retain(|_,c| c.output != id);
            seat.actions.retain(|a| a.contact.output != id);
            seat.shapes.retain(|key,_| seat.contacts.values().any(|c| c.id == *key));
            if let Some(pointer) = seat.virtual_pointers.remove(id) { pointer.destroy(); }
        }
    }
    fn select_seat(&mut self, index: usize, qh: &QueueHandle<Self>) {
        let seat = &mut self.seats[index];
        *self.vk.borrow_mut() = seat.keyboard.clone();
        if let Some(manager) = self.vp_manager.as_ref() {
            for surface in &self.surfaces {
                let pointer = seat.virtual_pointers.entry(surface.id.clone()).or_insert_with(||
                    manager.create_virtual_pointer_with_output(Some(&seat.seat),Some(&surface.output),qh,()));
                if let Some(output) = self.desktop.outputs.get(&surface.id) {
                    let platform = output.rt.backend.as_any().downcast_ref::<WlPlat>().unwrap();
                    *platform.pointer.borrow_mut() = Some(pointer.clone());
                }
            }
        }
    }
    fn cancel_seat(&mut self, index: usize) {
        let seat = &mut self.seats[index];
        let outputs: std::collections::HashSet<_> = seat.contacts.values().map(|c| c.output.clone())
            .chain(seat.actions.iter().map(|a| a.contact.output.clone())).collect();
        seat.contacts.clear(); seat.actions.clear(); seat.shapes.clear();
        for id in outputs {
            self.desktop.cancel(&id);
            // A canvas gesture can combine contacts from several seats. Reset
            // its remaining routes too, so a cancelled gesture cannot reappear.
            for other in &mut self.seats {
                other.contacts.retain(|_,c| c.output != id);
                other.actions.retain(|a| a.contact.output != id);
                other.shapes.retain(|key,_|other.contacts.values().any(|c|c.id == *key));
            }
        }
    }
    fn touch_frame(&mut self, index: usize) {
        let actions = std::mem::take(&mut self.seats[index].actions);
        // Look ahead within the protocol frame before drawing the first contact.
        // Both contacts of a palm can arrive in the same frame, in either order.
        let mut contacts: HashMap<String, std::collections::HashSet<i32>> = self.desktop.outputs.iter().map(|(id,o)|
            (id.clone(),o.ip.touch_over_ui.iter().filter_map(|(id,ui)|if !ui {Some(*id)} else {None}).collect())).collect();
        let mut counts: HashMap<String,usize> = contacts.iter().map(|(id,c)|(id.clone(),c.len())).collect();
        for action in &actions {
            let c = &action.contact;
            let Some(output) = self.desktop.outputs.get(&c.output) else { continue };
            let active = contacts.entry(c.output.clone()).or_default();
            match action.kind {
                TouchKind::Begin if !crate::over_ui(&output.app,c.x,c.y) => { active.insert(c.id); }
                TouchKind::End => { active.remove(&c.id); }
                _ => (),
            }
            let count = counts.entry(c.output.clone()).or_default(); *count = (*count).max(active.len());
        }
        for (id,count) in counts { if let Some(output) = self.desktop.outputs.get_mut(&id) { output.app.touch_frame_contacts = count; } }
        for action in actions {
            let c = action.contact;
            let diameter = self.seats[index].shapes.get(&c.id).copied().unwrap_or(0.0);
            self.desktop.touch(&c.output,c.id,c.x,c.y,action.kind,diameter);
        }
        for output in self.desktop.outputs.values_mut() { output.app.touch_frame_contacts = 0; }
        let seat = &mut self.seats[index];
        seat.shapes.retain(|id,_|seat.contacts.values().any(|c|c.id == *id));
    }
}

fn send_keymap(kb: &ZwpVirtualKeyboardV1) -> bool {
    use std::io::Write;
    use std::os::fd::{AsFd, FromRawFd};
    let name = std::ffi::CString::new("sidera-keymap").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(),libc::MFD_CLOEXEC) };
    if fd < 0 { return false; }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    if file.write_all(KEYMAP_XKB.as_bytes()).and_then(|_| file.write_all(&[0])).is_err() { return false; }
    kb.keymap(KeymapFormat::XkbV1,file.as_fd(),(KEYMAP_XKB.len()+1) as u32);
    true
}
impl CompositorHandler for WlState {
    fn scale_factor_changed(&mut self,_:&Connection,_:&QueueHandle<Self>,surface:&wl_surface::WlSurface,factor:i32) {
        if let Some(id) = self.surfaces.iter().find(|s| &s.surface == surface && s.fractional.is_none()).map(|s|s.id.clone()) {
            self.apply_scale(&id,factor.max(1) as f64);
        }
    }
    fn transform_changed(&mut self,_:&Connection,_:&QueueHandle<Self>,_:&wl_surface::WlSurface,_:wl_output::Transform) {}
    fn frame(&mut self,_:&Connection,_:&QueueHandle<Self>,_:&wl_surface::WlSurface,_:u32) {}
    fn surface_enter(&mut self,_:&Connection,_:&QueueHandle<Self>,_:&wl_surface::WlSurface,_:&wl_output::WlOutput) {}
    fn surface_leave(&mut self,_:&Connection,_:&QueueHandle<Self>,_:&wl_surface::WlSurface,_:&wl_output::WlOutput) {}
}
impl OutputHandler for WlState {
    fn output_state(&mut self) -> &mut OutputState { &mut self.output_state }
    fn new_output(&mut self,_:&Connection,qh:&QueueHandle<Self>,output:wl_output::WlOutput) { self.create_output(output,qh); }
    fn update_output(&mut self,_:&Connection,qh:&QueueHandle<Self>,output:wl_output::WlOutput) {
        self.create_output(output,qh);
    }
    fn output_destroyed(&mut self,_:&Connection,_:&QueueHandle<Self>,output:wl_output::WlOutput) {
        if let Some(id) = self.surfaces.iter().find(|s|s.output == output).map(|s|s.id.clone()) { self.remove_output(&id); }
    }
}
impl LayerShellHandler for WlState {
    fn closed(&mut self,_:&Connection,_:&QueueHandle<Self>,layer:&LayerSurface) {
        if let Some(id) = self.surface_id(layer.wl_surface()) { self.remove_output(&id); }
    }
    fn configure(&mut self,_:&Connection,_:&QueueHandle<Self>,layer:&LayerSurface,configure:LayerSurfaceConfigure,_:u32) {
        let Some(id) = self.surface_id(layer.wl_surface()) else { return };
        let surface = self.surfaces.iter().find(|s|s.id == id).unwrap();
        let fallback = self.output_state.info(&surface.output).and_then(|i|i.logical_size).unwrap_or((1920,1080));
        let (w,h) = configure.new_size;
        let (w,h) = (if w == 0 { fallback.0 } else {w as i32}, if h == 0 {fallback.1} else {h as i32});
        let platform = self.desktop.outputs[&id].rt.backend.as_any().downcast_ref::<WlPlat>().unwrap();
        let scale = platform.scale.get();
        if !crate::app::valid_geometry(w,h,scale) { log::error!("忽略过大 Wayland 输出：{w}x{h}@{scale}"); return; }
        platform.on_configure(w,h);
        self.desktop.configure(&id,w,h,scale);
    }
}
fn evdev_char(key: u32, upper: bool) -> Option<char> {
    let (lower, shifted) = match key {
        2 => ('1', '!'), 3 => ('2', '@'), 4 => ('3', '#'), 5 => ('4', '$'),
        6 => ('5', '%'), 7 => ('6', '^'), 8 => ('7', '&'), 9 => ('8', '*'),
        10 => ('9', '('), 11 => ('0', ')'), 12 => ('-', '_'), 13 => ('=', '+'),
        16 => ('q', 'Q'), 17 => ('w', 'W'), 18 => ('e', 'E'), 19 => ('r', 'R'),
        20 => ('t', 'T'), 21 => ('y', 'Y'), 22 => ('u', 'U'), 23 => ('i', 'I'),
        24 => ('o', 'O'), 25 => ('p', 'P'), 26 => ('[', '{'), 27 => (']', '}'),
        30 => ('a', 'A'), 31 => ('s', 'S'), 32 => ('d', 'D'), 33 => ('f', 'F'),
        34 => ('g', 'G'), 35 => ('h', 'H'), 36 => ('j', 'J'), 37 => ('k', 'K'),
        38 => ('l', 'L'), 39 => (';', ':'), 40 => ('\'', '"'), 41 => ('`', '~'),
        43 => ('\\', '|'), 44 => ('z', 'Z'), 45 => ('x', 'X'), 46 => ('c', 'C'),
        47 => ('v', 'V'), 48 => ('b', 'B'), 49 => ('n', 'N'), 50 => ('m', 'M'),
        51 => (',', '<'), 52 => ('.', '>'), 53 => ('/', '?'), 57 => (' ', ' '),
        _ => return None,
    };
    Some(if upper { shifted } else { lower })
}

impl wayland_client::Dispatch<wl_keyboard::WlKeyboard, ()> for WlState {
    fn event(
        state: &mut Self,
        keyboard: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
        let Some(index) = state.seats.iter().position(|seat| {
            seat.text_keyboard.as_ref().is_some_and(|item| item == keyboard)
        }) else { return };
        match event {
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.seats[index].mods = mods_depressed;
            }
            wl_keyboard::Event::Key { key, state: wayland_client::WEnum::Value(KeyState::Pressed), .. } => {
                let mods = state.seats[index].mods;
                let mut changed = false;
                for output in state.desktop.outputs.values_mut() {
                    if !output.app.process_block_open || !output.app.process_input_active {
                        continue;
                    }
                    let ctrl = mods & 4 != 0;
                    let shift = mods & 1 != 0;
                    let caps = mods & 2 != 0;
                    if key == 1 {
                        output.app.process_input_active = false;
                        output.rt.backend.set_text_input(false);
                        changed = true;
                    } else if key == 28 {
                        let value = output.app.process_input.trim().to_string();
                        if !value.is_empty() && value.chars().count() <= 256
                            && !output.app.process_rules.iter().any(|rule| rule == &value)
                            && output.app.process_rules.len() < 64
                        {
                            output.app.process_rules.push(value);
                            let _ = crate::process_block::save_rules(&output.app.process_rules);
                        }
                        output.app.process_input.clear();
                        changed = true;
                    } else if key == 14 {
                        output.app.process_input.pop();
                        changed = true;
                    } else if ctrl && key == 47 {
                        if let Some(text) = crate::clipboard::get_text() {
                            output.app.process_input = text.chars().take(256).collect();
                            changed = true;
                        }
                    } else if !ctrl {
                        if let Some(ch) = evdev_char(key, shift ^ caps) {
                            if output.app.process_input.chars().count() < 256 {
                                output.app.process_input.push(ch);
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        output.rt.redraw_full(&output.app);
                    }
                    break;
                }
            }
            _ => {}
        }
    }
}

impl SeatHandler for WlState {
    fn seat_state(&mut self) -> &mut SeatState { &mut self.seat_state }
    fn new_seat(&mut self,_:&Connection,_:&QueueHandle<Self>,seat:wl_seat::WlSeat) {
        if !self.seats.iter().any(|s|s.seat == seat) { self.seats.push(SeatDevices::new(seat)); }
    }
    fn new_capability(&mut self,_:&Connection,qh:&QueueHandle<Self>,seat:wl_seat::WlSeat,capability:Capability) {
        if !self.seats.iter().any(|s|s.seat == seat) { self.seats.push(SeatDevices::new(seat.clone())); }
        let index = self.seats.iter().position(|s|s.seat == seat).unwrap();
        let devices = &mut self.seats[index];
        if capability == Capability::Pointer && devices.pointer.is_none() {
            devices.pointer = self.seat_state.get_pointer_with_theme(qh,&seat,self.shm.wl_shm(),
                self.compositor.create_surface(qh),ThemeSpec::System).ok();
        }
        if capability == Capability::Touch && devices.touch.is_none() {
            // Own dispatch keeps shape/down/up together through wl_touch.frame.
            devices.touch = Some(seat.get_touch(qh,()));
        }
        if capability == Capability::Keyboard && devices.text_keyboard.is_none() {
            devices.text_keyboard = Some(seat.get_keyboard(qh, ()));
        }
        if devices.keyboard.is_none() {
            if let Some(manager) = self.vk_manager.as_ref() {
                let keyboard = manager.create_virtual_keyboard(&seat,qh,());
                if send_keymap(&keyboard) { devices.keyboard = Some(keyboard); } else { keyboard.destroy(); }
            }
        }
        self.select_seat(index,qh);
    }
    fn remove_capability(&mut self,_:&Connection,_:&QueueHandle<Self>,seat:wl_seat::WlSeat,capability:Capability) {
        let Some(index) = self.seats.iter().position(|s|s.seat == seat) else { return };
        if capability == Capability::Touch {
            self.cancel_seat(index);
            if let Some(touch) = self.seats[index].touch.take() { if touch.version() >= 3 { touch.release(); } }
        }
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.seats[index].text_keyboard.take() {
                if keyboard.version() >= 3 { keyboard.release(); }
            }
        }
        if capability == Capability::Pointer {
            self.seats[index].pointer.take();
            for id in self.desktop.outputs.keys().cloned().collect::<Vec<_>>() { self.desktop.cancel(&id); }
        }
    }
    fn remove_seat(&mut self,_:&Connection,qh:&QueueHandle<Self>,seat:wl_seat::WlSeat) {
        let Some(index) = self.seats.iter().position(|s|s.seat == seat) else { return };
        self.cancel_seat(index);
        self.seats.remove(index);
        *self.vk.borrow_mut() = None;
        for output in self.desktop.outputs.values() {
            *output.rt.backend.as_any().downcast_ref::<WlPlat>().unwrap().pointer.borrow_mut() = None;
        }
        if !self.seats.is_empty() { self.select_seat(0,qh); }
    }
}
impl PointerHandler for WlState {
    fn pointer_frame(&mut self,conn:&Connection,qh:&QueueHandle<Self>,pointer:&wl_pointer::WlPointer,events:&[PointerEvent]) {
        let Some(index) = self.seats.iter().position(|s|s.pointer.as_ref().is_some_and(|p|p.pointer() == pointer)) else { return };
        self.select_seat(index,qh);
        for event in events {
            let Some(id) = self.surface_id(&event.surface) else { continue };
            let (x,y) = (event.position.0 as i32,event.position.1 as i32);
            match event.kind {
                PointerEventKind::Enter {..} => {
                    if let Some(pointer) = self.seats[index].pointer.as_ref() { let _ = pointer.set_cursor(conn,CursorIcon::Default); }
                    self.desktop.motion(&id,x,y);
                }
                PointerEventKind::Leave {..} => self.desktop.leave(&id),
                PointerEventKind::Motion {..} => self.desktop.motion(&id,x,y),
                PointerEventKind::Axis {vertical,..} => {
                    let pages = if vertical.discrete != 0 { vertical.discrete as f64 }
                        else { vertical.absolute / crate::page_picker::ROW };
                    self.desktop.scroll(&id,x,y,pages);
                },
                PointerEventKind::Press {button,..} => if matches!(button,0x110|0x111) {
                    self.desktop.press(&id,x,y,if button == 0x110 {1} else {3});
                },
                PointerEventKind::Release {button,..} => if matches!(button,0x110|0x111) { self.desktop.release(&id,x,y,if button == 0x110 {1} else {3}); },
            }
        }
    }
}
impl wayland_client::Dispatch<wl_touch::WlTouch,()> for WlState {
    fn event(state:&mut Self,touch:&wl_touch::WlTouch,event:wl_touch::Event,_:&(),_:&Connection,qh:&QueueHandle<Self>) {
        let Some(index) = state.seats.iter().position(|s|s.touch.as_ref() == Some(touch)) else { return };
        state.select_seat(index,qh);
        if state.seats[index].actions.len() >= 512 { state.cancel_seat(index); return; }
        match event {
            wl_touch::Event::Down {surface,id,x,y,..} => {
                let Some(output) = state.surface_id(&surface) else { return };
                if state.seats[index].contacts.len() >= 32 { state.cancel_seat(index); return; }
                state.next_touch = state.next_touch.wrapping_add(1);
                let contact = Contact {id:state.next_touch,output,x:x as i32,y:y as i32};
                state.seats[index].contacts.insert(id,contact.clone());
                state.seats[index].actions.push(TouchAction {contact,kind:TouchKind::Begin});
            }
            wl_touch::Event::Motion {id,x,y,..} => {
                let seat = &mut state.seats[index];
                if let Some(c) = seat.contacts.get_mut(&id) {
                    c.x=x as i32; c.y=y as i32;
                    seat.actions.push(TouchAction {contact:c.clone(),kind:TouchKind::Update});
                }
            }
            wl_touch::Event::Up {id,..} => {
                let seat = &mut state.seats[index];
                if let Some(contact) = seat.contacts.remove(&id) { seat.actions.push(TouchAction {contact,kind:TouchKind::End}); }
            }
            wl_touch::Event::Shape {id,major,minor} => {
                let seat = &mut state.seats[index];
                if let Some(c) = seat.contacts.get(&id) { seat.shapes.insert(c.id,major.max(minor).clamp(0.0,1024.0)); }
            }
            wl_touch::Event::Frame => state.touch_frame(index),
            wl_touch::Event::Cancel => state.cancel_seat(index),
            _ => (),
        }
    }
}
impl ShmHandler for WlState { fn shm_state(&mut self) -> &mut Shm { &mut self.shm } }
delegate_compositor!(WlState);
delegate_output!(WlState);
delegate_shm!(WlState);
delegate_seat!(WlState);
delegate_pointer!(WlState);
delegate_layer!(WlState);
delegate_registry!(WlState);
impl ProvidesRegistryState for WlState {
    fn registry(&mut self) -> &mut RegistryState { &mut self.registry_state }
    smithay_client_toolkit::registry_handlers![OutputState, SeatState];
}

pub fn run(prepared:PreparedWayland,fonts:Fonts,icon:Option<Pixmap>,icon_big:Option<Pixmap>,single:Option<UnixListener>)
    -> Result<(),Box<dyn std::error::Error>>
{
    let PreparedWayland {conn,globals,event_queue,compositor,layer_shell,shm} = prepared;
    let qh = event_queue.handle();
    let registry_state = RegistryState::new(&globals);
    let vk_manager = registry_state.bind_one::<ZwpVirtualKeyboardManagerV1,WlState,()>(&qh,1..=1,()).ok();
    // Output mapping was added in v2. Never guess global coordinates on v1.
    let vp_manager = registry_state.bind_one::<ZwlrVirtualPointerManagerV1,WlState,()>(&qh,2..=2,()).ok();
    let fs_manager = registry_state.bind_one::<WpFractionalScaleManagerV1,WlState,()>(&qh,1..=1,()).ok();
    let viewporter = registry_state.bind_one::<WpViewporter,WlState,()>(&qh,1..=1,()).ok();
    let mut state = WlState {registry_state,seat_state:SeatState::new(&globals,&qh),output_state:OutputState::new(&globals,&qh),
        shm,conn:conn.clone(),compositor,layer_shell,vk_manager,vp_manager,fs_manager,viewporter,
        vk:Rc::new(RefCell::new(None)),uinput:Rc::new(RefCell::new(crate::uinput::KeyInjector::new())),
        surfaces:Vec::new(),seats:Vec::new(),next_touch:0,desktop:Desktop::new(fonts,icon,icon_big,single)};
    let mut event_loop: EventLoop<WlState> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(),event_queue).insert(handle.clone())?;
    handle.insert_source(Timer::from_duration(Duration::from_millis(16)),|_,_,state:&mut WlState| {
        state.desktop.tick();
        let _ = state.conn.flush();
        TimeoutAction::ToDuration(Duration::from_millis(16))
    })?;
    event_loop.run(None,&mut state, |_|{})?;
    Ok(())
}
