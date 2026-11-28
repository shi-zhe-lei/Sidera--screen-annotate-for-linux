// ============================================================
// Sidera · Windows Win32 backend
//
// - 透明、置顶、逐像素 alpha 的覆盖层（UpdateLayeredWindow）
// - WM_NCHITTEST 局部点击穿透
// - 鼠标 / WM_POINTER 触摸 / 热键 / 截图 / SendInput
// ============================================================
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sidera_core::app::App;
use sidera_core::backend::{Backend, IRect, Key};
use sidera_core::text::Fonts;
use sidera_core::gesture::TouchKind;
use tiny_skia::Pixmap;

use windows_sys::Win32::Foundation::{
    CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, BeginPaint,
    BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, CombineRgn, CreateRectRgn, DeleteDC, DeleteObject, EndPaint, GetDC,
    GetDIBits, HDC, HBITMAP, HGDIOBJ, PAINTSTRUCT, RGN_OR, ReleaseDC,
    SRCCOPY, ScreenToClient, SelectObject, SetWindowRgn, ValidateRect,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
    RegisterHotKey, ReleaseCapture, SendInput, SetCapture, UnregisterHotKey, VK_DOWN, VK_ESCAPE, VK_F5, VK_SHIFT,
    VK_UP, GetKeyState, SetFocus, VK_CONTROL, VK_MENU,
};
use windows_sys::Win32::UI::Input::Pointer::{
    GetPointerInfo, GetPointerTouchInfo, POINTER_FLAG_INCONTACT, POINTER_INFO, POINTER_TOUCH_INFO,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GWL_EXSTYLE,
    GetClientRect, GetClassNameW,
    GetForegroundWindow, GetCursorPos, GetSystemMetrics, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId,
    GetWindowLongPtrW, HTCLIENT, HTTRANSPARENT, HWND_TOPMOST, IDC_ARROW, IDI_APPLICATION,
    LoadCursorW, LoadIconW, LoadImageW, MF_STRING, MSG, PM_REMOVE, PeekMessageW,
    PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    TrackPopupMenu, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
    ULW_ALPHA, UpdateLayeredWindowIndirect, UPDATELAYEREDWINDOWINFO, WNDCLASSW, LWA_ALPHA,
    WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_HOTKEY, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_PAINT, WM_POINTERDOWN, WM_POINTERUP, WM_POINTERUPDATE,
    WM_CLOSE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_CONTEXTMENU, WM_LBUTTONDBLCLK,
    WM_KEYDOWN, WM_CHAR, WM_SIZE,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    WS_OVERLAPPEDWINDOW, WS_POPUP, WS_VISIBLE, HICON, IMAGE_ICON, LR_DEFAULTSIZE,
    LR_LOADFROMFILE,
};

const HOTKEY_TOGGLE: i32 = 1;
const HOTKEY_QUIT: i32 = 2;
const HOTKEY_PEN: i32 = 3;
const WM_APP_TRAY: u32 = 0x8001;
const TRAY_ID: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Settings,
    Toggle,
    Whiteboard,
    Show,
    Quit,
}

#[derive(Clone)]
pub enum WinEvent {
    MouseDown(i32, i32, u8),
    MouseMove(i32, i32),
    MouseUp(i32, i32, u8),
    Wheel(i32, i32, f64),
    Touch { id: i32, kind: TouchKind, x: i32, y: i32, diameter: f64 },
    ToggleMode,
    PenMode,
    Tray(TrayAction),
    Quit,
    ThemeChanged,
    DisplayChanged,
    ValueKey(sidera_core::value_input::Key),
    SettingsClose,
    SettingsResized,
}

pub struct Win32Plat {
    visual_hwnd: HWND,
    input_hwnd: HWND,
    origin: Cell<(i32, i32)>,
    phys_w: Cell<i32>,
    phys_h: Cell<i32>,
    scale: Cell<f64>,
    events: RefCell<VecDeque<WinEvent>>,
    input_rects: RefCell<Vec<IRect>>,
    full_input: Cell<bool>,
    text_input: Cell<bool>,
    previous_focus: Cell<HWND>,
    touch_last: RefCell<HashMap<u32, (i32, i32, f64)>>,
    hdc_screen: HDC,
    hdc_mem: HDC,
    dib: Cell<HBITMAP>,
    old_bitmap: Cell<HGDIOBJ>,
    bits: Cell<*mut u8>,
    dirty: Cell<bool>,
    dirty_rect: Cell<Option<(i32, i32, i32, i32)>>,
    update_failed: Cell<bool>,
    last_topmost: Cell<Instant>,
    tray_icon: HICON,
    tray_icon_owned: bool,
    pending_dpi: Cell<Option<u32>>,
    settings: RefCell<Option<SettingsWindow>>,
}

struct SettingsWindow {
    hwnd: HWND,
    hdc_mem: HDC,
    dib: HBITMAP,
    old_bitmap: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn lo(v: isize) -> i32 {
    (v as u32 & 0xffff) as i16 as i32
}

fn hi(v: isize) -> i32 {
    ((v as u32 >> 16) & 0xffff) as i16 as i32
}

pub fn init_dpi() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn pump_messages() {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

static DARK: AtomicBool = AtomicBool::new(false);

pub fn refresh_theme() {
    DARK.store(system_dark(), Ordering::Relaxed);
}

pub fn dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

pub fn system_dark() -> bool {
    let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value_name = wide("AppsUseLightTheme");
    let mut value: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            &mut value as *mut u32 as *mut c_void,
            &mut size,
        )
    };
    status == 0 && value == 0
}

