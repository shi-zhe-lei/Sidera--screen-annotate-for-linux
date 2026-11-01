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
    if let Err(error) = run() {
        fatal(&error);
    }
}

fn run() -> Result<(), String> {
    let _single = match single_instance()? {
        Some(handle) => handle,
        None => return Ok(()),
    };
    log("run start");
    if std::env::var("SIDERA_AUTOSTART_TEST").is_ok() {
        log(&format!("autostart before={}", ui::is_autostart()));
        ui::set_autostart(true);
        log(&format!("autostart set={}", ui::is_autostart()));
        ui::set_autostart(false);
        log(&format!("autostart cleared={}", ui::is_autostart()));
        return Ok(());
    }
    let plat = Win32Plat::create()?;
    log("window created");
    let (logical_w, logical_h) = plat.logical_size();
    let (canvas_w, canvas_h) = plat.canvas_size();
    let scale = plat.scale();

    let canvas = Pixmap::new(canvas_w, canvas_h).ok_or("无法创建画布")?;
    let mut app = App::new(canvas, logical_w, logical_h);
    app::load_settings(&mut app);
    app.set_scale(scale);
    win32::refresh_theme();
    app.update_system_theme(win32::dark());
    app.hotkey_ok = true;

    log("app initialized");
    let fonts = Fonts::load();
    log("fonts loaded");
    let rt = Rt {
        backend: plat,
        fonts,
        icon: None,
        icon_big: None,
        batching: Cell::new(false),
        dirty: RefCell::new(Vec::new()),
    };

    if let Ok(path) = std::env::var("SIDERA_RENDER_PNG") {
        let pm = ui::render_region(&app, &rt.fonts, None, 0, 0, app.screen_w, app.screen_h)
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
                    TrayAction::Settings => open_settings(&rt, &mut app),
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
                WinEvent::DisplayChanged => {}
                WinEvent::Quit => return Ok(()),
            }
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
        rt.backend.flush();

        let busy = app.is_drawing
            || ip.dragging
            || ip.slider.is_some()
            || app.press.is_some()
            || app.launch.active()
            || app.page_picker.is_some();
        std::thread::sleep(Duration::from_millis(if busy { 4 } else { 12 }));
    }
}
