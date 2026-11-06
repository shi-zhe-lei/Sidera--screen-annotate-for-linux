// ============================================================
// Sidera - X11 后端封装（x11rb，纯 Rust，不需要 libX11/libXtst）
// 覆盖：ARGB 覆盖层窗口、XShape 输入穿透、XTest 假键/虚拟右键、
//       全局热键、WPS 全屏检测、截图、合成器检测
// ============================================================
use x11rb::connection::Connection;
use x11rb::protocol::shape::{self, SK, SO};
use x11rb::protocol::xproto::{
    self, Atom, AtomEnum, Colormap, ColormapAlloc, ConfigureWindowAux, CreateGCAux,
    CreateWindowAux, Drawable, EventMask, Gcontext, GrabMode, ImageFormat, Keycode, ModMask, Rectangle,
    VisualClass, Window, WindowClass,
};
use x11rb::protocol::xinput::{self, DeviceClassData, Fp3232};
use x11rb::protocol::xtest;
use x11rb::rust_connection::RustConnection;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

pub type XResult<T> = Result<T, Box<dyn std::error::Error>>;

pub const XK_ESCAPE: u32 = 0xff1b;
pub const XK_UP: u32 = 0xff52;
pub const XK_DOWN: u32 = 0xff54;
pub const XK_D: u32 = 0x0064;
pub const XK_F5: u32 = 0xffc2;
pub const XK_SHIFT_L: u32 = 0xffe1;
pub const XK_BACKSPACE: u32 = 0xff08;
pub const XK_RETURN: u32 = 0xff0d;

pub struct X11 {
    pub conn: RustConnection,
    #[allow(dead_code)]
    pub screen_num: usize,
    pub is_xwayland: bool,
    pub root: Window,
    pub width: i32,
    pub height: i32,
    pub argb_visual: u32,
    pub argb_depth: u8,
    pub cmap: Colormap,
    pub net_wm_state: Atom,
    pub net_wm_fullscreen: Atom,
    pub net_active_window: Atom,
    pub net_wm_cm: Atom,
    pub key_escape: Keycode,
    pub key_up: Keycode,
    pub key_down: Keycode,
    pub key_d: Keycode,
    pub key_f5: Keycode,
    pub key_shift: Keycode,
    pub atom_touch_major: Atom,
    pub atom_touch_minor: Atom,
    pub atom_pos_x: Atom,
}

impl X11 {
    pub fn connect() -> XResult<Self> {
        let (conn, screen_num) = x11rb::connect(None)?;
        let is_xwayland = xproto::query_extension(&conn, b"XWAYLAND")?.reply()?.present;
        crate::startup::validate_x11_server(
            is_xwayland,
            conn.setup().release_number,
            &conn.setup().vendor,
        )?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;

        // 根窗口的完整几何（可能跨多显示器）
        let geo = xproto::get_geometry(&conn, root)?.reply()?;
        let width = geo.width as i32;
        let height = geo.height as i32;

        // 找 32 位 TrueColor 视觉（ARGB）
        let mut argb_visual = screen.root_visual;
        let mut argb_depth = screen.root_depth;
        for d in &screen.allowed_depths {
            if d.depth == 32 {
                for v in &d.visuals {
                    if v.class == VisualClass::TRUE_COLOR {
                        argb_visual = v.visual_id;
                        argb_depth = 32;
                    }
                }
            }
        }

        let cmap = conn.generate_id()?;
        xproto::create_colormap(&conn, ColormapAlloc::NONE, cmap, root, argb_visual)?;

        let net_wm_state = xproto::intern_atom(&conn, false, b"_NET_WM_STATE")?.reply()?.atom;
        let net_wm_fullscreen =
            xproto::intern_atom(&conn, false, b"_NET_WM_STATE_FULLSCREEN")?.reply()?.atom;
        let net_active_window =
            xproto::intern_atom(&conn, false, b"_NET_ACTIVE_WINDOW")?.reply()?.atom;
        let net_wm_cm = xproto::intern_atom(&conn, false, b"_NET_WM_CM_S0")?.reply()?.atom;
        let atom_touch_major =
            xproto::intern_atom(&conn, false, b"Abs MT Touch Major")?.reply()?.atom;
        let atom_touch_minor =
            xproto::intern_atom(&conn, false, b"Abs MT Touch Minor")?.reply()?.atom;
        let atom_pos_x = xproto::intern_atom(&conn, false, b"Abs MT Position X")?.reply()?.atom;

        let mut x = X11 {
            conn,
            screen_num,
            is_xwayland,
            root,
            width,
            height,
            argb_visual,
            argb_depth,
            cmap,
            net_wm_state,
            net_wm_fullscreen,
            net_active_window,
            net_wm_cm,
            key_escape: 0,
            key_up: 0,
            key_down: 0,
            key_d: 0,
            key_f5: 0,
            key_shift: 0,
            atom_touch_major,
            atom_touch_minor,
            atom_pos_x,
        };
        x.key_escape = x.keysym_to_keycode(XK_ESCAPE);
        x.key_up = x.keysym_to_keycode(XK_UP);
        x.key_down = x.keysym_to_keycode(XK_DOWN);
        x.key_d = x.keysym_to_keycode(XK_D);
        x.key_f5 = x.keysym_to_keycode(XK_F5);
        x.key_shift = x.keysym_to_keycode(XK_SHIFT_L);
        x.conn.flush()?;
        Ok(x)
    }