unsafe fn window_class(hwnd: HWND) -> String {
    if hwnd.is_null() { return String::new(); }
    let mut buf = [0u16; 256];
    let len = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

unsafe fn window_title(hwnd: HWND) -> String {
    if hwnd.is_null() { return String::new(); }
    let mut buf = [0u16; 1024];
    let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

unsafe fn window_exe(hwnd: HWND) -> String {
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 { return String::new(); }
    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if process.is_null() { return String::new(); }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len);
    CloseHandle(process);
    if ok == 0 { return String::new(); }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    path.rsplit(['\\', '/']).next().unwrap_or("")
        .trim_end_matches(".exe").to_ascii_lowercase()
}

unsafe fn is_wps_presentation_window(hwnd: HWND) -> bool {
    if hwnd.is_null() { return false; }
    let class = window_class(hwnd);
    let exe = window_exe(hwnd);
    let title = window_title(hwnd);
    sidera_core::wps::presentation_window(&format!("{class}\0{exe}\0"), &title)
}

impl Win32Plat {
    pub fn create() -> Result<Box<Win32Plat>, String> {
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);

            let class_name = wide("SideraOverlayWindow");
            let title = wide(&format!("Sidera {}", sidera_core::app::VERSION));
            let hinstance = GetModuleHandleW(null());

            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: null_mut(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: class_name.as_ptr(),
            };
            if RegisterClassW(&wc) == 0 {
                return Err("RegisterClassW 失败".into());
            }

            // The visual window is always click-through; a separate invisible
            // input window carries mouse/touch for the current input region.
            let visual_ex = WS_EX_LAYERED
                | WS_EX_TRANSPARENT
                | WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE;
            let visual = CreateWindowExW(
                visual_ex,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_POPUP | WS_VISIBLE,
                vx,
                vy,
                vw,
                vh,
                null_mut(),
                null_mut(),
                hinstance,
                null(),
            );
            if visual.is_null() {
                return Err("CreateWindowExW(visual) 失败".into());
            }

            let input_title = wide("Sidera Input");
            let input_ex = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
            let input = CreateWindowExW(
                input_ex,
                class_name.as_ptr(),
                input_title.as_ptr(),
                WS_POPUP | WS_VISIBLE,
                vx,
                vy,
                vw,
                vh,
                null_mut(),
                null_mut(),
                hinstance,
                null(),
            );
            if input.is_null() {
                DestroyWindow(visual);
                return Err("CreateWindowExW(input) 失败".into());
            }
            SetLayeredWindowAttributes(input, 0, 1, LWA_ALPHA);
            let empty_region = CreateRectRgn(0, 0, 0, 0);
            SetWindowRgn(input, empty_region, 1);

            let dpi = GetDpiForWindow(visual).max(96);
            let scale = (dpi as f64 / 96.0).clamp(1.0, 4.0);

            let hdc_screen = GetDC(null_mut());
            let hdc_mem = CreateCompatibleDC(hdc_screen);
            if hdc_screen.is_null() || hdc_mem.is_null() {
                return Err("创建 GDI 设备上下文失败".into());
            }

            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: vw,
                biHeight: -vh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut c_void = null_mut();
            let dib = CreateDIBSection(
                hdc_screen,
                &bmi,
                DIB_RGB_COLORS,
                &mut bits,
                null_mut(),
                0,
            );
            if dib.is_null() || bits.is_null() {
                return Err("CreateDIBSection 失败".into());
            }
            let old_bitmap = SelectObject(hdc_mem, dib);
            std::ptr::write_bytes(bits as *mut u8, 0, (vw * vh * 4) as usize);

            let (tray_icon, tray_icon_owned) = load_tray_icon();
            let plat = Box::new(Win32Plat {
                visual_hwnd: visual,
                input_hwnd: input,
                origin: Cell::new((vx, vy)),
                phys_w: Cell::new(vw),
                phys_h: Cell::new(vh),
                scale: Cell::new(scale),
                events: RefCell::new(VecDeque::new()),
                input_rects: RefCell::new(Vec::new()),
                full_input: Cell::new(false),
                text_input: Cell::new(false),
                previous_focus: Cell::new(null_mut()),
                touch_last: RefCell::new(HashMap::new()),
                hdc_screen,
                hdc_mem,
                dib: Cell::new(dib),
                old_bitmap: Cell::new(old_bitmap),
                bits: Cell::new(bits as *mut u8),
                dirty: Cell::new(true),
                dirty_rect: Cell::new(Some((0, 0, vw, vh))),
                update_failed: Cell::new(false),
                last_topmost: Cell::new(Instant::now()),
                tray_icon,
                tray_icon_owned,
                pending_dpi: Cell::new(None),
                settings: RefCell::new(None),
            });

            let ptr = &*plat as *const Win32Plat as isize;
            SetWindowLongPtrW(visual, GWLP_USERDATA, ptr);
            SetWindowLongPtrW(input, GWLP_USERDATA, ptr);

            RegisterHotKey(
                input,
                HOTKEY_TOGGLE,
                MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                'D' as u32,
            );
            RegisterHotKey(
                input,
                HOTKEY_QUIT,
                MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                'Q' as u32,
            );
            tray_add(input, tray_icon);

            SetWindowPos(
                visual,
                HWND_TOPMOST,
                vx,
                vy,
                vw,
                vh,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            SetWindowPos(
                input,
                HWND_TOPMOST,
                vx,
                vy,
                vw,
                vh,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            ShowWindow(visual, SW_SHOWNOACTIVATE);
            ShowWindow(input, SW_SHOWNOACTIVATE);

            Ok(plat)
        }
    }

    pub fn poll_event(&self) -> Option<WinEvent> {
        self.events.borrow_mut().pop_front()
    }

    pub fn logical_size(&self) -> (i32, i32) {
        (
            (self.phys_w.get() as f64 / self.scale.get()).round().max(1.0) as i32,
            (self.phys_h.get() as f64 / self.scale.get()).round().max(1.0) as i32,
        )
    }

    pub fn canvas_size(&self) -> (u32, u32) {
        (self.phys_w.get() as u32, self.phys_h.get() as u32)
    }

    pub fn scale(&self) -> f64 {
        self.scale.get()
    }

    /// Rebuild the overlay surface after a resolution or DPI change. Only the
    /// single-monitor layout is considered here.
    pub fn refresh_display(&self) -> Option<(i32, i32, f64)> {
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);
            let dpi = self.pending_dpi.take()
                .unwrap_or_else(|| GetDpiForWindow(self.visual_hwnd)).max(96);
            let scale = (dpi as f64 / 96.0).clamp(1.0, 4.0);
            SetWindowPos(self.visual_hwnd, HWND_TOPMOST, vx, vy, vw, vh,
                SWP_NOACTIVATE | SWP_SHOWWINDOW);
            SetWindowPos(self.input_hwnd, HWND_TOPMOST, vx, vy, vw, vh,
                SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let old = self.old_bitmap.get();
            if !old.is_null() {
                SelectObject(self.hdc_mem, old);
                self.old_bitmap.set(null_mut());
            }
            let dib = self.dib.get();
            if !dib.is_null() {
                DeleteObject(dib as HGDIOBJ);
                self.dib.set(null_mut());
                self.bits.set(null_mut());
            }
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: vw,
                biHeight: -vh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut c_void = null_mut();
            let dib = CreateDIBSection(self.hdc_screen, &bmi, DIB_RGB_COLORS,
                &mut bits, null_mut(), 0);
            if dib.is_null() || bits.is_null() {
                return None;
            }
            let old = SelectObject(self.hdc_mem, dib);
            std::ptr::write_bytes(bits as *mut u8, 0, (vw * vh * 4) as usize);
            self.dib.set(dib);
            self.bits.set(bits as *mut u8);
            self.old_bitmap.set(old);
            self.origin.set((vx, vy));
            self.phys_w.set(vw);
            self.phys_h.set(vh);
            self.scale.set(scale);
            self.dirty.set(true);
            self.dirty_rect.set(Some((0, 0, vw, vh)));
            let rects = self.input_rects.borrow().clone();
            let full = self.full_input.get();
            self.set_input_region(&rects, full);
            let logical_w = ((vw as f64) / scale).round().max(1.0) as i32;
            let logical_h = ((vh as f64) / scale).round().max(1.0) as i32;
            Some((logical_w, logical_h, scale))
        }
    }

    pub fn settings_window_open(&self) -> bool {
        self.settings.borrow().is_some()
    }

    pub fn settings_logical_size(&self) -> Option<(i32, i32)> {
        let settings = self.settings.borrow();
        let settings = settings.as_ref()?;
        unsafe {
            let mut client: RECT = std::mem::zeroed();
            if GetClientRect(settings.hwnd, &mut client) == 0 { return None; }
            let width = client.right - client.left;
            let height = client.bottom - client.top;
            if width <= 8 || height <= 8 { return None; }
            let scale = self.scale.get();
            Some((
                ((width as f64) / scale).round().max(1.0) as i32,
                ((height as f64) / scale).round().max(1.0) as i32,
            ))
        }
    }

    /// Create the native settings window (single-monitor layout).
    pub fn create_settings_window(&self) -> Option<(i32, i32)> {
        if self.settings.borrow().is_some() {
            return self.settings_logical_size();
        }
        unsafe {
            let class_name = wide("SideraSettingsWindow");
            let hinstance = GetModuleHandleW(null());
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(settings_wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: self.tray_icon,
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&wc);
            let scale = self.scale.get();
            let screen_w = GetSystemMetrics(SM_CXSCREEN);
            let screen_h = GetSystemMetrics(SM_CYSCREEN);
            let client_w = ((860.0 * scale).round() as i32).min((screen_w - 40).max(320));
            let client_h = ((620.0 * scale).round() as i32).min((screen_h - 72).max(240));
            let mut frame = RECT { left: 0, top: 0, right: client_w, bottom: client_h };
            AdjustWindowRect(&mut frame, WS_OVERLAPPEDWINDOW, 0);
            let win_w = frame.right - frame.left;
            let win_h = frame.bottom - frame.top;
            let x = ((screen_w - win_w) / 2).max(0);
            let y = ((screen_h - win_h) / 2).max(0);
            let title = wide("Sidera 设置");
            let hwnd = CreateWindowExW(0, class_name.as_ptr(), title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE, x, y, win_w, win_h,
                null_mut(), null_mut(), hinstance, null());
            if hwnd.is_null() { return None; }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*self as *const Win32Plat as isize);
            let mut surface = SettingsWindow {
                hwnd, hdc_mem: null_mut(), dib: null_mut(), old_bitmap: null_mut(),
                bits: null_mut(), width: 0, height: 0,
            };
            if self.ensure_settings_surface(&mut surface).is_none() {
                if !surface.old_bitmap.is_null() && !surface.hdc_mem.is_null() {
                    SelectObject(surface.hdc_mem, surface.old_bitmap);
                }
                if !surface.dib.is_null() { DeleteObject(surface.dib as HGDIOBJ); }
                if !surface.hdc_mem.is_null() { DeleteDC(surface.hdc_mem); }
                DestroyWindow(hwnd);
                return None;
            }
            *self.settings.borrow_mut() = Some(surface);
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            SetFocus(hwnd);
            self.settings_logical_size()
        }
    }

    fn ensure_settings_surface(&self, settings: &mut SettingsWindow) -> Option<()> {
        unsafe {
            let mut client: RECT = std::mem::zeroed();
            if GetClientRect(settings.hwnd, &mut client) == 0 { return None; }
            let width = (client.right - client.left).max(1);
            let height = (client.bottom - client.top).max(1);
            if !settings.bits.is_null() && settings.width == width && settings.height == height {
                return Some(());
            }
            if !settings.old_bitmap.is_null() && !settings.hdc_mem.is_null() {
                SelectObject(settings.hdc_mem, settings.old_bitmap);
                settings.old_bitmap = null_mut();
            }
            if !settings.dib.is_null() {
                DeleteObject(settings.dib as HGDIOBJ);
                settings.dib = null_mut();
                settings.bits = null_mut();
            }
            if settings.hdc_mem.is_null() {
                settings.hdc_mem = CreateCompatibleDC(self.hdc_screen);
            }
            if settings.hdc_mem.is_null() { return None; }
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut c_void = null_mut();
            let dib = CreateDIBSection(self.hdc_screen, &bmi, DIB_RGB_COLORS,
                &mut bits, null_mut(), 0);
            if dib.is_null() || bits.is_null() { return None; }
            let old = SelectObject(settings.hdc_mem, dib);
            std::ptr::write_bytes(bits as *mut u8, 0, (width * height * 4) as usize);
            settings.dib = dib;
            settings.old_bitmap = old;
            settings.bits = bits as *mut u8;
            settings.width = width;
            settings.height = height;
            Some(())
        }
    }

    pub fn paint_settings(&self, app: &App, fonts: &Fonts, icon: Option<&Pixmap>) {
        let mut guard = self.settings.borrow_mut();
        let Some(settings) = guard.as_mut() else { return };
        if self.ensure_settings_surface(settings).is_none() { return; }
        let Some(mut pm) = Pixmap::new(settings.width as u32, settings.height as u32) else { return };
        sidera_core::ui::draw_settings_window(app, fonts, icon, &mut pm, self.scale.get() as f32);
        let src = pm.data();
        let dst = unsafe {
            std::slice::from_raw_parts_mut(settings.bits, (settings.width * settings.height * 4) as usize)
        };
        for (target, source) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            target.copy_from_slice(&[source[2], source[1], source[0], 255u8]);
        }
        unsafe {
            let dc = GetDC(settings.hwnd);
            if !dc.is_null() {
                BitBlt(dc, 0, 0, settings.width, settings.height, settings.hdc_mem, 0, 0, SRCCOPY);
                ReleaseDC(settings.hwnd, dc);
            }
        }
    }

    pub fn raise_settings_window(&self) {
        if let Some(settings) = self.settings.borrow().as_ref() {
            unsafe {
                ShowWindow(settings.hwnd, SW_SHOW);
                SetForegroundWindow(settings.hwnd);
                SetFocus(settings.hwnd);
            }
        }
    }

    pub fn close_settings_window(&self) {
        let Some(settings) = self.settings.borrow_mut().take() else { return };
        unsafe {
            SetWindowLongPtrW(settings.hwnd, GWLP_USERDATA, 0);
            if !settings.old_bitmap.is_null() && !settings.hdc_mem.is_null() {
                SelectObject(settings.hdc_mem, settings.old_bitmap);
            }
            if !settings.dib.is_null() {
                DeleteObject(settings.dib as HGDIOBJ);
            }
            if !settings.hdc_mem.is_null() {
                DeleteDC(settings.hdc_mem);
            }
            DestroyWindow(settings.hwnd);
        }
    }

    unsafe fn on_settings_message(&self, hwnd: HWND, msg: u32, wparam: WPARAM,
        lparam: LPARAM) -> Option<LRESULT> {
        let scale = self.scale.get().max(1.0);
        let client = |x: i32, y: i32| {
            ((x as f64 / scale).round() as i32, (y as f64 / scale).round() as i32)
        };
        match msg {
            WM_PAINT => {
                let mut paint: PAINTSTRUCT = std::mem::zeroed();
                let dc = BeginPaint(hwnd, &mut paint);
                if !dc.is_null() {
                    if let Some(settings) = self.settings.borrow().as_ref() {
                        if !settings.bits.is_null() && !settings.hdc_mem.is_null() {
                            BitBlt(dc, 0, 0, settings.width, settings.height,
                                settings.hdc_mem, 0, 0, SRCCOPY);
                        }
                    }
                }
                EndPaint(hwnd, &paint);
                Some(0)
            }
            WM_ERASEBKGND => Some(1),
            WM_SIZE => {
                // SIZE_MINIMIZED behaves like closing: the overlay ignores input
                // while the native settings window exists, so keeping it open
                // while iconified would leave the sidebar unreachable.
                if wparam as u32 == 1 { self.enqueue(WinEvent::SettingsClose); }
                else { self.enqueue(WinEvent::SettingsResized); }
                Some(0)
            }
            WM_CLOSE => { self.enqueue(WinEvent::SettingsClose); Some(0) }
            WM_DESTROY => { self.enqueue(WinEvent::SettingsClose); Some(0) }
            WM_MOUSEMOVE => {
                let (x, y) = client(lo(lparam), hi(lparam));
                self.enqueue(WinEvent::MouseMove(x, y));
                Some(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = client(lo(lparam), hi(lparam));
                SetCapture(hwnd);
                self.enqueue(WinEvent::MouseDown(x, y, 1));
                Some(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = client(lo(lparam), hi(lparam));
                ReleaseCapture();
                self.enqueue(WinEvent::MouseUp(x, y, 1));
                Some(0)
            }
            WM_MOUSEWHEEL => {
                let mut point = POINT { x: lo(lparam), y: hi(lparam) };
                ScreenToClient(hwnd, &mut point);
                let (x, y) = client(point.x, point.y);
                let delta = ((wparam >> 16) as u16 as i16) as f64;
                self.enqueue(WinEvent::Wheel(x, y, -delta / 120.0));
                Some(0)
            }
            WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
                let id = (wparam & 0xffff) as u32;
                if msg == WM_POINTERUP { ReleaseCapture(); }
                let mut info: POINTER_INFO = std::mem::zeroed();
                if GetPointerInfo(id, &mut info) == 0 { return Some(0); }
                let mut point = POINT { x: info.ptPixelLocation.x, y: info.ptPixelLocation.y };
                ScreenToClient(hwnd, &mut point);
                let (x, y) = client(point.x, point.y);
                match msg {
                    WM_POINTERDOWN => {
                        SetCapture(hwnd);
                        self.enqueue(WinEvent::MouseDown(x, y, 1));
                    }
                    WM_POINTERUPDATE => self.enqueue(WinEvent::MouseMove(x, y)),
                    _ => {
                        ReleaseCapture();
                        self.enqueue(WinEvent::MouseUp(x, y, 1));
                    }
                }
                Some(0)
            }
            WM_KEYDOWN => {
                if let Some(key) = key_from_vk(wparam) {
                    self.enqueue(WinEvent::ValueKey(key));
                    Some(0)
                } else { None }
            }
            WM_CHAR => {
                if let Some(ch) = char::from_u32(wparam as u32).filter(|ch| !ch.is_control()) {
                    self.enqueue(WinEvent::ValueKey(
                        sidera_core::value_input::Key::Character(ch)));
                }
                Some(0)
            }
            _ => None,
        }
    }

    pub fn show(&self) {
        unsafe {
            ShowWindow(self.visual_hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(self.input_hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(
                self.visual_hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            SetWindowPos(
                self.input_hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    unsafe fn show_tray_menu(&self, hwnd: HWND) {
        log::info!("tray menu open");
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let items: [(usize, &str); 5] = [
            (1, "设置"),
            (2, "切换画笔和光标"),
            (3, "白板"),
            (4, "显示工具栏"),
            (5, "退出"),
        ];
        for (id, label) in items {
            let text = wide(label);
            AppendMenuW(menu, MF_STRING, id, text.as_ptr());
        }
        let mut point = POINT { x: 0, y: 0 };
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            point.x,
            point.y,
            0,
            hwnd,
            null(),
        );
        DestroyMenu(menu);
        PostMessageW(hwnd, 0, 0, 0);
        let action = match command {
            1 => Some(TrayAction::Settings),
            2 => Some(TrayAction::Toggle),
            3 => Some(TrayAction::Whiteboard),
            4 => Some(TrayAction::Show),
            5 => Some(TrayAction::Quit),
            _ => None,
        };
        if let Some(action) = action {
            self.enqueue(WinEvent::Tray(action));
        }
    }

    pub fn dump_png(&self, path: &str) -> Result<(), String> {
        if self.bits.get().is_null() {
            return Err("DIB 未初始化".into());
        }
        let src = unsafe {
            std::slice::from_raw_parts(self.bits.get(), (self.phys_w.get() * self.phys_h.get() * 4) as usize)
        };
        let mut rgba = Vec::with_capacity(src.len());
        for px in src.chunks_exact(4) {
            let b = px[0] as u32;
            let g = px[1] as u32;
            let r = px[2] as u32;
            let a = px[3] as u32;
            let un = |c: u32| if a == 0 { 0 } else { ((c * 255 + a / 2) / a).min(255) as u8 };
            rgba.extend_from_slice(&[un(r), un(g), un(b), px[3]]);
        }
        let img = image::RgbaImage::from_raw(self.phys_w.get() as u32, self.phys_h.get() as u32, rgba)
            .ok_or("无法创建 DIB PNG 缓冲")?;
        img.save(path).map_err(|e| e.to_string())
    }

    fn enqueue(&self, ev: WinEvent) {
        if let Ok(mut q) = self.events.try_borrow_mut() {
            if q.len() < 4096 {
                q.push_back(ev);
            }
        }
    }

    fn hit_test(&self, x: i32, y: i32) -> bool {
        if self.full_input.get() {
            return true;
        }
        self.input_rects
            .try_borrow()
            .map(|rects| rects.iter().any(|r| {
                x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
            }))
            .unwrap_or(false)
    }

    fn to_logical(&self, x: i32, y: i32) -> (i32, i32) {
        (
            (x as f64 / self.scale.get()).round() as i32,
            (y as f64 / self.scale.get()).round() as i32,
        )
    }

    unsafe fn on_message(
        &self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            WM_KEYDOWN if self.text_input.get() => {
                if let Some(key) = key_from_vk(wparam) {
                    self.enqueue(WinEvent::ValueKey(key));
                }
                Some(0)
            }
            WM_CHAR if self.text_input.get() => {
                if let Some(ch) = char::from_u32(wparam as u32).filter(|ch| !ch.is_control()) {
                    self.enqueue(WinEvent::ValueKey(sidera_core::value_input::Key::Character(ch)));
                }
                Some(0)
            }
            WM_NCHITTEST => {
                let x = lo(lparam) - self.origin.get().0;
                let y = hi(lparam) - self.origin.get().1;
                let (lx, ly) = self.to_logical(x, y);
                Some(if self.hit_test(lx, ly) {
                    HTCLIENT as LRESULT
                } else {
                    HTTRANSPARENT as LRESULT
                })
            }
            WM_MOUSEMOVE => {
                let (x, y) = self.to_logical(lo(lparam), hi(lparam));
                self.enqueue(WinEvent::MouseMove(x, y));
                Some(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = self.to_logical(lo(lparam), hi(lparam));
                SetCapture(hwnd);
                self.enqueue(WinEvent::MouseDown(x, y, 1));
                Some(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = self.to_logical(lo(lparam), hi(lparam));
                ReleaseCapture();
                self.enqueue(WinEvent::MouseUp(x, y, 1));
                Some(0)
            }
            WM_RBUTTONDOWN => {
                let (x, y) = self.to_logical(lo(lparam), hi(lparam));
                SetCapture(hwnd);
                self.enqueue(WinEvent::MouseDown(x, y, 3));
                Some(0)
            }
            WM_RBUTTONUP => {
                let (x, y) = self.to_logical(lo(lparam), hi(lparam));
                ReleaseCapture();
                self.enqueue(WinEvent::MouseUp(x, y, 3));
                Some(0)
            }
            WM_MOUSEWHEEL => {
                let x = lo(lparam) - self.origin.get().0;
                let y = hi(lparam) - self.origin.get().1;
                let (x, y) = self.to_logical(x, y);
                let delta = ((wparam >> 16) as u16 as i16) as f64;
                // WM_MOUSEWHEEL is positive for wheel-up; shared on_scroll uses
                // negative for up (matching X11 Button4).
                self.enqueue(WinEvent::Wheel(x, y, -delta / 120.0));
                Some(0)
            }
            WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
                self.on_pointer(msg, wparam);
                Some(0)
            }
            WM_HOTKEY => {
                match wparam as i32 {
                    HOTKEY_TOGGLE => self.enqueue(WinEvent::ToggleMode),
                    HOTKEY_QUIT => self.enqueue(WinEvent::Quit),
                    HOTKEY_PEN => self.enqueue(WinEvent::PenMode),
                    _ => {}
                }
                Some(0)
            }
            WM_APP_TRAY => {
                let action = (lparam as u32) & 0xffff;
                match action {
                    WM_LBUTTONUP | 0x0400 | 0x0401 => {
                        self.enqueue(WinEvent::Tray(TrayAction::Show));
                    }
                    WM_LBUTTONDBLCLK => {
                        self.enqueue(WinEvent::Tray(TrayAction::Settings));
                    }
                    WM_RBUTTONUP | WM_CONTEXTMENU => self.show_tray_menu(hwnd),
                    _ => {}
                }
                Some(0)
            }
            WM_SETTINGCHANGE => {
                self.enqueue(WinEvent::ThemeChanged);
                Some(0)
            }
            WM_DISPLAYCHANGE => {
                self.pending_dpi.set(None);
                self.enqueue(WinEvent::DisplayChanged);
                Some(0)
            }
            WM_DPICHANGED => {
                self.pending_dpi.set(Some((wparam & 0xffff) as u32));
                self.enqueue(WinEvent::DisplayChanged);
                Some(0)
            }
            WM_ERASEBKGND => Some(1),
            WM_PAINT => {
                ValidateRect(hwnd, null());
                Some(0)
            }
            WM_DESTROY => {
                self.enqueue(WinEvent::Quit);
                PostQuitMessage(0);
                Some(0)
            }
            _ => None,
        }
    }

    unsafe fn on_pointer(&self, msg: u32, wparam: WPARAM) {
        let id = (wparam & 0xffff) as u32;
        let mut info: POINTER_INFO = std::mem::zeroed();
        if GetPointerInfo(id, &mut info) == 0 {
            if msg == WM_POINTERUP {
                if let Some((x, y, d)) = self.touch_last.borrow_mut().remove(&id) {
                    self.enqueue(WinEvent::Touch {
                        id: id as i32,
                        kind: TouchKind::End,
                        x,
                        y,
                        diameter: d,
                    });
                }
            }
            return;
        }
        if msg == WM_POINTERUPDATE && info.pointerFlags & POINTER_FLAG_INCONTACT == 0 {
            return;
        }
        let x = info.ptPixelLocation.x - self.origin.get().0;
        let y = info.ptPixelLocation.y - self.origin.get().1;
        let (lx, ly) = self.to_logical(x, y);

        let mut touch: POINTER_TOUCH_INFO = std::mem::zeroed();
        let diameter = if GetPointerTouchInfo(id, &mut touch) != 0 {
            let w = (touch.rcContact.right - touch.rcContact.left).unsigned_abs() as f64;
            let h = (touch.rcContact.bottom - touch.rcContact.top).unsigned_abs() as f64;
            (w.max(h) / self.scale.get()).clamp(0.0, 4096.0)
        } else {
            0.0
        };

        let kind = match msg {
            WM_POINTERDOWN => TouchKind::Begin,
            WM_POINTERUPDATE => TouchKind::Update,
            _ => TouchKind::End,
        };
        let (x, y, d) = if kind == TouchKind::End {
            self.touch_last
                .borrow_mut()
                .remove(&id)
                .unwrap_or((lx, ly, 0.0))
        } else {
            self.touch_last.borrow_mut().insert(id, (lx, ly, diameter));
            (lx, ly, diameter)
        };
        self.enqueue(WinEvent::Touch { id: id as i32, kind, x, y, diameter: d });
    }
}

impl Drop for Win32Plat {
    fn drop(&mut self) {
        self.close_settings_window();
        unsafe {
            UnregisterHotKey(self.input_hwnd, HOTKEY_TOGGLE);
            UnregisterHotKey(self.input_hwnd, HOTKEY_QUIT);
            UnregisterHotKey(self.input_hwnd, HOTKEY_PEN);
            tray_delete(self.input_hwnd);
            if self.tray_icon_owned && !self.tray_icon.is_null() {
                DestroyIcon(self.tray_icon);
            }
            SetWindowLongPtrW(self.input_hwnd, GWLP_USERDATA, 0);
            SetWindowLongPtrW(self.visual_hwnd, GWLP_USERDATA, 0);
            if !self.hdc_mem.is_null() && !self.old_bitmap.get().is_null() {
                SelectObject(self.hdc_mem, self.old_bitmap.get());
            }
            if !self.dib.get().is_null() {
                DeleteObject(self.dib.get() as HGDIOBJ);
            }
            if !self.hdc_mem.is_null() {
                DeleteDC(self.hdc_mem);
            }
            if !self.hdc_screen.is_null() {
                ReleaseDC(null_mut(), self.hdc_screen);
            }
            DestroyWindow(self.input_hwnd);
            DestroyWindow(self.visual_hwnd);
        }
    }
}

impl Win32Plat {
    /// Own Ctrl+P only while a WPS slideshow is visible.
    pub fn set_pen_hotkey(&self, on: bool) {
        unsafe {
            if on {
                RegisterHotKey(self.input_hwnd, HOTKEY_PEN, MOD_CONTROL | MOD_NOREPEAT, 'P' as u32);
            } else {
                UnregisterHotKey(self.input_hwnd, HOTKEY_PEN);
            }
        }
    }
}

struct DocumentWheel { window: HWND, receiver: HWND, remainder: f64 }

impl sidera_core::document_scroll::ScrollTarget for DocumentWheel {
    fn clear_pending(&mut self) { self.remainder = 0.0; }

    fn scroll_by(&mut self, pixels: f64) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsChild, IsWindow};
        unsafe {
            if GetForegroundWindow() != self.window || IsWindow(self.receiver) == 0
                || (self.receiver != self.window && IsChild(self.window, self.receiver) == 0) { return false; }
            self.remainder -= pixels * (120.0 / 48.0);
            let delta = self.remainder.trunc() as i16;
            if delta == 0 { return true; }
            self.remainder -= f64::from(delta);
            let mut rect = std::mem::zeroed();
            if GetWindowRect(self.receiver, &mut rect) == 0 { return false; }
            let x = rect.left + (rect.right - rect.left) / 2;
            let y = rect.top + (rect.bottom - rect.top) / 2;
            let point = ((y as u16 as u32) << 16) | x as u16 as u32;
            PostMessageW(self.receiver, WM_MOUSEWHEEL, (delta as u16 as usize) << 16, point as isize) != 0
        }
    }
}

impl Backend for Win32Plat {
    fn document_scroll_target(&self) -> Option<Box<dyn sidera_core::document_scroll::ScrollTarget>> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO};
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() || window == self.visual_hwnd || window == self.input_hwnd { return None; }
            let mut info: GUITHREADINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
            let thread = GetWindowThreadProcessId(window, std::ptr::null_mut());
            let receiver = if GetGUIThreadInfo(thread, &mut info) != 0 && !info.hwndFocus.is_null() { info.hwndFocus } else { window };
            Some(Box::new(DocumentWheel { window, receiver, remainder: 0.0 }))
        }
    }
    fn set_text_input(&self, on: bool) {
        if self.text_input.replace(on) == on { return; }
        unsafe {
            let style = GetWindowLongPtrW(self.input_hwnd, GWL_EXSTYLE);
            if on {
                self.previous_focus.set(GetForegroundWindow());
                SetWindowLongPtrW(self.input_hwnd, GWL_EXSTYLE, style & !(WS_EX_NOACTIVATE as isize));
                SetForegroundWindow(self.input_hwnd);
                SetFocus(self.input_hwnd);
            } else {
                SetWindowLongPtrW(self.input_hwnd, GWL_EXSTYLE, style | WS_EX_NOACTIVATE as isize);
                if GetForegroundWindow() == self.input_hwnd && !self.previous_focus.get().is_null() {
                    SetForegroundWindow(self.previous_focus.get());
                }
            }
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn screen_size(&self) -> (i32, i32) {
        self.logical_size()
    }

    fn present(&self, pm: &Pixmap, x: i32, y: i32) {
        let px = (x as f64 * self.scale.get()).round() as i32;
        let py = (y as f64 * self.scale.get()).round() as i32;
        let pw = pm.width() as i32;
        let ph = pm.height() as i32;
        if pw <= 0 || ph <= 0 || self.bits.get().is_null() {
            return;
        }
        let src = pm.data();
        let dst = unsafe {
            std::slice::from_raw_parts_mut(self.bits.get(), (self.phys_w.get() * self.phys_h.get() * 4) as usize)
        };
        let phys_w = self.phys_w.get();
        let phys_h = self.phys_h.get();
        for row in 0..ph {
            let dy = py + row;
            if dy < 0 || dy >= phys_h {
                continue;
            }
            let x0 = px.max(0);
            let x1 = (px + pw).min(phys_w);
            if x1 <= x0 { continue; }
            let src_start = ((row * pw) as usize + (x0 - px) as usize) * 4;
            let src_row = &src[src_start..src_start + (x1 - x0) as usize * 4];
            let dst_start = (dy as usize * phys_w as usize + x0 as usize) * 4;
            let dst_row = &mut dst[dst_start..dst_start + (x1 - x0) as usize * 4];
            for (target, source) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(4)) {
                target.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
            }
        }
        let rect = (px.max(0), py.max(0), (px + pw).min(phys_w), (py + ph).min(phys_h));
        if rect.2 > rect.0 && rect.3 > rect.1 {
            self.dirty_rect.set(Some(match self.dirty_rect.get() {
                Some((x0, y0, x1, y1)) =>
                    (x0.min(rect.0), y0.min(rect.1), x1.max(rect.2), y1.max(rect.3)),
                None => rect,
            }));
        }
        self.dirty.set(true);
    }

    fn set_input_region(&self, rects: &[IRect], full: bool) {
        *self.input_rects.borrow_mut() = rects.to_vec();
        self.full_input.set(full);
        unsafe {
            if full {
                SetWindowRgn(self.input_hwnd, null_mut(), 1);
                return;
            }
            let total = CreateRectRgn(0, 0, 0, 0);
            for r in rects {
                let x1 = (r.x as f64 * self.scale.get()).round() as i32;
                let y1 = (r.y as f64 * self.scale.get()).round() as i32;
                let x2 = ((r.x + r.w) as f64 * self.scale.get()).round() as i32;
                let y2 = ((r.y + r.h) as f64 * self.scale.get()).round() as i32;
                if x2 <= x1 || y2 <= y1 {
                    continue;
                }
                let piece = CreateRectRgn(x1, y1, x2, y2);
                CombineRgn(total, total, piece, RGN_OR);
                DeleteObject(piece as HGDIOBJ);
            }
            if SetWindowRgn(self.input_hwnd, total, 1) == 0 {
                DeleteObject(total as HGDIOBJ);
            }
        }
    }

    fn fake_key(&self, key: Key) {
        let vk = match key {
            Key::Up => VK_UP,
            Key::Down => VK_DOWN,
            Key::Escape => VK_ESCAPE,
        };
        unsafe {
            send_keys(&[(vk, false), (vk, true)]);
        }
    }

    fn start_presentation(&self) -> bool {
        unsafe {
            send_keys(&[
                (VK_SHIFT, false),
                (VK_F5, false),
                (VK_F5, true),
                (VK_SHIFT, true),
            ]);
        }
        true
    }

    fn can_inject_keys_without_prompt(&self) -> bool {
        true
    }

    fn screenshot(&self) -> Option<(Vec<u8>, u32, u32)> {
        unsafe {
            let screen = GetDC(null_mut());
            if screen.is_null() {
                return None;
            }
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, self.phys_w.get(), self.phys_h.get());
            if mem.is_null() || bmp.is_null() {
                if !mem.is_null() {
                    DeleteDC(mem);
                }
                ReleaseDC(null_mut(), screen);
                return None;
            }
            let old = SelectObject(mem, bmp);
            let copied = BitBlt(
                mem,
                0,
                0,
                self.phys_w.get(),
                self.phys_h.get(),
                screen,
                self.origin.get().0,
                self.origin.get().1,
                SRCCOPY | CAPTUREBLT,
            );
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: self.phys_w.get(),
                biHeight: -self.phys_h.get(),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut buf = vec![0u8; (self.phys_w.get() * self.phys_h.get() * 4) as usize];
            let lines = GetDIBits(
                mem,
                bmp,
                0,
                self.phys_h.get() as u32,
                buf.as_mut_ptr() as *mut c_void,
                &mut bmi,
                DIB_RGB_COLORS,
            );
            SelectObject(mem, old);
            DeleteObject(bmp as HGDIOBJ);
            DeleteDC(mem);
            ReleaseDC(null_mut(), screen);
            if copied == 0 || lines == 0 {
                return None;
            }
            for px in buf.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            Some((buf, self.phys_w.get() as u32, self.phys_h.get() as u32))
        }
    }

    fn description(&self) -> &'static str {
        "Windows 10"
    }

    fn is_presentation_fullscreen(&self) -> bool {
        // Single-monitor Win32 path: a WPS presentation window that covers the
        // primary screen is treated as a fullscreen show.
        unsafe {
            let hwnd = GetForegroundWindow();
            if !is_wps_presentation_window(hwnd) { return false; }
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rect) == 0 { return false; }
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            width >= GetSystemMetrics(SM_CXSCREEN) - 8
                && height >= GetSystemMetrics(SM_CYSCREEN) - 8
        }
    }

    fn presentation_foreground(&self) -> Option<bool> {
        Some(unsafe { is_wps_presentation_window(GetForegroundWindow()) })
    }

    fn has_compositor(&self) -> bool {
        true
    }

    fn shape_available(&self) -> bool {
        true
    }

    fn set_pen_hotkey(&self, on: bool) {
        Win32Plat::set_pen_hotkey(self, on);
    }
    fn grab_hotkey(&self) -> bool {
        true
    }

    fn set_scale(&self, _scale: f64) {}

    fn device_pixel_ratio(&self) -> f64 {
        self.scale.get()
    }

    fn flush(&self) {
        unsafe {
            if self.last_topmost.get().elapsed() > Duration::from_millis(500) {
                SetWindowPos(
                    self.visual_hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                SetWindowPos(
                    self.input_hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                self.last_topmost.set(Instant::now());
            }
        }
        if !self.dirty.get() {
            return;
        }
        self.dirty.set(false);
        unsafe {
            let pt_dst = POINT { x: self.origin.get().0, y: self.origin.get().1 };
            let pt_src = POINT { x: 0, y: 0 };
            let size = SIZE { cx: self.phys_w.get(), cy: self.phys_h.get() };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let dirty_range = self.dirty_rect.take()
                .unwrap_or((0, 0, self.phys_w.get(), self.phys_h.get()));
            let dirty = RECT {
                left: dirty_range.0, top: dirty_range.1,
                right: dirty_range.2, bottom: dirty_range.3,
            };
            let info = UPDATELAYEREDWINDOWINFO {
                cbSize: size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
                hdcDst: self.hdc_screen,
                pptDst: &pt_dst,
                psize: &size,
                hdcSrc: self.hdc_mem,
                pptSrc: &pt_src,
                crKey: 0,
                pblend: &blend,
                dwFlags: ULW_ALPHA,
                prcDirty: &dirty,
            };
            let ok = UpdateLayeredWindowIndirect(self.visual_hwnd, &info);
            if ok == 0 {
                self.dirty.set(true);
                self.dirty_rect.set(Some(dirty_range));
                if !self.update_failed.get() {
                    self.update_failed.set(true);
                    sidera_core::diag_log::warn("UpdateLayeredWindow 失败");
                }
            } else {
                self.update_failed.set(false);
            }
        }
    }
}

unsafe fn key_from_vk(wparam: WPARAM) -> Option<sidera_core::value_input::Key> {
    use sidera_core::value_input::Key;
    let altgr = GetKeyState(VK_MENU as i32) < 0;
    let ctrl = GetKeyState(VK_CONTROL as i32) < 0 && !altgr;
    match wparam as u32 {
        27 => Some(Key::Escape), 13 => Some(Key::Enter), 8 => Some(Key::Backspace),
        46 => Some(Key::Delete), 37 => Some(Key::Left), 39 => Some(Key::Right),
        36 => Some(Key::Home), 35 => Some(Key::End),
        65 if ctrl => Some(Key::SelectAll), 86 if ctrl => Some(Key::Paste), _ => None,
    }
}

unsafe fn send_keys(keys: &[(u16, bool)]) {
    let mut inputs: Vec<INPUT> = keys
        .iter()
        .map(|&(vk, up)| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        })
        .collect();
    if !inputs.is_empty() {
        SendInput(
            inputs.len() as u32,
            inputs.as_mut_ptr(),
            size_of::<INPUT>() as i32,
        );
    }
}


