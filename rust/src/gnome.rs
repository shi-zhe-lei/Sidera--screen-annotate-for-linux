//! GNOME Shell native actor backend. Versioned, same-user local IPC; no X11 window.
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, VecDeque};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use crate::instance::Instance as UnixListener;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc::{self, Receiver, SyncSender}};
use std::time::{Duration, Instant};
use serde_json::{json, Value};
use tiny_skia::Pixmap;
use crate::backend::{Backend, IRect, Key};
use crate::desktop::Desktop;
use crate::gesture::TouchKind;
use crate::text::Fonts;

const TILE: u32 = 256;
type Batch = Vec<Vec<u8>>;

fn packet(kind: u8, bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len() + 5);
    output.extend_from_slice(&((bytes.len() + 1) as u32).to_be_bytes());
    output.push(kind); output.extend_from_slice(bytes); output
}
fn control(value: &Value) -> Vec<u8> { packet(1, &serde_json::to_vec(value).unwrap()) }
fn read_control(stream: &mut UnixStream) -> Result<Value, Box<dyn std::error::Error>> {
    let mut header = [0; 4]; stream.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if !(2..=65536).contains(&length) { return Err("GNOME 扩展消息长度无效".into()); }
    let mut data = vec![0; length]; stream.read_exact(&mut data)?;
    if data[0] != 1 { return Err("GNOME 扩展消息类型无效".into()); }
    Ok(serde_json::from_slice(&data[1..])?)
}

fn read_hello(stream: &mut UnixStream) -> Result<Value, Box<dyn std::error::Error>> {
    let hello = read_control(stream).map_err(|error| -> Box<dyn std::error::Error> {
        if let Some(io) = error.downcast_ref::<std::io::Error>() {
            match io.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut =>
                    return "GNOME 原生扩展已连接，但没有完成画布初始化回应。请查看桌面日志中的 Sidera 错误；兼容模式仍可使用。".into(),
                std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset =>
                    return "GNOME 原生扩展在初始化完成前断开了连接。请查看桌面日志中的 Sidera 错误。".into(),
                _ => (),
            }
        }
        format!("读取 GNOME 原生扩展启动回应失败：{error}").into()
    })?;
    if hello["type"] == "error" {
        let detail = hello["message"].as_str().unwrap_or("扩展未提供具体原因");
        return Err(format!("GNOME 原生扩展：{}", detail.chars().take(500).collect::<String>()).into());
    }
    monitors(&hello)?;
    Ok(hello)
}