    fn keysym_to_keycode(&self, ks: u32) -> Keycode {
        let setup = self.conn.setup();
        let min = setup.min_keycode;
        let max = setup.max_keycode;
        let count = max - min + 1;
        if let Ok(cookie) = xproto::get_keyboard_mapping(&self.conn, min, count) {
            if let Ok(reply) = cookie.reply() {
                let per = reply.keysyms_per_keycode as usize;
                if per == 0 {
                    return 0;
                }
                for (i, chunk) in reply.keysyms.chunks(per).enumerate() {
                    if chunk.iter().any(|k| *k == ks) {
                        return min + i as u8;
                    }
                }
            }
        }
        0
    }

    /// 创建覆盖层窗口，返回 (window, gc)
    pub fn create_overlay(&self) -> XResult<(Window, Gcontext)> {
        self.create_window(0, 0, self.width, self.height)
    }

    pub fn create_window(&self, x: i32, y: i32, w: i32, h: i32) -> XResult<(Window, Gcontext)> {
        self.create_window_with_input(x, y, w, h, true)
    }

    fn create_window_with_input(&self, x: i32, y: i32, w: i32, h: i32, input: bool) -> XResult<(Window, Gcontext)> {
        let win = self.conn.generate_id()?;
        let event_mask = EventMask::EXPOSURE
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::BUTTON1_MOTION
            | EventMask::BUTTON3_MOTION
            | EventMask::KEY_PRESS
            | EventMask::LEAVE_WINDOW
            | EventMask::STRUCTURE_NOTIFY;
        let aux = CreateWindowAux::new()
            .background_pixel(0u32)
            .border_pixel(0u32)
            .colormap(self.cmap)
            .override_redirect(1u32)
            .event_mask(event_mask);
        xproto::create_window(
            &self.conn,
            self.argb_depth,
            win,
            self.root,
            x as i16,
            y as i16,
            w as u16,
            h as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            self.argb_visual,
            &aux,
        )?;
        // The persistent visual surface must never acquire pointer focus, even
        // briefly during its first map, or Mutter can cache it as an input target.
        if !input {
            self.set_input_shape(win, &[], false)?;
        }
        let gc = self.conn.generate_id()?;
        xproto::create_gc(&self.conn, gc, win, &CreateGCAux::new().graphics_exposures(0u32))?;
        xproto::map_window(&self.conn, win)?;
        xproto::configure_window(
            &self.conn,
            win,
            &ConfigureWindowAux::new().stack_mode(xproto::StackMode::ABOVE),
        )?;
        self.conn.flush()?;
        Ok((win, gc))
    }

    fn label_overlay(&self, win: Window, role: &[u8]) -> XResult<()> {
        let atom = xproto::intern_atom(&self.conn, false, b"_SIDERA_OVERLAY_ROLE")?.reply()?.atom;
        xproto::change_property(
            &self.conn, xproto::PropMode::REPLACE, win, atom, AtomEnum::STRING,
            8, role.len() as u32, role,
        )?.check()?;
        Ok(())
    }

    /// 把整张 pixmap（预乘 RGBA）转成 X11 的 BGRA 并上传到 (x,y)
    pub fn put_pixmap(
        &self,
        win: Window,
        gc: Gcontext,
        pm: &tiny_skia::Pixmap,
        x: i32,
        y: i32,
    ) -> XResult<()> {
        let src = pm.data();
        let mut buf = Vec::with_capacity(src.len());
        for px in src.chunks_exact(4) {
            buf.push(px[2]);
            buf.push(px[1]);
            buf.push(px[0]);
            buf.push(px[3]);
        }
        xproto::put_image(
            &self.conn,
            ImageFormat::Z_PIXMAP,
            win as Drawable,
            gc,
            pm.width() as u16,
            pm.height() as u16,
            x as i16,
            y as i16,
            0,
            self.argb_depth,
            &buf,
        )?;
        self.conn.flush()?;
        Ok(())
    }

    pub fn destroy_window(&self, win: Window) {
        let _ = xproto::destroy_window(&self.conn, win);
        let _ = self.conn.flush();
    }

    /// XI 2.2 触摸需要先声明客户端版本；原始事件只能订阅在根窗口。
    /// 普通触摸事件已带尺寸 valuator，覆盖层不需要全局 raw 触摸监听。
    pub fn select_touch_events(&self, win: Window) -> XResult<()> {
        let version = xinput::xi_query_version(&self.conn, 2, 2)?.reply()?;
        if (version.major_version, version.minor_version) < (2, 2) {
            return Err(format!(
                "触摸需要 XInput 2.2，当前服务器为 {}.{}",
                version.major_version, version.minor_version
            )
            .into());
        }
        let mask = xinput::XIEventMask::TOUCH_BEGIN
            | xinput::XIEventMask::TOUCH_UPDATE
            | xinput::XIEventMask::TOUCH_END
            | xinput::XIEventMask::TOUCH_OWNERSHIP;
        xinput::xi_select_events(
            &self.conn,
            win,
            &[xinput::EventMask {
                deviceid: xinput::Device::ALL.into(),
                mask: vec![mask],
            }],
        )?
        .check()?;
        Ok(())
    }

