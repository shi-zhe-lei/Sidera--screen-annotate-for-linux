#![windows_subsystem = "windows"]

mod win32;

use std::cell::{Cell, RefCell};
use std::time::Duration;

use sidera_core::app::{self, App};
use sidera_core::text::Fonts;
use sidera_core::ui;
use sidera_core::wps::WpsBridge;
use sidera_core::{
    apply_input_shape, begin_launch, handle_btn, handle_touch, on_motion, on_press, on_release,
    on_scroll, open_settings, switch_to_cursor, switch_to_draw, tick, Ip, Rt, Timers,
};
use sidera_core::ui::Btn;
use tiny_skia::Pixmap;

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, SetLastError};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_OK, MessageBoxW,
};

use win32::{TrayAction, Win32Plat, WinEvent};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn log(message: &str) {
    let dir = std::env::var("TEMP")
        .or_else(|_| std::env::var("TMP"))
        .unwrap_or_else(|_| ".".into());
    let path = std::path::Path::new(&dir).join("sidera-win-startup.log");
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(file, "{message}");
    }
}

fn fatal(message: &str) {
    log(&format!("FATAL: {message}"));
    let text = wide(message);
    let caption = wide("Sidera · Windows");
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn save_png(pm: &Pixmap, path: &str) -> Result<(), String> {
    let mut rgba = Vec::with_capacity(pm.data().len());
    for px in pm.data().chunks_exact(4) {
        let a = px[3] as u32;
        let un = |c: u8| {
            if a == 0 {
                0
            } else {
                ((c as u32 * 255 + a / 2) / a).min(255) as u8
            }
        };
        rgba.extend_from_slice(&[un(px[0]), un(px[1]), un(px[2]), px[3]]);
    }
    let img = image::RgbaImage::from_raw(pm.width(), pm.height(), rgba)
        .ok_or("无法创建 PNG 缓冲")?;
    img.save(path).map_err(|e| e.to_string())
}

fn single_instance() -> Result<Option<isize>, String> {
    let name = wide("Local\\SideraWindowsOverlay");
    unsafe {
        SetLastError(0);
        let handle = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
        if handle.is_null() {
            return Err("创建单实例互斥量失败".into());
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(None);
        }
        Ok(Some(handle as isize))
    }
}

fn init_log() {
    let path = std::env::temp_dir().join("sidera-win.log");
    if let Ok(file) = std::fs::File::create(path) {
        let _ = env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("debug"),
        )
        .target(env_logger::Target::Pipe(Box::new(file)))
        .try_init();
    }
}

fn main() {
    init_log();
    log("main start");
    win32::init_dpi();
    log("dpi ok");
    let result = run();
    sidera_core::process_block::stop_daemon();
    if let Err(error) = result {
        fatal(&error);
    }
}

fn close_native_settings(rt: &sidera_core::Rt, app: &mut App, ip: &mut Ip) {
    if let Some(plat) = rt.backend.as_any().downcast_ref::<Win32Plat>() {
        plat.close_settings_window();
    }
    app.settings_ui.native_size = None;
    app.settings_ui.native_dirty.set(false);
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
    rt.backend.set_text_input(false);
    ip.pending_ui = None;
    ip.slider = None;
    ip.panel_scroll = None;
    ip.picker_drag = None;
    app.press = None;
    sidera_core::apply_input_shape(rt, app);
    rt.redraw_full(app);
}

