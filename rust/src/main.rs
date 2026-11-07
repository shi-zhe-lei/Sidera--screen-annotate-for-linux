//! Sidera Linux entry point (X11 / Wayland / GNOME).
//! Shared logic lives in the `sidera_core` library.
// Copyright (C) 2026 Carl_Jin   GNU GPL v3

#[cfg(unix)]
mod linux {
    use sidera_core::*;
    use sidera_core::app::{now_ms, App};
    use sidera_core::gesture::TouchKind;
    use sidera_core::instance::Instance as UnixListener;
    use sidera_core::text::Fonts;
    use sidera_core::ui::Btn;
    use sidera_core::wps::WpsBridge;

    use std::collections::HashSet;
    use std::io::Read;
    use std::time::Duration;

    use tiny_skia::Pixmap;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, KeyButMask};
    use x11rb::protocol::Event;

fn process_key(rt: &Rt, app: &mut App, keycode: u8, modifiers: KeyButMask) -> bool {
    let Some(platform) = rt.backend.as_any().downcast_ref::<x11::X11Plat>() else {
        return false;
    };
    if let Some(key) = platform.x11.text_key(keycode, modifiers) {
        text_input_key(rt, app, key);
    }
    true
}
fn touch_dev(deviceid: u16, sourceid: u16) -> u8 {
    (if sourceid != 0 { sourceid } else { deviceid }) as u8
}

/// X11 事件坐标是物理像素，需换算成逻辑像素；Wayland 事件本就是逻辑坐标（比例为 1）
#[inline]
fn to_logical(rt: &Rt, v: i32) -> i32 {
    let s = rt.backend.device_pixel_ratio();
    if s <= 0.0 {
        v
    } else {
        (v as f64 / s).round() as i32
    }
}