    /// 设置输入区域：白=可点击。rects 为空且 full=true 表示恢复整窗可输入。
    pub fn set_input_shape(&self, win: Window, rects: &[Rectangle], full: bool) -> XResult<()> {
        if full {
            shape::mask(&self.conn, SO::SET, SK::INPUT, win, 0, 0, x11rb::NONE)?.check()?;
        } else {
            shape::rectangles(
                &self.conn, SO::SET, SK::INPUT, xproto::ClipOrdering::UNSORTED,
                win, 0, 0, rects,
            )?.check()?;
        }
        Ok(())
    }

    fn uses_mutter_xwayland(&self) -> bool {
        if !self.is_xwayland {
            return false;
        }
        let check = || -> XResult<bool> {
            let supporting = xproto::intern_atom(&self.conn, true, b"_NET_SUPPORTING_WM_CHECK")?.reply()?.atom;
            let name = xproto::intern_atom(&self.conn, true, b"_NET_WM_NAME")?.reply()?.atom;
            if supporting == 0 || name == 0 { return Ok(false); }
            let prop = xproto::get_property(&self.conn, false, self.root, supporting, AtomEnum::WINDOW, 0, 1)?.reply()?;
            let Some(wm) = prop.value32().and_then(|mut values| values.next()) else { return Ok(false); };
            let name = xproto::get_property(&self.conn, false, wm, name, AtomEnum::ANY, 0, 64)?.reply()?;
            Ok(matches!(name.value.as_slice(), b"GNOME Shell" | b"Mutter"))
        };
        check().unwrap_or(false)
    }

    /// 协商 XInput 2.2，返回 (major, minor)。
    /// 必须在选择触摸事件前调用：客户端不声明 2.2，服务器按 2.0 处理，不会投递触摸事件。
    pub fn negotiate_xi(&self) -> XResult<(u16, u16)> {
        let v = xinput::xi_query_version(&self.conn, 2, 2)?.reply()?;
        self.conn.flush()?;
        Ok((v.major_version, v.minor_version))
    }