fn run() -> Result<(), String> {
    log("run start");
    if std::env::var("SIDERA_AUTOSTART_TEST").is_ok() {
        log(&format!("autostart before={}", ui::is_autostart()));
        ui::set_autostart(true);
        log(&format!("autostart set={}", ui::is_autostart()));
        ui::set_autostart(false);
        log(&format!("autostart cleared={}", ui::is_autostart()));
        return Ok(());
    }
    if app::launched_by_autostart() {
        let delay = app::load_startup_delay();
        if delay > 0 {
            log(&format!("autostart delay {delay}s"));
            std::thread::sleep(Duration::from_secs(u64::from(delay)));
        }
    }
    let _single = match single_instance()? {
        Some(handle) => handle,
        None => return Ok(()),
    };
    let plat = Win32Plat::create()?;
    log("window created");
    let (logical_w, logical_h) = plat.logical_size();
    let (canvas_w, canvas_h) = plat.canvas_size();
    let scale = plat.scale();

    let canvas = Pixmap::new(canvas_w, canvas_h).ok_or("无法创建画布")?;
    let mut app = App::new(canvas, logical_w, logical_h);
    app::load_settings(&mut app);
    app.set_scale(scale);
    sidera_core::process_block::set_daemon_enabled(app.process_block_enabled);
    win32::refresh_theme();
    app.update_system_theme(win32::dark());
    app.hotkey_ok = true;
    if app.wps_debug {
        sidera_core::wps::ensure_addin_registered_if_needed();
    }

    log("app initialized");
    let fonts = Fonts::load();
    log("fonts loaded");
    let rt = Rt {
        backend: plat,
        fonts,
        icon: sidera_core::ui_icons::app_icon(44),
        icon_big: sidera_core::ui_icons::app_icon(84),
        batching: Cell::new(false),
        dirty: RefCell::new(Vec::new()),
    };

    if let Ok(path) = std::env::var("SIDERA_RENDER_PNG") {
        let pm = ui::render_region(&app, &rt.fonts, rt.icon.as_ref(), 0, 0, app.screen_w, app.screen_h)
            .ok_or("渲染失败")?;
        save_png(&pm, &path)?;
        log(&format!("rendered {path}"));
        return Ok(());
    }

    if let Ok(path) = std::env::var("SIDERA_DUMP_DIB") {
        rt.redraw_full(&app);
        rt.backend.flush();
        rt.backend
            .as_any()
            .downcast_ref::<Win32Plat>()
            .ok_or("后端类型错误")?
            .dump_png(&path)?;
        log(&format!("dumped {path}"));
        return Ok(());
    }

    if std::env::var("SIDERA_SETTINGS_SMOKE").is_ok() {
        let plat = rt.backend.as_any().downcast_ref::<Win32Plat>().unwrap();
        if let Some((w, h)) = plat.create_settings_window() {
            app.settings_ui.native_size = Some((w, h));
        }
        open_settings(&rt, &mut app);
        plat.paint_settings(&app, &rt.fonts, rt.icon.as_ref());
        log("settings smoke ok");
        return Ok(());
    }

    let mut ip = Ip::default();
    let mut wps_bridge = if app.wps_debug { WpsBridge::start() } else { None };
    let mut timers = Timers::new();

    log("starting loop");
    begin_launch(&rt, &mut app);
    apply_input_shape(&rt, &app);
    rt.redraw_full(&app);
    rt.backend.flush();

    loop {
        win32::pump_messages();
        rt.begin_batch();

        let plat = rt
            .backend
            .as_any()
            .downcast_ref::<Win32Plat>()
            .ok_or("Windows 后端类型错误")?;
        while let Some(event) = plat.poll_event() {
            match event {
                WinEvent::MouseDown(x, y, button) => {
                    on_press(&rt, &mut app, &mut ip, wps_bridge.as_ref(), x, y, button)
                }
                WinEvent::MouseMove(x, y) => on_motion(&rt, &mut app, &mut ip, x, y),
                WinEvent::MouseUp(x, y, button) => {
                    on_release(&rt, &mut app, &mut ip, wps_bridge.as_ref(), x, y, button)
                }
                WinEvent::Wheel(x, y, pages) => on_scroll(&rt, &mut app, &ip, x, y, pages),
                WinEvent::Touch {
                    id,
                    kind,
                    x,
                    y,
                    diameter,
                } => handle_touch(
                    &rt,
                    &mut app,
                    &mut ip,
                    wps_bridge.as_ref(),
                    id,
                    x,
                    y,
                    kind,
                    diameter,
                ),
                WinEvent::PenMode => { switch_to_draw(&rt, &mut app, 1); }
                WinEvent::ToggleMode => {
                    if app.mode == 0 {
                        switch_to_draw(&rt, &mut app, 1);
                    } else {
                        switch_to_cursor(&rt, &mut app);
                    }
                }
                WinEvent::Tray(action) => {
                    log(&format!("tray {action:?}"));
                    match action {
                    TrayAction::Settings => {
                        let plat = rt.backend.as_any().downcast_ref::<Win32Plat>().unwrap();
                        if plat.settings_window_open() {
                            app.settings_ui.closing_at = 0;
                            app.settings_ui.native_dirty.set(true);
                            plat.raise_settings_window();
                        } else {
                            let size = plat.create_settings_window();
                            if let Some((w, h)) = size {
                                app.settings_ui.native_size = Some((w, h));
                            }
                            open_settings(&rt, &mut app);
                            if size.is_some() {
                                sidera_core::apply_input_shape(&rt, &app);
                                rt.clear_overlay(&app);
                                app.settings_ui.native_dirty.set(true);
                            }
                        }
                    }
                    TrayAction::Toggle => {
                        if app.mode == 0 {
                            switch_to_draw(&rt, &mut app, 1);
                        } else {
                            switch_to_cursor(&rt, &mut app);
                        }
                    }
                    TrayAction::Whiteboard => {
                        handle_btn(&rt, &mut app, wps_bridge.as_ref(), Btn::Whiteboard, false)
                    }
                    TrayAction::Show => {
                        plat.show();
                        rt.redraw_full(&app);
                    }
                    TrayAction::Quit => return Ok(()),
                    }
                },
                WinEvent::ThemeChanged => {
                    win32::refresh_theme();
                    if app.update_system_theme(win32::dark()) {
                        rt.redraw_full(&app);
                    }
                }
                WinEvent::DisplayChanged => {
                    if let Some(plat) = rt.backend.as_any().downcast_ref::<Win32Plat>() {
                        if let Some((w, h, scale)) = plat.refresh_display() {
                            app.configure_geometry(w, h, scale);
                            if plat.settings_window_open() {
                                plat.close_settings_window();
                                if let Some((w, h)) = plat.create_settings_window() {
                                    app.settings_ui.native_size = Some((w, h));
                                    app.settings_ui.native_dirty.set(true);
                                }
                            }
                            sidera_core::apply_input_shape(&rt, &app);
                            rt.redraw_full(&app);
                        }
                    }
                }
                WinEvent::ValueKey(key) => { sidera_core::text_input_key(&rt, &mut app, key); }
                WinEvent::SettingsClose => {
                    if app.settings_ui.closing_at == 0 {
                        app.settings_ui.closing_at = sidera_core::app::now_ms();
                        app.settings_ui.native_dirty.set(true);
                    }
                }
                WinEvent::SettingsResized => {
                    if let Some(plat) = rt.backend.as_any().downcast_ref::<Win32Plat>() {
                        if let Some((w, h)) = plat.settings_logical_size() {
                            app.settings_ui.native_size = Some((w, h));
                            app.settings_ui.native_dirty.set(true);
                        }
                    }
                }
                WinEvent::Quit => return Ok(()),
            }
        }

        if let Some(plat) = rt.backend.as_any().downcast_ref::<Win32Plat>() {
            if let Some(size) = plat.settings_logical_size() {
                if app.settings_ui.native_size != Some(size) {
                    app.settings_ui.native_size = Some(size);
                    app.settings_ui.native_dirty.set(true);
                }
            }
        }
        if app.settings_ui.closing_at > 0
            && sidera_core::app::now_ms() - app.settings_ui.closing_at >= 120
        {
            close_native_settings(&rt, &mut app, &mut ip);
        }

        tick(
            &rt,
            &mut app,
            &mut ip,
            &mut wps_bridge,
            &mut timers,
            win32::dark(),
        );
        rt.end_batch(&app);
        if app.settings_ui.native_dirty.get() {
            app.settings_ui.native_dirty.set(false);
            if let Some(plat) = rt.backend.as_any().downcast_ref::<Win32Plat>() {
                plat.paint_settings(&app, &rt.fonts, rt.icon.as_ref());
            }
        }
        rt.backend.flush();

        let busy = app.is_drawing
            || ip.dragging
            || ip.slider.is_some()
            || ip.pending_ui.is_some()
            || app.press.is_some()
            || app.launch.active()
            || sidera_core::ui::sidebar_animating(&app)
            || app.tool_selector.animating()
            || app.settings_ui.animating()
            || app.settings_ui.native_dirty.get()
            || app.open_anim != 0
            || app.page_picker.is_some();
        std::thread::sleep(Duration::from_millis(if busy { 4 } else { 12 }));
    }
}