pub struct Prepared { stream: UnixStream, hello: Value }
pub fn prepare() -> Result<Option<Prepared>, Box<dyn std::error::Error>> {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else { return Ok(None) };
    let directory = std::path::PathBuf::from(runtime).join("sidera-gnome");
    let path = directory.join("bridge.sock");
    if !path.exists() { return Ok(None); }
    let metadata = std::fs::symlink_metadata(&directory)?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err("GNOME 扩展通信目录权限无效".into());
    }
    let mut stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED,
        &mut credentials as *mut _ as *mut libc::c_void, &mut length) } != 0 || credentials.uid != uid {
        return Err("GNOME 扩展通信用户不匹配".into());
    }
    stream.set_read_timeout(Some(Duration::from_secs(4)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let hello = read_hello(&mut stream)?;
    Ok(Some(Prepared { stream, hello }))
}

#[derive(Clone)]
struct Monitor { id: String, index: u32, generation: u32, w: i32, h: i32, scale: f64 }
fn monitors(value: &Value) -> Result<Vec<Monitor>, Box<dyn std::error::Error>> {
    if value["type"] != "hello" || value["protocol"] != 1 { return Err("GNOME 扩展协议版本不匹配".into()); }
    let generation = value["generation"].as_u64().filter(|v| *v <= u32::MAX as u64).ok_or("缺少输出版本")? as u32;
    let items = value["monitors"].as_array().filter(|v| v.len() <= 16).ok_or("输出列表无效")?;
    let mut ids = BTreeSet::new(); let mut indexes = BTreeSet::new();
    items.iter().map(|v| {
        let id = v["id"].as_str().filter(|s| !s.is_empty() && s.len() <= 1024).ok_or("输出名称无效")?.to_owned();
        let index = v["index"].as_u64().filter(|v| *v < 16).ok_or("输出序号无效")? as u32;
        let w = v["width"].as_i64().and_then(|v| i32::try_from(v).ok()).ok_or("输出宽度无效")?;
        let h = v["height"].as_i64().and_then(|v| i32::try_from(v).ok()).ok_or("输出高度无效")?;
        let scale = v["scale"].as_f64().ok_or("输出缩放无效")?;
        if !crate::app::valid_geometry(w, h, scale) || !ids.insert(id.clone()) || !indexes.insert(index) {
            return Err("重复或过大的输出配置".into());
        }
        Ok(Monitor { id, index, generation, w, h, scale })
    }).collect()
}

#[derive(Clone)]
struct Transport { sender: SyncSender<Batch>, broken: Arc<AtomicBool> }
impl Transport {
    fn send(&self, batch: Batch) -> bool {
        match self.sender.try_send(batch) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(_)) => false,
            Err(mpsc::TrySendError::Disconnected(_)) => { self.broken.store(true, Ordering::Relaxed); false }
        }
    }
}
fn transport(mut stream: UnixStream) -> Result<(Transport, Receiver<Value>), Box<dyn std::error::Error>> {
    let mut writer = stream.try_clone()?;
    // A silent or disabled extension must not retain the application's input.
    stream.set_read_timeout(Some(Duration::from_secs(8)))?;
    let (tx, rx) = mpsc::sync_channel::<Batch>(1);
    let (events, incoming) = mpsc::sync_channel::<Value>(512);
    let broken = Arc::new(AtomicBool::new(false));
    let failed = broken.clone();
    std::thread::spawn(move || {
        while let Ok(batch) = rx.recv() {
            if batch.iter().try_for_each(|packet| writer.write_all(packet)).is_err() { break; }
        }
        failed.store(true, Ordering::Relaxed);
        let _ = writer.shutdown(std::net::Shutdown::Both);
    });
    let failed = broken.clone();
    std::thread::spawn(move || {
        while let Ok(value) = read_control(&mut stream) {
            // Bounded queue; do not drop touch/button edges and keep running.
            if events.try_send(value).is_err() { break; }
        }
        failed.store(true, Ordering::Relaxed);
        let _ = stream.shutdown(std::net::Shutdown::Both);
    });
    Ok((Transport { sender: tx, broken }, incoming))
}