    /// 枚举支持触摸的设备（建议先 negotiate_xi）
    pub fn touch_device_ids(&self) -> Vec<u16> {
        let mut ids = Vec::new();
        if let Some(reply) = xinput::xi_query_device(&self.conn, 0u16)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            for info in &reply.infos {
                if info
                    .classes
                    .iter()
                    .any(|c| matches!(c.data, DeviceClassData::Touch(_)))
                {
                    log::info!(
                        "[TOUCH] 设备 {} '{}' 支持触摸",
                        info.deviceid,
                        String::from_utf8_lossy(&info.name)
                    );
                    ids.push(info.deviceid);
                }
            }
        }
        ids
    }

    /// 接受或拒绝触摸所有权。拒绝时让触摸继续交给下层窗口（例如 WPS）。
    pub fn allow_touch(&self, deviceid: u16, touchid: u32, window: Window, accept: bool) {
        let mode = if accept { xinput::EventMode::ACCEPT_TOUCH } else { xinput::EventMode::REJECT_TOUCH };
        let _ = xinput::xi_allow_events(&self.conn, 0u32, deviceid, mode, touchid, window);
        let _ = self.conn.flush();
    }

    pub fn keycode_to_char(&self, keycode: Keycode, shift: bool) -> Option<char> {
        if keycode == 0 {
            return None;
        }
        let reply = xproto::get_keyboard_mapping(&self.conn, keycode, 1)
            .ok()?
            .reply()
            .ok()?;
        let per = reply.keysyms_per_keycode as usize;
        if per == 0 {
            return None;
        }
        let index = if shift { 1.min(per - 1) } else { 0 };
        let keysym = *reply.keysyms.get(index)?;
        let ch = char::from_u32(keysym)?;
        if ch.is_control() {
            None
        } else {
            Some(ch)
        }
    }

    pub fn keycode_is(&self, keycode: Keycode, keysym: u32) -> bool {
        if keycode == 0 {
            return false;
        }
        let Ok(cookie) = xproto::get_keyboard_mapping(&self.conn, keycode, 1) else {
            return false;
        };
        let Ok(reply) = cookie.reply() else {
            return false;
        };
        reply
            .keysyms
            .iter()
            .take(reply.keysyms_per_keycode as usize)
            .any(|value| *value == keysym)
    }

    pub fn fake_key(&self, keycode: Keycode) {
        if keycode == 0 {
            return;
        }
        let _ = xtest::fake_input(
            &self.conn,
            xproto::KEY_PRESS_EVENT,
            keycode,
            0,
            self.root,
            0,
            0,
            0,
        );
        let _ = xtest::fake_input(
            &self.conn,
            xproto::KEY_RELEASE_EVENT,
            keycode,
            0,
            self.root,
            0,
            0,
            0,
        );
        let _ = self.conn.flush();
    }

    /// 演示软件“从当前页开始”的快捷键：Shift+F5。
    pub fn fake_start_show(&self) {
        for (kind, code) in [
            (xproto::KEY_PRESS_EVENT, self.key_shift),
            (xproto::KEY_PRESS_EVENT, self.key_f5),
            (xproto::KEY_RELEASE_EVENT, self.key_f5),
            (xproto::KEY_RELEASE_EVENT, self.key_shift),
        ] {
            if code != 0 {
                let _ = xtest::fake_input(&self.conn, kind, code, 0, self.root, 0, 0, 0);
            }
        }
        let _ = self.conn.flush();
    }

    pub fn virtual_right_click(&self, x: i32, y: i32) {
        let _ = xtest::fake_input(
            &self.conn,
            xproto::MOTION_NOTIFY_EVENT,
            0,
            0,
            self.root,
            x as i16,
            y as i16,
            0,
        );
        let _ = xtest::fake_input(
            &self.conn,
            xproto::BUTTON_PRESS_EVENT,
            3,
            0,
            self.root,
            x as i16,
            y as i16,
            0,
        );
        let _ = xtest::fake_input(
            &self.conn,
            xproto::BUTTON_RELEASE_EVENT,
            3,
            0,
            self.root,
            x as i16,
            y as i16,
            0,
        );
        let _ = self.conn.flush();
    }

    /// 抓取 Ctrl+Shift+D 全局热键（含 NumLock/CapsLock 组合）
    pub fn grab_hotkey(&self) -> bool {
        if self.key_d == 0 {
            return false;
        }
        let mods = ModMask::CONTROL | ModMask::SHIFT;
        let combos = [
            mods,
            mods | ModMask::from(1u16 << 1),  // Mod2 (NumLock)
            mods | ModMask::from(1u16),       // Lock (CapsLock)
            mods | ModMask::from(1u16 << 1) | ModMask::from(1u16),
        ];
        for m in combos {
            let _ = xproto::grab_key(
                &self.conn,
                false,
                self.root,
                m,
                self.key_d,
                xproto::GrabMode::ASYNC,
                xproto::GrabMode::ASYNC,
            );
        }
        let _ = self.conn.flush();
        true
    }

    /// 检测 X11 设备缩放（逻辑像素 → 物理像素的比例）。
    /// 优先 SIDERA_SCALE 覆盖，其次根窗口 Xft.dpi，再退回 QT/GDK 环境变量。
    pub fn detect_scale(&self) -> f64 {
        if let Ok(v) = std::env::var("SIDERA_SCALE") {
            if let Ok(f) = v.parse::<f64>() {
                if f > 0.1 {
                    return f.clamp(1.0, 4.0);
                }
            }
        }
        if let Ok(dpi) = self.xft_dpi() {
            if dpi > 1.0 {
                return (dpi / 96.0).clamp(1.0, 4.0);
            }
        }
        for key in ["QT_SCALE_FACTOR", "GDK_SCALE"] {
            if let Ok(v) = std::env::var(key) {
                if let Ok(f) = v.parse::<f64>() {
                    if f > 0.1 {
                        return f.clamp(1.0, 4.0);
                    }
                }
            }
        }
        1.0
    }

    /// 读取根窗口 RESOURCE_MANAGER 里的 Xft.dpi
    fn xft_dpi(&self) -> Result<f64, Box<dyn std::error::Error>> {
        let atom = xproto::intern_atom(&self.conn, true, b"RESOURCE_MANAGER")?
            .reply()?
            .atom;
        let prop = xproto::get_property(
            &self.conn,
            false,
            self.root,
            atom,
            AtomEnum::STRING,
            0,
            4096,
        )?
        .reply()?;
        let text = String::from_utf8_lossy(&prop.value);
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("Xft.dpi:") {
                if let Ok(f) = rest.trim().parse::<f64>() {
                    return Ok(f);
                }
            }
        }
        Err("Xft.dpi not set".into())
    }

    pub fn has_compositor(&self) -> bool {
        if let Ok(cookie) = xproto::get_selection_owner(&self.conn, self.net_wm_cm) {
            if let Ok(reply) = cookie.reply() {
                return reply.owner != x11rb::NONE;
            }
        }
        false
    }

    /// WPS/OnlyOffice/LibreOffice 全屏放映检测（递归窗口树）
    pub fn is_presentation_fullscreen(&self, self_win: Window) -> bool {
        let mut stack: Vec<Window> = vec![self.root];
        let mut count = 0;
        while let Some(w) = stack.pop() {
            if count > 6000 {
                break;
            }
            count += 1;
            if w == self_win {
                continue;
            }
            if self.is_office_window(w) {
                let attr = xproto::get_window_attributes(&self.conn, w)
                    .ok()
                    .and_then(|c| c.reply().ok());
                if let Some(attr) = attr {
                    if attr.map_state == xproto::MapState::VIEWABLE {
                        let mut fullscreen = false;
                        let prop = xproto::get_property(
                            &self.conn,
                            false,
                            w,
                            self.net_wm_state,
                            AtomEnum::ATOM,
                            0,
                            1024,
                        )
                        .ok()
                        .and_then(|c| c.reply().ok());
                        if let Some(prop) = prop {
                            for chunk in prop.value.chunks_exact(4) {
                                let a = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                                if a == self.net_wm_fullscreen {
                                    fullscreen = true;
                                    break;
                                }
                            }
                        }
                        if !fullscreen {
                            let geo = xproto::get_geometry(&self.conn, w)
                                .ok()
                                .and_then(|c| c.reply().ok());
                            if let Some(geo) = geo {
                                if geo.width as i32 >= self.width - 8
                                    && geo.height as i32 >= self.height - 8
                                {
                                    fullscreen = true;
                                }
                            }
                        }
                        if fullscreen {
                            return true;
                        }
                    }
                }
            }
            let tree = xproto::query_tree(&self.conn, w)
                .ok()
                .and_then(|c| c.reply().ok());
            if let Some(tree) = tree {
                for ch in tree.children {
                    stack.push(ch);
                }
            }
        }
        false
    }

    /// 当前活动窗口是否属于 WPS 类办公软件或本覆盖层。
    /// 用于避免后台放映窗口仍把 Sidera 切到放映模式。
    pub fn is_office_foreground(&self, self_win: Window) -> Option<bool> {
        let active = xproto::get_property(&self.conn, false, self.root, self.net_active_window,
            AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?
            .value32()?.next()?;
        if active == x11rb::NONE {
            return None;
        }
        let mut window = active;
        for _ in 0..32 {
            if window == self_win || self.is_office_window(window) {
                return Some(true);
            }
            let parent = xproto::query_tree(&self.conn, window).ok()?.reply().ok()?.parent;
            if parent == x11rb::NONE || parent == self.root {
                break;
            }
            window = parent;
        }
        Some(false)
    }

    fn is_office_window(&self, w: Window) -> bool {
        let prop = xproto::get_property(
            &self.conn,
            false,
            w,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            0,
            1024,
        )
        .ok()
        .and_then(|c| c.reply().ok());
        let Some(prop) = prop else {
            return false;
        };
        let name = String::from_utf8_lossy(&prop.value).to_lowercase();
        const KEYS: &[&str] = &[
            "wps",
            "wpp",
            "et",
            "wpspdf",
            "onlyoffice",
            "desktopeditors",
            "soffice",
            "impress",
            "libreoffice",
        ];
        name.split('\0').any(|field| KEYS.iter().any(|key| {
            if key.len() <= 3 { field == *key } else { field.contains(key) }
        }))
    }

    /// 抓取整个根窗口，返回 RGBA 像素
    pub fn screenshot(&self) -> Option<(Vec<u8>, u32, u32)> {
        let geometry = xproto::get_geometry(&self.conn,self.root).ok()?.reply().ok()?;
        let (w,h) = (geometry.width,geometry.height);
        if !crate::app::valid_geometry(w as i32,h as i32,1.0) { return None; }
        let reply = xproto::get_image(
            &self.conn,
            ImageFormat::Z_PIXMAP,
            self.root,
            0,
            0,
            w,
            h,
            u32::MAX,
        )
        .ok()?
        .reply()
        .ok()?;
        let mut rgba = vec![0u8; w as usize * h as usize * 4];
        for i in 0..(w as usize * h as usize) {
            let b = reply.data.get(i * 4).copied().unwrap_or(0);
            let g = reply.data.get(i * 4 + 1).copied().unwrap_or(0);
            let r = reply.data.get(i * 4 + 2).copied().unwrap_or(0);
            rgba[i * 4] = r;
            rgba[i * 4 + 1] = g;
            rgba[i * 4 + 2] = b;
            rgba[i * 4 + 3] = 255;
        }
        Some((rgba, w as u32, h as u32))
    }

    /// XShape 扩展是否可用
    pub fn shape_available(&self) -> bool {
        shape::query_version(&self.conn)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some()
    }
}