unsafe fn load_tray_icon() -> (HICON, bool) {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in ["sidera.ico", "assets/sidera.ico"] {
                let path = dir.join(name);
                if path.is_file() {
                    let text = wide(&path.to_string_lossy());
                    let icon = LoadImageW(
                        null_mut(),
                        text.as_ptr(),
                        IMAGE_ICON,
                        0,
                        0,
                        LR_LOADFROMFILE | LR_DEFAULTSIZE,
                    );
                    if !icon.is_null() {
                        return (icon as HICON, true);
                    }
                }
            }
        }
    }
    (LoadIconW(null_mut(), IDI_APPLICATION), false)
}

unsafe fn tray_add(hwnd: HWND, icon: HICON) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_APP_TRAY;
    nid.hIcon = icon;
    let tip = wide("Sidera");
    for (index, unit) in tip.iter().take(127).enumerate() {
        nid.szTip[index] = *unit;
    }
    Shell_NotifyIconW(NIM_ADD, &nid);
}

unsafe fn tray_delete(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

unsafe extern "system" fn settings_wndproc(hwnd: HWND, msg: u32,
    wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Win32Plat;
    if !ptr.is_null() {
        if let Some(state) = ptr.as_ref() {
            if let Some(result) = state.on_settings_message(hwnd, msg, wparam, lparam) {
                return result;
            }
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Win32Plat;
    if !ptr.is_null() {
        if let Some(state) = ptr.as_ref() {
            if let Some(result) = state.on_message(hwnd, msg, wparam, lparam) {
                return result;
            }
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