struct GnomePlat {
    monitor: RefCell<Monitor>, transport: Transport,
    back: RefCell<Pixmap>, dirty: RefCell<BTreeSet<(u32, u32)>>,
    input: RefCell<(Vec<IRect>, bool)>, input_dirty: Cell<bool>,
    commands: RefCell<VecDeque<Value>>, fullscreen: Cell<bool>,
    wps_presentation: Cell<bool>,
    keyboard: bool, hotkey: bool,
}
impl GnomePlat {
    fn new(monitor: Monitor, transport: Transport, capabilities: &Value) -> Self {
        Self { monitor: RefCell::new(monitor), transport,
            back: RefCell::new(Pixmap::new(1, 1).unwrap()), dirty: RefCell::new(BTreeSet::new()),
            input: RefCell::new((Vec::new(), false)), input_dirty: Cell::new(true),
            commands: RefCell::new(VecDeque::new()), fullscreen: Cell::new(false),
            wps_presentation: Cell::new(false),
            keyboard: capabilities["keyboard"] == true, hotkey: capabilities["hotkey"] == true }
    }
    fn configure(&self, monitor: Monitor) {
        let w = (monitor.w as f64 * monitor.scale).round() as u32;
        let h = (monitor.h as f64 * monitor.scale).round() as u32;
        let mut back = self.back.borrow_mut();
        if (back.width(), back.height()) != (w, h) { *back = Pixmap::new(w, h).unwrap(); }
        *self.monitor.borrow_mut() = monitor;
        self.dirty.borrow_mut().clear();
        for y in (0..h).step_by(TILE as usize) { for x in (0..w).step_by(TILE as usize) {
            self.dirty.borrow_mut().insert((x, y));
        } }
        self.input_dirty.set(true);
        self.commands.borrow_mut().clear();
        self.wps_presentation.set(false);
    }
    fn command(&self, mut value: Value) {
        let m = self.monitor.borrow(); value["generation"] = json!(m.generation); value["index"] = json!(m.index);
        let mut queue = self.commands.borrow_mut();
        if queue.len() < 8 { queue.push_back(value); }
    }
}
impl Backend for GnomePlat {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn description(&self) -> &'static str { "GNOME 原生 Wayland" }
    fn screen_size(&self) -> (i32, i32) { let m = self.monitor.borrow(); (m.w, m.h) }
    fn has_compositor(&self) -> bool { true }
    fn grab_hotkey(&self) -> bool { self.hotkey }
    fn is_presentation_fullscreen(&self) -> bool { self.fullscreen.get() }
    fn presentation_foreground(&self) -> Option<bool> { Some(self.wps_presentation.get()) }
    fn can_inject_keys_without_prompt(&self) -> bool { self.keyboard }
    fn screenshot(&self) -> Option<(Vec<u8>, u32, u32)> { None }
    fn request_screenshot(&self) -> bool { self.command(json!({"type":"screenshot"})); true }
    fn fake_key(&self, key: Key) {
        if self.keyboard {
            self.command(json!({"type":"key", "key":format!("{key:?}"),
                "presentationOnly":self.wps_presentation.get()}));
        }
    }
    fn start_presentation(&self) -> bool {
        if !self.keyboard || !self.wps_presentation.get() { return false; }
        self.command(json!({"type":"key", "key":"StartShow"}));
        true
    }
    fn virtual_right_click(&self, x: i32, y: i32) { self.command(json!({"type":"rightClick", "x":x, "y":y})); }
    fn can_forward_right_click(&self) -> bool { true }
    fn set_text_input(&self, on: bool) { self.command(json!({"type":"text_input", "enabled":on})); }
    fn set_input_region(&self, rects: &[IRect], full: bool) {
        let old = self.input.borrow();
        let equal = old.1 == full && old.0.len() == rects.len() && old.0.iter().zip(rects).all(|(a,b)|
            (a.x,a.y,a.w,a.h) == (b.x,b.y,b.w,b.h));
        drop(old);
        if !equal { *self.input.borrow_mut() = (rects.to_vec(), full); self.input_dirty.set(true); }
    }
    fn present(&self, pm: &Pixmap, x: i32, y: i32) {
        let scale = self.monitor.borrow().scale;
        let x = (x as f64 * scale).round() as i32;
        let y = (y as f64 * scale).round() as i32;
        let mut back = self.back.borrow_mut();
        let (bw,bh) = (back.width() as i32, back.height() as i32);
        let (left,top) = (x.max(0),y.max(0));
        let (right,bottom) = ((x+pm.width() as i32).min(bw), (y+pm.height() as i32).min(bh));
        if left >= right || top >= bottom { return; }
        for row in top..bottom {
            let dst = ((row*bw+left)*4) as usize;
            let src = (((row-y)*pm.width() as i32+left-x)*4) as usize;
            let len = ((right-left)*4) as usize;
            back.data_mut()[dst..dst+len].copy_from_slice(&pm.data()[src..src+len]);
        }
        let mut dirty = self.dirty.borrow_mut();
        for ty in (top as u32 / TILE)..=((bottom-1) as u32 / TILE) {
            for tx in (left as u32 / TILE)..=((right-1) as u32 / TILE) { dirty.insert((tx*TILE,ty*TILE)); }
        }
    }
    fn flush(&self) {
        let dirty = self.dirty.borrow();
        if dirty.is_empty() && !self.input_dirty.get() && self.commands.borrow().is_empty() { return; }
        let m = self.monitor.borrow();
        let mut batch = Vec::new();
        if !dirty.is_empty() {
            let back = self.back.borrow(); let (w,h) = (back.width(),back.height());
            batch.push(control(&json!({"type":"begin","generation":m.generation,"index":m.index,"width":w,"height":h})));
            for &(x,y) in dirty.iter() {
                let (tw,th) = (TILE.min(w-x),TILE.min(h-y));
                let mut bytes = Vec::with_capacity((tw*th*4+32) as usize);
                for value in [m.generation,m.index,w,h,x,y,tw,th] { bytes.extend_from_slice(&value.to_be_bytes()); }
                for row in y..y+th {
                    let start = ((row*w+x)*4) as usize;
                    bytes.extend_from_slice(&back.data()[start..start+(tw*4) as usize]);
                }
                batch.push(packet(2,&bytes));
            }
            batch.push(control(&json!({"type":"end","generation":m.generation,"index":m.index})));
        }
        if self.input_dirty.get() {
            let input = self.input.borrow();
            let rects: Vec<_> = input.0.iter().map(|r| [r.x,r.y,r.w,r.h]).collect();
            batch.push(control(&json!({"type":"input","generation":m.generation,"index":m.index,"full":input.1,"rects":rects})));
        }
        for command in self.commands.borrow().iter() { batch.push(control(command)); }
        drop(dirty);
        if self.transport.send(batch) {
            self.dirty.borrow_mut().clear(); self.input_dirty.set(false); self.commands.borrow_mut().clear();
        }
    }
}