// ============================================================
// X11 后端实现（Backend trait）
// ============================================================
pub struct X11Plat {
    pub x11: X11,
    pub win: Window,
    pub gc: Gcontext,
    render_win: Window,
    render_gc: Gcontext,
    scale: Cell<f64>,
    physical: Cell<(i32, i32)>,
    touch_axis: RefCell<HashMap<u8, TouchAxis>>,
    refresh_input_regions: bool,
    input_region: RefCell<Option<(bool, Vec<Rectangle>)>>,
    input_refresh_pending: Cell<bool>,
    key_injector: RefCell<Option<crate::uinput::KeyInjector>>,
}

#[derive(Clone, Copy, Default)]
struct TouchAxis {
    /// (valuator 编号, min, max)
    major: Option<(u16, f64, f64)>,
    minor: Option<(u16, f64, f64)>,
    posx_min: f64,
    posx_max: f64,
}

impl X11Plat {
    pub fn grab_text_keyboard(&self) -> bool {
        xproto::grab_keyboard(
            &self.x11.conn,
            false,
            self.win,
            0u32,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some()
    }

    pub fn ungrab_text_keyboard(&self) {
        let _ = xproto::ungrab_keyboard(&self.x11.conn, 0u32);
        let _ = self.x11.conn.flush();
    }

    pub fn keycode_to_char(&self, keycode: Keycode, shift: bool) -> Option<char> {
        self.x11.keycode_to_char(keycode, shift)
    }

    pub fn keycode_is(&self, keycode: Keycode, keysym: u32) -> bool {
        self.x11.keycode_is(keycode, keysym)
    }

    pub fn new(x11: X11, win: Window, gc: Gcontext) -> XResult<Self> {
        let refresh_input_regions = x11.uses_mutter_xwayland();
        let (render_win, render_gc) = if refresh_input_regions {
            if x11.argb_depth != 32 {
                return Err("GNOME XWayland 兼容模式需要 32 位透明窗口视觉".into());
            }
            let visual = x11.create_window_with_input(0, 0, x11.width, x11.height, false)?;
            x11.label_overlay(win, b"input")?;
            x11.label_overlay(visual.0, b"visual")?;
            visual
        } else {
            (win, gc)
        };
        xproto::change_window_attributes(&x11.conn,x11.root,
            &xproto::ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE))?.check()?;
        let physical = Cell::new((x11.width,x11.height));
        let key_injector = RefCell::new(if x11.is_xwayland { crate::uinput::KeyInjector::new() } else { None });
        let plat = X11Plat {
            x11,
            win,
            gc,
            render_win,
            render_gc,
            scale: Cell::new(1.0),
            physical,
            touch_axis: RefCell::new(HashMap::new()),
            refresh_input_regions,
            input_region: RefCell::new(None),
            input_refresh_pending: Cell::new(false),
            key_injector,
        };
        plat.raise_overlays()?;
        Ok(plat)
    }