fn handle_event(rt: &Rt, app: &mut App, ip: &mut Ip, wps: Option<&WpsBridge>, ev: Event) {
    // 屏蔽 X 合成的鼠标事件，避免一次触摸被处理两遍 / 残留笔画
    if let Event::ButtonPress(_) | Event::ButtonRelease(_) | Event::MotionNotify(_) = &ev {
        // 有触摸按下中，或 evdev 触摸启用后（无真实指针则全屏蔽，否则触摸后 400ms 内屏蔽）
        if !ip.touch_ids.is_empty()
            || (ip.evdev_touch
                && (ip.ignore_core_pointer || now_ms() - ip.last_touch_ms < 400))
        {
            return;
        }
    }
    match ev {
        Event::Expose(e) => {
            if e.count == 0 {
                rt.redraw_full(app);
            } else {
                let _ = e.width;
            }
        }
        Event::ButtonPress(e) => {
            if std::env::var("SIDERA_TRACE").is_ok() {
                log::info!("[BTN] at ({},{}) detail={}", e.event_x, e.event_y, e.detail);
            }
            on_press(
                rt,
                app,
                ip,
                wps,
                to_logical(rt, e.event_x as i32),
                to_logical(rt, e.event_y as i32),
                e.detail,
            )
        }
        Event::MotionNotify(e) => {
            if std::env::var("SIDERA_TRACE").is_ok() {
                log::info!("[MOTION] ({},{})", e.event_x, e.event_y);
            }
            on_motion(
                rt,
                app,
                ip,
                to_logical(rt, e.event_x as i32),
                to_logical(rt, e.event_y as i32),
            )
        }
        Event::ButtonRelease(e) => on_release(
            rt,
            app,
            ip,
            wps,
            to_logical(rt, e.event_x as i32),
            to_logical(rt, e.event_y as i32),
            e.detail,
        ),
        Event::LeaveNotify(_) => {
            if ip.pending_ui.is_some() { on_motion(rt, app, ip, -1, -1); }
        }
        Event::KeyPress(e) => {
            if (app.settings_ui.editor.is_some() || (app.process_block_open && app.process_input_active))
                && process_key(rt, app, e.detail, e.state) { return; }
            let mods = KeyButMask::CONTROL | KeyButMask::SHIFT;
            let key_d = rt
                .backend
                .as_any()
                .downcast_ref::<x11::X11Plat>()
                .map(|p| p.x11.key_d)
                .unwrap_or(0);
            if e.detail == key_d && (e.state & mods) == mods {
                if app.mode != 0 {
                    switch_to_cursor(rt, app);
                } else {
                    switch_to_draw(rt, app, 1);
                }
            }
        }
        Event::XinputRawTouchBegin(e) | Event::XinputRawTouchUpdate(e) => {
            if let Some(xp) = rt.backend.as_any().downcast_ref::<x11::X11Plat>() {
                let d = xp.touch_diameter(touch_dev(e.deviceid, e.sourceid), &e.valuator_mask, &e.axisvalues_raw);
                if d > 0.0 {
                    ip.touch_shape.insert(e.detail as i32, d);
                }
            }
        }
        Event::XinputRawTouchEnd(e) => {
            ip.touch_shape.remove(&(e.detail as i32));
        }
        Event::XinputTouchBegin(e) => {
            let x = to_logical(rt, (e.event_x as f64 / 65536.0).round() as i32);
            let y = to_logical(rt, (e.event_y as f64 / 65536.0).round() as i32);
            let d = ip.touch_shape.get(&(e.detail as i32)).copied().unwrap_or_else(|| {
                rt.backend
                    .as_any()
                    .downcast_ref::<x11::X11Plat>()
                    .map(|xp| xp.touch_diameter(touch_dev(e.deviceid, e.sourceid), &e.valuator_mask, &e.axisvalues))
                    .unwrap_or(0.0)
            });
            let id = e.detail as i32;
            let wants_touch = app.launch.active() || app.whiteboard || app.mode != 0
                || app.settings_open || app.confirm_quit || app.show_diag || app.show_input_group_prompt
                || dismiss_capture(app) || over_ui(app, x, y);
            log::info!("[TOUCH] begin id={} ({},{}) d={:.1} mode={} want={}",
                e.detail, x, y, d, app.mode, wants_touch);
            if let Some(xp) = rt.backend.as_any().downcast_ref::<x11::X11Plat>() {
                // Cursor mode must stay click-through: rejecting ownership lets
                // WPS receive the touch instead of the full-screen overlay.
                xp.x11.allow_touch(e.deviceid, e.detail, e.event, wants_touch);
            }
            if wants_touch {
                handle_touch(rt, app, ip, wps, id, x, y, TouchKind::Begin, d);
            } else {
                if ip.rejected_touch.len() > 64 { ip.rejected_touch.clear(); }
                ip.rejected_touch.insert(id);
                use std::sync::atomic::{AtomicI64, Ordering};
                static LAST: AtomicI64 = AtomicI64::new(0);
                let now = now_ms();
                if now - LAST.load(Ordering::Relaxed) > 2000 {
                    LAST.store(now, Ordering::Relaxed);
                    diag_log::info(&format!("光标模式触摸穿透：id={id} ({x},{y})"));
                }
            }
        }
        Event::XinputTouchUpdate(e) => {
            let id = e.detail as i32;
            if ip.rejected_touch.contains(&id) { return; }
            let x = to_logical(rt, (e.event_x as f64 / 65536.0).round() as i32);
            let y = to_logical(rt, (e.event_y as f64 / 65536.0).round() as i32);
            let d = ip.touch_shape.get(&id).copied().unwrap_or_else(|| {
                rt.backend
                    .as_any()
                    .downcast_ref::<x11::X11Plat>()
                    .map(|xp| xp.touch_diameter(touch_dev(e.deviceid, e.sourceid), &e.valuator_mask, &e.axisvalues))
                    .unwrap_or(0.0)
            });
            handle_touch(rt, app, ip, wps, id, x, y, TouchKind::Update, d);
        }
        Event::XinputTouchEnd(e) => {
            let id = e.detail as i32;
            if ip.rejected_touch.remove(&id) { return; }
            let x = to_logical(rt, (e.event_x as f64 / 65536.0).round() as i32);
            let y = to_logical(rt, (e.event_y as f64 / 65536.0).round() as i32);
            handle_touch(rt, app, ip, wps, id, x, y, TouchKind::End, 0.0);
        }
        Event::XinputTouchOwnership(e) => {
            let accept = !ip.rejected_touch.contains(&(e.touchid as i32));
            if let Some(xp) = rt.backend.as_any().downcast_ref::<x11::X11Plat>() {
                xp.x11.allow_touch(e.deviceid, e.touchid, e.event, accept);
            }
        }
        _ => {}
    }
}
fn load_icon(size: u32) -> Option<Pixmap> {
    sidera_core::ui_icons::app_icon(size)
}