fn configure(desktop: &mut Desktop, transport: &Transport, value: &Value) -> Result<u32, Box<dyn std::error::Error>> {
    let monitors = monitors(value)?;
    let ids: BTreeSet<_> = monitors.iter().map(|m| m.id.clone()).collect();
    for id in desktop.outputs.keys().filter(|id| !ids.contains(*id)).cloned().collect::<Vec<_>>() { desktop.remove(&id); }
    for m in monitors {
        if !desktop.outputs.contains_key(&m.id) {
            desktop.add(m.id.clone(), Box::new(GnomePlat::new(m.clone(),transport.clone(),&value["capabilities"])));
        }
        desktop.cancel(&m.id);
        let output = &desktop.outputs[&m.id];
        output.rt.backend.as_any().downcast_ref::<GnomePlat>().unwrap().configure(m.clone());
        desktop.configure(&m.id,m.w,m.h,m.scale);
    }
    Ok(value["generation"].as_u64().unwrap() as u32)
}

pub fn run(prepared: Prepared, fonts: Fonts, icon: Option<Pixmap>, icon_big: Option<Pixmap>, single: Option<UnixListener>)
    -> Result<(), Box<dyn std::error::Error>>
{
    let (transport, incoming) = transport(prepared.stream)?;
    let mut desktop = Desktop::new(fonts,icon,icon_big,single);
    let mut generation = configure(&mut desktop,&transport,&prepared.hello)?;
    let mut ping = Instant::now() - Duration::from_secs(1);
    while !transport.broken.load(Ordering::Relaxed) {
        let frame_started = Instant::now();
        for value in incoming.try_iter().take(256) {
            let kind = value["type"].as_str().unwrap_or_default();
            match kind {
                "hello" => { generation = configure(&mut desktop,&transport,&value)?; continue; }
                "toggle" => { desktop.toggle(); continue; }
                "screenshot" => { log::info!("截图已保存并复制：{}", value["path"]); continue; }
                "error" => { log::warn!("GNOME 扩展：{}",value["message"]); continue; }
                _ => (),
            }
            if value["generation"].as_u64() != Some(generation as u64) { continue; }
            if kind == "state" {
                for output in desktop.outputs.values() {
                    let backend = output.rt.backend.as_any().downcast_ref::<GnomePlat>().unwrap();
                    backend.fullscreen.set(value["fullscreen"].as_i64() == Some(backend.monitor.borrow().index as i64));
                    backend.wps_presentation.set(value["wpsPresentation"] == true);
                }
                continue;
            }
            let Some(id) = value["id"].as_str() else { continue };
            let (x,y) = (value["x"].as_i64().unwrap_or(0).clamp(-32768,32768) as i32,
                value["y"].as_i64().unwrap_or(0).clamp(-32768,32768) as i32);
            match kind {
                "motion" => desktop.motion(id,x,y),
                "value_key" => {
                    use crate::value_input::Key;
                    let key = match value["key"].as_str().unwrap_or_default() {
                        "enter" => Some(Key::Enter), "escape" => Some(Key::Escape),
                        "backspace" => Some(Key::Backspace), "delete" => Some(Key::Delete),
                        "left" => Some(Key::Left), "right" => Some(Key::Right),
                        "home" => Some(Key::Home), "end" => Some(Key::End),
                        "select_all" => Some(Key::SelectAll), "paste" => Some(Key::Paste),
                        _ => value["text"].as_str().and_then(|s| s.chars().next()).map(Key::Character),
                    };
                    if let Some(key) = key { desktop.value_key(id, key); }
                }
                "scroll" => if let Some(delta) = value["delta"].as_f64().filter(|d| d.is_finite()) {
                    desktop.scroll(id,x,y,delta.clamp(-8.0,8.0));
                },
                "press" => if let Some(button @ (1|3)) = value["button"].as_u64() { desktop.press(id,x,y,button as u8); },
                "release" => if let Some(button @ (1|3)) = value["button"].as_u64() { desktop.release(id,x,y,button as u8); },
                "leave" => desktop.leave(id),
                "cancel" => desktop.cancel(id),
                "touch" => {
                    let Some(touch) = value["touch"].as_i64().and_then(|n|i32::try_from(n).ok()) else { continue };
                    match value["kind"].as_str() {
                        Some("cancel") => desktop.cancel(id),
                        Some("begin") => desktop.touch(id,touch,x,y,TouchKind::Begin,0.0),
                        Some("update") => desktop.touch(id,touch,x,y,TouchKind::Update,0.0),
                        Some("end") => desktop.touch(id,touch,x,y,TouchKind::End,0.0),
                        _ => (),
                    }
                }
                _ => (),
            }
        }
        if ping.elapsed() >= Duration::from_secs(1) && transport.send(vec![control(&json!({"type":"ping"}))]) { ping = Instant::now(); }
        desktop.tick();
        let interval = Duration::from_millis(16);
        let delay = if desktop.settings_animating() {
            interval.saturating_sub(frame_started.elapsed())
        } else {
            interval
        };
        std::thread::sleep(delay);
    }
    // Shell has already removed the actors; a stale client must not relaunch them.
    log::info!("GNOME 原生扩展已断开，Sidera 退出");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_reports_extension_error_instead_of_protocol_mismatch() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        server.write_all(&control(&json!({"type":"error","fatal":true,"message":"Unrecognized parameter affectsInputRegion"}))).unwrap();
        let error = read_hello(&mut client).unwrap_err().to_string();
        assert!(error.contains("affectsInputRegion"));
        assert!(!error.contains("协议版本不匹配"));
    }
    #[test]
    fn startup_names_missing_reply_and_early_disconnect() {
        let (mut client, server) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        let error = read_hello(&mut client).unwrap_err().to_string();
        assert!(error.contains("没有完成画布初始化回应"));
        assert!(!error.contains("os error 11"));
        drop(server);
        assert!(read_hello(&mut client).unwrap_err().to_string().contains("断开"));
    }
    #[test]
    fn rejects_duplicate_and_unbounded_monitors() {
        let monitor = json!({"id":"DP-1", "index":0,"width":1920,"height":1080,"scale":1.25});
        let mut hello = json!({"type":"hello","protocol":1,"generation":1,"monitors":[monitor]});
        assert!(monitors(&hello).is_ok());
        let duplicate = hello["monitors"][0].clone();
        hello["monitors"].as_array_mut().unwrap().push(duplicate);
        assert!(monitors(&hello).is_err());
        hello["monitors"] = json!([{"id":"bad","index":0,"width":2147483647,"height":2147483647,"scale":4.0}]);
        assert!(monitors(&hello).is_err());
    }
}