    pub fn sync_geometry(&self) -> XResult<Option<(i32, i32, f64)>> {
        let geo = xproto::get_geometry(&self.x11.conn,self.x11.root)?.reply()?;
        let dimensions = (geo.width as i32,geo.height as i32);
        let scale = self.x11.detect_scale();
        let (w,h) = ((dimensions.0 as f64/scale).round() as i32,(dimensions.1 as f64/scale).round() as i32);
        if !crate::app::valid_geometry(w,h,scale) { return Err("X11 显示尺寸或缩放无效".into()); }
        if dimensions == self.physical.get() && (self.scale.get()-scale).abs() < 1e-3 { return Ok(None); }
        for window in [self.win,self.render_win] {
            xproto::configure_window(&self.x11.conn,window,&ConfigureWindowAux::new()
                .x(0).y(0).width(geo.width as u32).height(geo.height as u32))?.check()?;
        }
        self.physical.set(dimensions); self.scale.set(scale);
        self.touch_axis.borrow_mut().clear();
        self.input_region.borrow_mut().take();
        Ok(Some((w,h,scale)))
    }

    /// Keep the input surface above the permanent visual surface. Only the
    /// transparent input surface is remapped; ink and toolbars stay visible.
    pub fn raise_overlays(&self) -> XResult<()> {
        if self.render_win != self.win {
            xproto::configure_window(
                &self.x11.conn, self.render_win,
                &ConfigureWindowAux::new().stack_mode(xproto::StackMode::ABOVE),
            )?.check()?;
        }
        xproto::configure_window(
            &self.x11.conn, self.win,
            &ConfigureWindowAux::new().stack_mode(xproto::StackMode::ABOVE),
        )?.check()?;
        Ok(())
    }

    /// Mutter can retain the old pointer target after an XShape change even
    /// though a fresh actor pick is correct. Remapping invalidates that cache.
    /// Never unmap during an active press/touch: that would cancel its grab and
    /// can discard the release. The caller redraws the retained canvas afterward.
    pub fn refresh_input_window(&self, input_idle: bool) -> XResult<bool> {
        if !self.input_refresh_pending.get() || !input_idle {
            return Ok(false);
        }
        let pointer = xproto::query_pointer(&self.x11.conn, self.x11.root)?.reply()?;
        let buttons = xproto::KeyButMask::BUTTON1 | xproto::KeyButMask::BUTTON2
            | xproto::KeyButMask::BUTTON3 | xproto::KeyButMask::BUTTON4
            | xproto::KeyButMask::BUTTON5;
        if u16::from(pointer.mask & buttons) != 0 {
            return Ok(false);
        }
        xproto::unmap_window(&self.x11.conn, self.win)?.check()?;
        xproto::map_window(&self.x11.conn, self.win)?.check()?;
        self.raise_overlays()?;
        if self.render_win != self.win {
            xproto::clear_area(&self.x11.conn, false, self.win, 0, 0, 0, 0)?.check()?;
        }
        self.input_refresh_pending.set(false);
        Ok(true)
    }

    /// 由 XI2 触摸事件的 valuator 计算触点直径（逻辑像素）
    pub fn touch_diameter(&self, devid: u8, mask: &[u32], values: &[Fp3232]) -> f64 {
        if !self.touch_axis.borrow().contains_key(&devid) {
            let info = self.load_touch_axis(devid);
            self.touch_axis.borrow_mut().insert(devid, info);
        }
        let axis = *self.touch_axis.borrow().get(&devid).unwrap();
        // 优先 Touch Major，其次 Touch Minor（之前会把先遇到的 Minor 当 Major）
        let Some((idx, amin, amax)) = axis.major.or(axis.minor) else {
            return 0.0;
        };
        let Some(mv) = fp_value(mask, values, idx) else {
            return 0.0;
        };
        let screen = self.physical.get().0 as f64;
        // 优先用尺寸轴自身量程归一化到屏幕像素；量程无效(0)时退回位置轴量程
        let physical = if amax > amin {
            (mv - amin) / (amax - amin) * screen
        } else if axis.posx_max > axis.posx_min {
            mv * screen / (axis.posx_max - axis.posx_min)
        } else {
            mv
        };
        let logical = (physical / self.scale.get().max(0.001)).clamp(0.0, 4096.0);
        if std::env::var("SIDERA_TRACE").is_ok() {
            log::info!(
                "[TOUCH] dev {} idx={} raw={:.1} range=({:.0},{:.0}) -> {:.1}px",
                devid,
                idx,
                mv,
                amin,
                amax,
                logical
            );
        }
        logical
    }