fn single_instance() -> Result<Option<UnixListener>, Box<dyn std::error::Error>> {
    let command: &[u8] = if std::env::args().any(|arg| arg == "--toggle") { b"toggle" } else { b"show" };
    let instance = instance::acquire(command).map_err(|error| format!("无法创建单实例通信文件：{error}"))?;
    if instance.is_none() { log::warn!("Sidera 已在运行，退出新实例"); }
    Ok(instance)
}
pub fn run() -> std::process::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    match run_application() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            startup::report_error(&error);
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_application() -> Result<(), Box<dyn std::error::Error>> {
    // 软件渲染（无 GPU 老机器）
    std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");

    let backend = startup::detect_backend()?;
    diag_log::info(&format!("Sidera {} 启动，后端：{:?}", app::VERSION, backend));

    ui::ensure_autostart_flag();
    if app::launched_by_autostart() {
        let delay = app::load_startup_delay();
        if delay > 0 {
            diag_log::info(&format!("自动启动延迟 {delay} 秒，等待系统侧边栏就绪"));
            std::thread::sleep(Duration::from_secs(u64::from(delay)));
        }
    }

    let single = single_instance()?;
    if single.is_none() {
        return Ok(());
    }

    system_theme::start();
    tray::start();
    let fonts = Fonts::load();
    let icon_big = load_icon(84);
    let icon = load_icon(44);
    if icon.is_some() {
        log::info!("[INFO] 已加载软件图标");
    } else {
        log::warn!("[WARN] 未找到 sidera.png，设置/闪屏将无图标（可设 SIDERA_ICON 指定）");
    }

    let gnome_desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()
        .split(':').any(|name| name.eq_ignore_ascii_case("gnome"));
    if backend == startup::DisplayBackend::Gnome || (backend == startup::DisplayBackend::Wayland && gnome_desktop) {
        if let Some(prepared) = gnome::prepare()? {
            log::info!("[INFO] 平台: GNOME 原生 Wayland");
            return gnome::run(prepared, fonts, icon, icon_big, single);
        }
        if backend == startup::DisplayBackend::Gnome {
            return Err("GNOME 原生扩展尚未启用。请运行 sidera-gnome-install，重新登录后启用 Sidera 原生批注扩展。也可通过 SIDERA_BACKEND=x11 使用兼容模式。".into());
        }
    }
    if backend == startup::DisplayBackend::Wayland {
        match wayland::prepare() {
            Ok(prepared) => {
                log::info!("[INFO] 平台: wayland");
                return wayland::run(prepared, fonts, icon, icon_big, single);
            }
            Err(wayland::PrepareError::MissingLayerShell) if startup::allow_x11_fallback() => {
                log::warn!("[WARN] 当前桌面没有 layer-shell，使用 X11/XWayland 兼容模式；全局热键、原生 Wayland 应用的翻页/截图等能力可能受桌面限制");
            }
            Err(error) => return Err(error.into()),
        }
    }

    let x11 = x11::X11::connect().map_err(|e| format!("无法连接 X11 桌面：{e}"))?;
    let scale = x11.detect_scale();
    let logical_w = ((x11.width as f64) / scale).round().max(1.0) as i32;
    let logical_h = ((x11.height as f64) / scale).round().max(1.0) as i32;
    log::info!(
        "[INFO] 平台: x11  屏幕 {}x{} (物理 {}x{})  缩放 {}",
        logical_w,
        logical_h,
        x11.width,
        x11.height,
        scale
    );

    let (win, gc) = x11.create_overlay().map_err(|e| format!("创建 X11 覆盖层失败：{e}"))?;

    // 触摸输入：先协商 XInput 2.2 并优先用 XInput2 触摸；不可用再退回 evdev。
    // 关键：不调用 XIQueryVersion(2,2) 时服务器按 2.0 处理，不会投递触摸事件。
    let (xi_major, xi_minor) = match x11.negotiate_xi() {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[WARN] XIQueryVersion 失败: {}", e);
            (0, 0)
        }
    };
    let xi22 = xi_major > 2 || (xi_major == 2 && xi_minor >= 2);
    let touch_ids = if xi22 { x11.touch_device_ids() } else { Vec::new() };
    log::info!(
        "[TOUCH] XInput {}.{}，触摸设备 {} 个",
        xi_major,
        xi_minor,
        touch_ids.len()
    );
    let mut evdev_touch = None;
    let mut xi_touch = false;
    if !touch_ids.is_empty() {
        match x11.select_touch_events(win) {
            Ok(()) => {
                xi_touch = true;
                log::info!("[TOUCH] 使用 XInput2 触摸");
            }
            Err(e) => log::warn!("[WARN] XInput2 触摸选择失败: {}", e),
        }
    }
    if !xi_touch {
        evdev_touch = touch_evdev::spawn(logical_w, logical_h);
        if evdev_touch.is_some() {
            log::info!("[TOUCH] 使用 evdev 内核触摸输入");
        }
    }

    let has_node = touch_evdev::has_touch_node();
    let in_group = touch_evdev::in_input_group();
    let need_input_group = !xi_touch && evdev_touch.is_none() && has_node && !in_group;
    log::info!(
        "[PERM] xi_touch={} evdev_started={} has_touch_node={} in_input_group={} -> prompt={}",
        xi_touch,
        evdev_touch.is_some(),
        has_node,
        in_group,
        need_input_group
    );

    let mut canvas = Pixmap::new(x11.width as u32, x11.height as u32).unwrap();
    canvas.fill(tiny_skia::Color::TRANSPARENT);
    let mut app = App::new(canvas, logical_w, logical_h);
    app::load_settings(&mut app);
    app.update_system_theme(system_theme::dark());
    // X11 DPI 缩放：逻辑尺寸 × scale = 物理分辨率画布
    app.set_scale(scale);
    app.show_input_group_prompt = need_input_group;
    process_block::set_daemon_enabled(app.process_block_enabled);

    let rt = Rt {
        backend: Box::new(x11::X11Plat::new(x11, win, gc)
            .map_err(|e| format!("初始化 X11 覆盖层失败：{e}"))?),
        fonts,
        icon,
        icon_big,
        batching: std::cell::Cell::new(false),
        dirty: std::cell::RefCell::new(Vec::new()),
    };
    rt.backend.set_scale(scale);

    let xp = rt
        .backend
        .as_any()
        .downcast_ref::<x11::X11Plat>()
        .expect("x11 backend");
    app.hotkey_ok = rt.backend.grab_hotkey();
    let x11 = &xp.x11;
    let win = xp.win;
    let _gc = xp.gc;

    if !x11.has_compositor() {
        log::warn!(
            "[WARN] 未检测到 X11 合成器！透明画布将无法显示。请开启合成器，例如: picom & 或 xcompmgr &"
        );
    }

    // 冒烟测试：渲染一帧后立即退出（用于无显示环境自检）
    if std::env::var("SIDERA_SMOKE").is_ok() {
        log::info!("[SMOKE] 渲染一帧后退出");
        rt.redraw_full(&app);
        std::thread::sleep(Duration::from_millis(200));
        x11.destroy_window(win);
        return Ok(());
    }

    // 调试：画一条穿过侧边栏的笔迹并保存整屏 PNG，用于检查渲染顺序
    if let Ok(path) = std::env::var("SIDERA_DEBUG_DRAW") {
        app.mode = 1;
        paint::stroke_tapered(&mut app, (0, 800), (400, 800), 6, 6);
        paint::stroke_tapered(&mut app, (30, 620), (30, 900), 6, 6);
        paint::stroke_tapered(&mut app, (2500, 800), (2560, 800), 6, 6);
        // 模拟一条密集的鼠标曲线（多采样点并集）
        let mut pts: Vec<(f32, f32, f32)> = Vec::new();
        for i in 0..220 {
            let t = i as f32 / 219.0;
            let x = 300.0 + t * 700.0;
            let y = 900.0 + (t * 12.0).sin() * 90.0;
            pts.push((x, y, 10.0));
        }
        let c = app.pen_color();
        paint::draw_stroke_union(&mut app.canvas, c, &pts, tiny_skia::Transform::identity());
        if let Some(pm) =
            ui::render_region(&app, &rt.fonts, rt.icon.as_ref(), 0, 0, app.screen_w, app.screen_h)
        {
            let mut rgba = vec![0u8; pm.data().len()];
            for (i, px) in pm.data().chunks_exact(4).enumerate() {
                let a = px[3] as u32;
                let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a / 2) / a).min(255) as u8 };
                rgba[i * 4] = un(px[0]);
                rgba[i * 4 + 1] = un(px[1]);
                rgba[i * 4 + 2] = un(px[2]);
                rgba[i * 4 + 3] = px[3];
            }
            if let Some(img) = image::RgbaImage::from_raw(pm.width(), pm.height(), rgba) {
                let _ = img.save(&path);
                log::info!("[DEBUG] 已保存渲染结果: {}", path);
            }
        }
        x11.destroy_window(win);
        return Ok(());
    }

    // 调试：直接打开设置面板并渲染保存
    if let Ok(path) = std::env::var("SIDERA_DEBUG_SETTINGS") {
        app.settings_open = true;
        if let Some(pm) =
            ui::render_region(&app, &rt.fonts, rt.icon.as_ref(), 0, 0, app.screen_w, app.screen_h)
        {
            let mut rgba = vec![0u8; pm.data().len()];
            for (i, px) in pm.data().chunks_exact(4).enumerate() {
                let a = px[3] as u32;
                let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a / 2) / a).min(255) as u8 };
                rgba[i * 4] = un(px[0]);
                rgba[i * 4 + 1] = un(px[1]);
                rgba[i * 4 + 2] = un(px[2]);
                rgba[i * 4 + 3] = px[3];
            }
            if let Some(img) = image::RgbaImage::from_raw(pm.width(), pm.height(), rgba) {
                let _ = img.save(&path);
                log::info!("[DEBUG] 设置面板已保存: {}", path);
            }
        }
        x11.destroy_window(win);
        return Ok(());
    }

    // 调试：查询窗口的 XShape 输入区域
    if std::env::var("SIDERA_DEBUG_SHAPE").is_ok() {
        use x11rb::protocol::shape;
        apply_input_shape(&rt, &app);
        std::thread::sleep(Duration::from_millis(200));
        match shape::get_rectangles(&x11.conn, win, shape::SK::INPUT)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(r) => {
                let coords: Vec<(i16, i16, u16, u16)> = r
                    .rectangles
                    .iter()
                    .map(|x| (x.x, x.y, x.width, x.height))
                    .collect();
                log::info!("[SHAPE] 输入区域矩形 {} 个: {:?}", r.rectangles.len(), coords);
            }
            None => log::info!("[SHAPE] 查询失败"),
        }
        // 再切到绘画模式（全窗输入），查询是否恢复整窗
        app.mode = 1;
        apply_input_shape(&rt, &app);
        std::thread::sleep(Duration::from_millis(200));
        if let Some(r) = shape::get_rectangles(&x11.conn, win, shape::SK::INPUT)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            let coords: Vec<(i16, i16, u16, u16)> = r
                .rectangles
                .iter()
                .map(|x| (x.x, x.y, x.width, x.height))
                .collect();
            log::info!("[SHAPE] 绘画模式输入区域 {} 个: {:?}", r.rectangles.len(), coords);
        }
        x11.destroy_window(win);
        return Ok(());
    }

    // 调试：验证 XTest 指针移动是否生效（判断 XWayland 下能否注入事件）
    if std::env::var("SIDERA_TEST_XTEST").is_ok() {
        use x11rb::protocol::xproto;
        let before = xproto::query_pointer(&x11.conn, x11.root)
            .ok()
            .and_then(|c| c.reply().ok());
        x11.virtual_right_click(123, 456); // 内部会先 fake motion
        std::thread::sleep(Duration::from_millis(300));
        let after = xproto::query_pointer(&x11.conn, x11.root)
            .ok()
            .and_then(|c| c.reply().ok());
        if let (Some(b), Some(a)) = (before, after) {
            log::info!(
                "[XTEST] pointer before=({},{}) after=({},{})",
                b.root_x,
                b.root_y,
                a.root_x,
                a.root_y
            );
        }
        x11.destroy_window(win);
        return Ok(());
    }

    // Shared, non-blocking startup card; the normal event loop drives its exit.
    begin_launch(&rt, &mut app);

    // 初始光标模式 + 侧边栏输入区域
    apply_input_shape(&rt, &app);
    rt.redraw_full(&app);

    // WPS 桥（若默认开启）
    let mut wps_bridge: Option<WpsBridge> = None;
    if app.wps_debug {
        wps_bridge = WpsBridge::start();
        if wps_bridge.is_none() {
            log::warn!("[WARN] WPS HTTP 16666 绑定失败");
        }
    }

    let mut ip = Ip::default();
    ip.evdev_touch = evdev_touch.is_some();
    ip.ignore_core_pointer = evdev_touch.is_some() && !touch_evdev::has_relative_pointer();
    ip.last_touch_ms = now_ms();
    if ip.evdev_touch {
        log::info!(
            "[TOUCH] evdev 触摸已启用，ignore_core_pointer={}",
            ip.ignore_core_pointer
        );
    }

    // 独立追踪真实 Begin/End，不使用手势层 1.5 秒超时清理后的状态
    // 来判断是否可以重映射；静止长按仍可能持有触摸抓取。
    let mut active_x11_touches = HashSet::new();


    let mut timers = Timers::new();

    loop {
        // 本帧开始累积脏区，事件处理期间不立即渲染
        rt.begin_batch();
        // 0. evdev 触摸（若有）
        if let Some(rx) = evdev_touch.as_ref() {
            while let Ok(contacts) = rx.try_recv() {
                for c in contacts {
                    // 起点不在“可交互”区域的触点，整段忽略（穿透给桌面）
                    match c.phase {
                        touch_evdev::Phase::Begin => {
                            if !point_input_ok(&app, c.x, c.y) {
                                ip.ignored_touch.insert(c.id);
                                continue;
                            }
                            ip.ignored_touch.remove(&c.id);
                        }
                        touch_evdev::Phase::Update => {
                            if ip.ignored_touch.contains(&c.id) {
                                continue;
                            }
                        }
                        touch_evdev::Phase::End => {
                            if ip.ignored_touch.remove(&c.id) {
                                continue;
                            }
                        }
                    }
                    let kind = match c.phase {
                        touch_evdev::Phase::Begin => TouchKind::Begin,
                        touch_evdev::Phase::Update => TouchKind::Update,
                        touch_evdev::Phase::End => TouchKind::End,
                    };
                    handle_touch(
                        &rt,
                        &mut app,
                        &mut ip,
                        wps_bridge.as_ref(),
                        c.id,
                        c.x,
                        c.y,
                        kind,
                        c.major,
                    );
                }
            }
        }
        // 1. X 事件
        loop {
            match x11.conn.poll_for_event() {
                Ok(Some(ev)) => {
                    let geometry_changed = match &ev {
                        Event::ConfigureNotify(e) => e.window == x11.root,
                        Event::PropertyNotify(e) => e.window == x11.root && e.atom == u32::from(x11rb::protocol::xproto::AtomEnum::RESOURCE_MANAGER),
                        _ => false,
                    };
                    if geometry_changed {
                        if let Some((w,h,scale)) = xp.sync_geometry()? {
                            cancel_input(&rt,&mut app,&mut ip);
                            active_x11_touches.clear();
                            app.configure_geometry(w,h,scale);
                            apply_input_shape(&rt,&app);
                            rt.redraw_full(&app);
                        }
                    }
                    match &ev {
                        Event::XinputTouchBegin(e) => {
                            active_x11_touches.insert((e.deviceid, e.detail));
                        }
                        Event::XinputTouchEnd(e) => {
                            active_x11_touches.remove(&(e.deviceid, e.detail));
                        }
                        _ => {}
                    }
                    handle_event(&rt, &mut app, &mut ip, wps_bridge.as_ref(), ev);
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(format!("X11 桌面连接已断开：{e}").into());
                }
            }
        }

        // 2. 单实例唤醒
        if let Some(l) = single.as_ref() {
            if let Ok((mut c, _)) = l.accept() {
                let mut buf = [0u8; 16];
                let _ = c.set_read_timeout(Some(Duration::from_millis(20)));
                let n = c.read(&mut buf).unwrap_or(0);
                if &buf[..n] == b"toggle" {
                    if app.mode == 0 { switch_to_draw(&rt, &mut app, 1); }
                    else { switch_to_cursor(&rt, &mut app); }
                }
                let _ = x11.conn.map_window(win);
                let _ = xp.raise_overlays();
                let _ = x11.conn.flush();
                rt.redraw_full(&app);
            }
        }

        while let Some(action) = tray::poll() {
            match action {
                tray::TrayAction::Settings => open_settings(&rt, &mut app),
                tray::TrayAction::Toggle => {
                    if app.mode == 0 {
                        switch_to_draw(&rt, &mut app, 1);
                    } else {
                        switch_to_cursor(&rt, &mut app);
                    }
                }
                tray::TrayAction::Whiteboard => {
                    handle_btn(&rt, &mut app, wps_bridge.as_ref(), Btn::Whiteboard, false);
                }
                tray::TrayAction::Show => {
                    let _ = x11.conn.map_window(win);
                    let _ = xp.raise_overlays();
                    let _ = x11.conn.flush();
                    rt.redraw_full(&app);
                }
                tray::TrayAction::Quit => {
                    process_block::stop_daemon();
                    std::process::exit(0);
                }
            }
        }

        tick(&rt, &mut app, &mut ip, &mut wps_bridge, &mut timers, system_theme::dark());

        // 输入区域变化后刷新 Mutter 的指针命中缓存。等释放事件处理完，
        // 再重映射同一覆盖层；批注画布保留并在本帧重新上屏。
        let input_idle = active_x11_touches.is_empty() && !ip.dragging && ip.slider.is_none()
            && !app.is_drawing && !app.page_picker.as_ref().is_some_and(|p| p.dragging());
        if xp.refresh_input_window(input_idle)
            .map_err(|e| format!("刷新 X11 点击区域失败：{e}"))? {
            rt.redraw_full(&app);
        }

        // 一帧结束：合并相交脏区，保留左右工具栏独立刷新。
        rt.end_batch(&app);

        // Keep the low-latency frame rate while drawing or animating, but let
        // low-end classroom CPUs idle between frames. This also reduces X11
        // traffic while WPS is presenting and Sidera has no input.
        let busy = app.is_drawing || ip.dragging || ip.slider.is_some()
            || ip.pending_ui.is_some() || app.press.is_some()
            || app.launch.active() || ui::sidebar_animating(&app)
            || app.settings_ui.animating()
            || app.open_anim != 0 || app.page_picker.is_some();
        std::thread::sleep(Duration::from_millis(if busy { 4 } else { 12 }));
    }
}
}

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    if std::env::args().any(|arg| arg == "--blocker") {
        return std::process::ExitCode::from(
            sidera_core::process_block::run_blocker() as u8,
        );
    }
    linux::run()
}

#[cfg(windows)]
fn main() {
    eprintln!("Sidera Windows: build the windows/ workspace, not this Linux binary.");
}