    fn load_touch_axis(&self, devid: u8) -> TouchAxis {
        let mut a = TouchAxis::default();
        if let Some(reply) = self.query_devices(devid as u16) {
            for info in &reply.infos {
                if info.deviceid as u8 == devid {
                    self.fill_touch_axis(&mut a, info);
                }
            }
        }
        // 回退：deviceid/sourceid 混淆或该设备无尺寸轴时，遍历所有设备找带尺寸轴的
        if a.major.is_none() && a.minor.is_none() {
            if let Some(reply) = self.query_devices(0) {
                for info in &reply.infos {
                    let mut t = TouchAxis::default();
                    self.fill_touch_axis(&mut t, info);
                    if t.major.is_some() || t.minor.is_some() {
                        log::info!(
                            "[TOUCH] dev {} 无尺寸轴，回退用设备 {} 的轴",
                            devid,
                            info.deviceid
                        );
                        a = t;
                        break;
                    }
                }
            }
        }
        log::info!(
            "[TOUCH] dev {} major={:?} minor={:?} posx=({:.0},{:.0})",
            devid,
            a.major,
            a.minor,
            a.posx_min,
            a.posx_max
        );
        a
    }

    fn query_devices(&self, devid: u16) -> Option<xinput::XIQueryDeviceReply> {
        xinput::xi_query_device(&self.x11.conn, devid)
            .ok()
            .and_then(|c| c.reply().ok())
    }

    fn fill_touch_axis(&self, a: &mut TouchAxis, info: &xinput::XIDeviceInfo) {
        for class in &info.classes {
            if let DeviceClassData::Valuator(v) = &class.data {
                let vmin = fp3232_to_f64(v.min);
                let vmax = fp3232_to_f64(v.max);
                if v.label == self.x11.atom_touch_major {
                    a.major = Some((v.number, vmin, vmax));
                } else if v.label == self.x11.atom_touch_minor {
                    a.minor = Some((v.number, vmin, vmax));
                } else if v.label == self.x11.atom_pos_x {
                    a.posx_min = vmin;
                    a.posx_max = vmax;
                }
                // 打印所有 valuator，便于定位驱动有没有上报尺寸轴
                if std::env::var("SIDERA_TRACE").is_ok() {
                    let name = xproto::get_atom_name(&self.x11.conn, v.label)
                        .ok()
                        .and_then(|c| c.reply().ok())
                        .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                        .unwrap_or_default();
                    log::info!(
                        "[TOUCH-AXIS] dev {} #{} label='{}' range=({:.0},{:.0})",
                        info.deviceid,
                        v.number,
                        name,
                        vmin,
                        vmax
                    );
                }
            }
        }
    }
}

fn fp3232_to_f64(v: Fp3232) -> f64 {
    v.integral as f64 + (v.frac as f64) / 4294967296.0
}

/// 从 XI2 valuator_mask/axisvalues 中取指定 valuator 的值
fn fp_value(mask: &[u32], values: &[Fp3232], idx: u16) -> Option<f64> {
    let mut k = 0usize;
    for (i, m) in mask.iter().enumerate() {
        let mut bits = *m;
        while bits != 0 {
            let b = bits.trailing_zeros();
            let number = (i as u32) * 32 + b;
            if number as u16 == idx {
                return values.get(k).map(|v| fp3232_to_f64(*v));
            }
            k += 1;
            bits &= bits - 1;
        }
    }
    None
}

impl crate::backend::Backend for X11Plat {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn screen_size(&self) -> (i32, i32) {
        let sc = self.scale.get().max(0.001);
        (
            ((self.physical.get().0 as f64) / sc).round() as i32,
            ((self.physical.get().1 as f64) / sc).round() as i32,
        )
    }
    fn present(&self, pm: &tiny_skia::Pixmap, x: i32, y: i32) {
        let sc = self.scale.get();
        let px = ((x as f64) * sc).round() as i32;
        let py = ((y as f64) * sc).round() as i32;
        let _ = self.x11.put_pixmap(self.render_win, self.render_gc, pm, px, py);
    }
    fn set_input_region(&self, rects: &[crate::backend::IRect], full: bool) {
        let sc = self.scale.get();
        let xr: Vec<Rectangle> = rects
            .iter()
            .map(|r| Rectangle {
                x: ((r.x as f64) * sc).round() as i16,
                y: ((r.y as f64) * sc).round() as i16,
                width: (((r.w as f64) * sc).round() as i32).max(0) as u16,
                height: (((r.h as f64) * sc).round() as i32).max(0) as u16,
            })
            .collect();
        let region = (full, if full { Vec::new() } else { xr });
        if self.input_region.borrow().as_ref().is_some_and(|old| {
            old.0 == region.0 && old.1.len() == region.1.len()
                && old.1.iter().zip(&region.1).all(|(a, b)| {
                    (a.x, a.y, a.width, a.height) == (b.x, b.y, b.width, b.height)
                })
        }) {
            return;
        }
        match self.x11.set_input_shape(self.win, &region.1, full) {
            Ok(()) => {
                *self.input_region.borrow_mut() = Some(region);
                self.input_refresh_pending.set(self.refresh_input_regions);
            }
            Err(error) => log::warn!("[WARN] 更新 X11 点击区域失败: {error}"),
        }
    }
    fn set_text_input(&self, on: bool) {
        if on {
            if !self.grab_text_keyboard() {
                log::warn!("无法抓取键盘，进程名可先复制后用“粘贴”");
            }
        } else {
            self.ungrab_text_keyboard();
        }
    }
    fn fake_key(&self, key: crate::backend::Key) {
        use crate::backend::Key;
        if self.x11.is_xwayland {
            if let Some(injector) = self.key_injector.borrow_mut().as_mut() {
                injector.key(match key { Key::Up => 103, Key::Down => 108, Key::Escape => 1 });
            }
            return;
        }
        let kc = match key {
            Key::Up => self.x11.key_up,
            Key::Down => self.x11.key_down,
            Key::Escape => self.x11.key_escape,
        };
        self.x11.fake_key(kc);
    }
    fn start_presentation(&self) -> bool {
        if self.x11.is_xwayland {
            if let Some(injector) = self.key_injector.borrow_mut().as_mut() {
                injector.start_presentation();
                return true;
            }
            return false;
        }
        self.x11.fake_start_show();
        true
    }
    fn can_inject_keys_without_prompt(&self) -> bool {
        // Xwayland 的 XTEST 可能经 libei 请求 RemoteDesktop 权限。
        !self.x11.is_xwayland || self.key_injector.borrow().is_some()
    }
    fn virtual_right_click(&self, x: i32, y: i32) {
        // Xwayland XTEST can start a RemoteDesktop authorization request.
        if self.x11.is_xwayland { return; }
        let sc = self.scale.get();
        self.x11.virtual_right_click(
            ((x as f64) * sc).round() as i32,
            ((y as f64) * sc).round() as i32,
        );
    }
    fn screenshot(&self) -> Option<(Vec<u8>, u32, u32)> {
        // Xwayland's root image omits native Wayland windows. Use Screenshot portal.
        if self.x11.is_xwayland { None } else { self.x11.screenshot() }
    }
    fn can_forward_right_click(&self) -> bool { !self.x11.is_xwayland }
    fn description(&self) -> &'static str {
        if self.x11.is_xwayland { "XWayland 兼容模式" } else { "原生 X11" }
    }
    fn is_presentation_fullscreen(&self) -> bool {
        self.x11.is_presentation_fullscreen(self.win)
    }
    fn presentation_foreground(&self) -> Option<bool> {
        self.x11.is_office_foreground(self.win)
    }
    fn has_compositor(&self) -> bool {
        self.x11.has_compositor()
    }
    fn shape_available(&self) -> bool {
        self.x11.shape_available()
    }
    fn grab_hotkey(&self) -> bool {
        self.x11.grab_hotkey()
    }
    fn set_scale(&self, s: f64) {
        self.scale.set(s.clamp(1.0, 4.0));
    }
    fn device_pixel_ratio(&self) -> f64 {
        self.scale.get()
    }
}

#[cfg(test)]
#[path = "x11_input_tests.rs"]
mod input_protocol_tests;

#[cfg(test)]
mod protocol_tests {
    use super::*;

    // Run only against an isolated server, for example:
    // xvfb-run -a <cargo-test-executable> x11::protocol_tests --ignored --nocapture
    // These tests validate protocol acceptance, not physical touchscreen behavior.
    #[test]
    #[ignore = "requires an isolated Xvfb server"]
    fn touch_selection_is_accepted_and_can_be_read_back() {
        let x11 = X11::connect().expect("connect to isolated Xvfb");
        let (win, _) = x11.create_window(0, 0, 64, 64).unwrap();
        x11.select_touch_events(win).expect("negotiate XI 2.2 and select touch events");

        let selected = xinput::xi_get_selected_events(&x11.conn, win)
            .unwrap()
            .reply()
            .unwrap();
        let expected = xinput::XIEventMask::TOUCH_BEGIN
            | xinput::XIEventMask::TOUCH_UPDATE
            | xinput::XIEventMask::TOUCH_END
            | xinput::XIEventMask::TOUCH_OWNERSHIP;
        let all_devices = selected.masks.iter().find(|mask| mask.deviceid == 0).unwrap();
        assert_eq!(all_devices.mask, vec![expected]);
        let root_selected = xinput::xi_get_selected_events(&x11.conn, x11.root)
            .unwrap()
            .reply()
            .unwrap();
        assert!(root_selected.masks.is_empty(), "no global raw touch subscription");
        x11.destroy_window(win);
    }

    #[test]
    #[ignore = "requires an isolated Xvfb server"]
    fn old_raw_touch_selection_is_rejected_on_overlay() {
        let x11 = X11::connect().expect("connect to isolated Xvfb");
        let (win, _) = x11.create_window(0, 0, 64, 64).unwrap();
        xinput::xi_query_version(&x11.conn, 2, 2).unwrap().reply().unwrap();
        let old_mask = xinput::XIEventMask::TOUCH_BEGIN
            | xinput::XIEventMask::TOUCH_UPDATE
            | xinput::XIEventMask::TOUCH_END
            | xinput::XIEventMask::TOUCH_OWNERSHIP
            | xinput::XIEventMask::RAW_TOUCH_BEGIN
            | xinput::XIEventMask::RAW_TOUCH_UPDATE
            | xinput::XIEventMask::RAW_TOUCH_END;
        let result = xinput::xi_select_events(
            &x11.conn,
            win,
            &[xinput::EventMask { deviceid: 0, mask: vec![old_mask] }],
        )
        .unwrap()
        .check();
        assert!(matches!(
            result,
            Err(x11rb::errors::ReplyError::X11Error(ref error))
                if error.error_kind == x11rb::protocol::ErrorKind::Value
        ), "old subscription must report BadValue, got {result:?}");
        assert!(xinput::xi_get_selected_events(&x11.conn, win)
            .unwrap().reply().unwrap().masks.is_empty());
        x11.destroy_window(win);
    }
}
