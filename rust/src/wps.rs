// ============================================================
// Sidera - WPS 联动桥（本地 HTTP 127.0.0.1:16666 + 加载项注册 + 心跳）
// 平台无关；在独立线程运行 TCP 服务，主线程轮询共享状态
// ============================================================
use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::app::now_ms;
use crate::bridge_ws;
use sha1::{Digest, Sha1};

#[derive(Clone, Debug)]
pub enum WpsEvent {
    SlideshowBegin(i32),
    SlideshowEnd,
    RealPos(i32),
}

pub struct WpsShared {
    pub connected: bool,
    pub last_seen: i64,
    pub real_pos: i32,
    pub page_index: i32,
    pub page_total: i32,
    pub can_jump: bool,
    pub deck_epoch: i32,
    pub can_start: bool,
    pub start_pending_until: i64,
    pub whiteboard: bool,
    pub test_mode: bool,
    pub sse_clients: usize,
    pub ws_clients: usize,
    pub negative_since: i64,
    pub queue: VecDeque<(i64, String)>,
    pub command_failed_until: i64,
    pub events: Vec<WpsEvent>,
    sequences: BTreeMap<String, u64>,
}

impl Default for WpsShared {
    fn default() -> Self {
        WpsShared {
            connected: false,
            last_seen: 0,
            real_pos: -1,
            page_index: -1,
            page_total: -1,
            can_jump: false,
            deck_epoch: -1,
            can_start: false,
            start_pending_until: 0,
            whiteboard: false,
            test_mode: true,
            sse_clients: 0,
            ws_clients: 0,
            negative_since: 0,
            queue: VecDeque::new(),
            command_failed_until: 0,
            events: Vec::new(),
            sequences: BTreeMap::new(),
        }
    }
}

pub struct WpsBridge {
    pub shared: Arc<Mutex<WpsShared>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WpsBridge {
    fn drop(&mut self) {
        // 通知服务线程退出并等待其结束，确保 16666 端口被释放，
        // 否则关闭 WPS 调试后再打开会因端口被旧线程占用而无法重新绑定。
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub fn log_file() -> std::path::PathBuf {
    // A missing home disables the log rather than sharing a writable /tmp file.
    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/nonexistent")).join("wps-api-debug.log")
}

pub fn wps_log(msg: &str) {
    log::info!("[WPSAPI] {}", msg);
    if cfg!(test) { return; }
    let path = log_file();
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    if let Ok(mut f) = options.open(&path) {
        if !f.metadata().map(|m| m.is_file()).unwrap_or(false) { return; }
        if f.metadata().map(|m| m.len() > 1024 * 1024).unwrap_or(false) { let _ = f.set_len(0); }
        let _ = writeln!(f, "{} {}", now_ms(), msg);
    }
}

fn addin_dir() -> Option<std::path::PathBuf> {
    let mut cands: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(env) = std::env::var("WPS_ADDIN_DIR") {
        cands.push(std::path::PathBuf::from(env));
    }
    if let Ok(cwd) = std::env::current_dir() {
        cands.push(cwd.join("wps-addin"));
        cands.push(cwd.join("../wps-addin"));
        cands.push(cwd.join("../../wps-addin"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("wps-addin"));
            cands.push(dir.join("../wps-addin"));
            cands.push(dir.join("../../wps-addin"));
            cands.push(dir.join("../../../wps-addin"));
            cands.push(dir.join("../../../../wps-addin"));
            cands.push(dir.join("../../../../../wps-addin"));
        }
    }
    cands.push(std::path::PathBuf::from("/usr/share/sidera/wps-addin"));
    cands
        .into_iter()
        .find(|p| p.join("manifest.xml").exists())
}

const ADDIN_SCHEMA: &str = "sidera-wps-addin-registration-v2";
#[cfg(unix)]
const ADDIN_FILES: [&str; 8] = [
    "publish.xml",
    "manifest.xml",
    "main.js",
    "js/bridge.js",
    "ribbon.xml",
    "index.html",
    "install.sh",
    "cleanup-old.sh",
];
#[cfg(windows)]
const ADDIN_FILES: [&str; 6] = [
    "publish.xml",
    "manifest.xml",
    "main.js",
    "js/bridge.js",
    "ribbon.xml",
    "index.html",
];

fn addin_fingerprint(dir: &std::path::Path) -> Option<String> {
    let mut digest = Sha1::new();
    digest.update(ADDIN_SCHEMA.as_bytes());
    digest.update(crate::app::VERSION.as_bytes());
    for name in ADDIN_FILES {
        let bytes = std::fs::read(dir.join(name)).ok()?;
        digest.update(name.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    Some(format!("{:x}", digest.finalize()))
}

fn registration_state_path() -> Option<std::path::PathBuf> {
    dirs::state_dir().map(|path| path.join("sidera/wps-addin-state"))
}

#[cfg(unix)]
fn wps_publish_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|home| home.join(".local/share/Kingsoft/wps/jsaddons/publish.xml"))
}

#[cfg(windows)]
fn wps_publish_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|path| path.join("kingsoft/wps/jsaddons/publish.xml"))
}

fn registration_present() -> bool {
    let Some(path) = wps_publish_path() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.contains("name=\"sidera-bridge\"") || text.contains("name='sidera-bridge'")
}

fn mark_registered(fingerprint: &str) {
    let Some(path) = registration_state_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = crate::persistence::atomic_write(&path, fingerprint.as_bytes());
}

/// Register only after install/upgrade, or when the WPS entry is missing.
/// Never rewrite WPS-owned files merely because WPS changed its own enable flag.
pub fn ensure_addin_registered_if_needed() {
    let Some(dir) = addin_dir() else {
        wps_log("警告: 未找到 wps-addin 目录（请设置 WPS_ADDIN_DIR）");
        return;
    };
    let Some(fingerprint) = addin_fingerprint(&dir) else {
        wps_log("警告: 无法读取 WPS 加载项文件，跳过自动登记");
        return;
    };
    let recorded = registration_state_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|value| value.trim().to_string());
    if recorded.as_deref() == Some(fingerprint.as_str()) && registration_present() {
        return;
    }
    crate::diag_log::info("WPS 加载项首次安装/升级，执行一次登记");
    match register_addin() {
        Ok(text) => {
            mark_registered(&fingerprint);
            wps_log(&text);
            crate::diag_log::info(&format!("WPS 加载项登记成功：{text}"));
        }
        Err(text) => {
            wps_log(&text);
            crate::diag_log::error(&text);
        }
    }
}

#[cfg(unix)]
pub fn register_addin() -> Result<String, String> {
    let dir = addin_dir().ok_or_else(|| "未找到 wps-addin 目录".to_string())?;
    let output = run_script(&dir.join("install.sh"), &["--repair"], Duration::from_secs(5))
        .map_err(|error| format!("无法运行加载项登记：{error}"))?;
    if output.status.success() {
        if let Some(fingerprint) = addin_fingerprint(&dir) {
            mark_registered(&fingerprint);
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!("加载项登记失败：{}", String::from_utf8_lossy(&output.stderr).trim()))
    }
}

#[cfg(windows)]
pub fn register_addin() -> Result<String, String> {
    let dir = addin_dir().ok_or_else(|| "未找到 wps-addin 目录".to_string())?;
    let template = std::fs::read(dir.join("publish.xml"))
        .map_err(|error| format!("读取 WPS 登记模板失败：{error}"))?;
    let expected = publish_entry_attributes(&template)?;
    let path = wps_publish_path().ok_or_else(|| "无法确定 WPS 登记目录".to_string())?;
    let message = rewrite_publish(&path, &expected, false)?;
    if let Some(fingerprint) = addin_fingerprint(&dir) {
        mark_registered(&fingerprint);
    }
    Ok(message)
}

#[cfg(unix)]
pub fn remove_addin_registration() -> Result<String, String> {
    let dir = addin_dir().ok_or_else(|| "未找到 wps-addin 目录".to_string())?;
    let output = run_script(&dir.join("cleanup-old.sh"), &["--remove-current"], Duration::from_secs(5))
        .map_err(|error| format!("无法运行加载项清除：{error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!("加载项清除失败：{}", String::from_utf8_lossy(&output.stderr).trim()))
    }
}

#[cfg(windows)]
pub fn remove_addin_registration() -> Result<String, String> {
    let path = wps_publish_path().ok_or_else(|| "无法确定 WPS 登记目录".to_string())?;
    rewrite_publish(&path, &[], true)
}

#[cfg(unix)]
fn run_script(script: &std::path::Path, args: &[&str], timeout: Duration) -> std::io::Result<std::process::Output> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut child = Command::new("bash").arg(script).args(args).process_group(0)
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output(),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            result => {
                // The installer may be waiting on its XML lock. End its private
                // process group, including Python, rather than leave an orphan.
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL); }
                let _ = child.wait();
                return Err(result.err().unwrap_or_else(|| std::io::Error::new(
                    std::io::ErrorKind::TimedOut, "加载项登记超过等待时限")));
            }
        }
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ManagedEntry {
    Current,
    Legacy,
    OldBridge,
}

#[cfg(windows)]
fn element_attributes(
    element: &quick_xml::events::BytesStart<'_>,
) -> Result<Vec<(String, String)>, String> {
    let mut attrs = Vec::new();
    for attr in element.attributes().with_checks(false) {
        let attr = attr.map_err(|error| format!("解析加载项属性失败：{error}"))?;
        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|error| format!("解析加载项属性失败：{error}"))?
            .to_string();
        attrs.push((key, value));
    }
    Ok(attrs)
}

#[cfg(windows)]
fn publish_entry_attributes(template: &[u8]) -> Result<Vec<(String, String)>, String> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let text = std::str::from_utf8(template)
        .map_err(|error| format!("加载项模板不是 UTF-8：{error}"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader::from_str(text);
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if element.name().as_ref() == b"jspluginonline" =>
            {
                let attrs = element_attributes(&element)?;
                if attrs.iter().any(|(key, value)| key == "name" && value == "sidera-bridge") {
                    return Ok(attrs);
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(format!("解析加载项模板失败：{error}")),
            _ => {}
        }
    }
    Err("安装模板缺少 sidera-bridge 登记".into())
}

#[cfg(windows)]
fn managed_entry(attrs: &[(String, String)]) -> Option<ManagedEntry> {
    let value = |key: &str| {
        attrs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
            .unwrap_or("")
    };
    let name = value("name");
    if name == "sidera-bridge" {
        return Some(ManagedEntry::Current);
    }
    if name == "screen-annotate-bridge" {
        return Some(ManagedEntry::Legacy);
    }
    if value("url")
        .trim_end_matches('/')
        .eq_ignore_ascii_case("http://127.0.0.1:16666")
    {
        return Some(ManagedEntry::OldBridge);
    }
    None
}

#[cfg(windows)]
fn merged_attributes(
    existing: &[(String, String)],
    expected: &[(String, String)],
) -> Vec<(String, String)> {
    let mut out = existing.to_vec();
    for (key, value) in expected {
        if let Some(slot) = out.iter_mut().find(|(existing_key, _)| existing_key == key) {
            slot.1 = value.clone();
        } else {
            out.push((key.clone(), value.clone()));
        }
    }
    out
}

#[cfg(windows)]
fn entry_start(attrs: &[(String, String)]) -> quick_xml::events::BytesStart<'static> {
    let mut element = quick_xml::events::BytesStart::new("jspluginonline");
    for (key, value) in attrs {
        let escaped = value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        element.push_attribute((key.as_str(), escaped.as_str()));
    }
    element
}

#[cfg(windows)]
fn rewrite_publish(
    path: &std::path::Path,
    expected: &[(String, String)],
    remove_current: bool,
) -> Result<String, String> {
    use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};
    use quick_xml::reader::Reader;
    use quick_xml::writer::Writer;

    let original = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("读取 WPS 登记失败：{error}")),
    };
    if original.is_none() && remove_current {
        return Ok("未找到 WPS 加载项登记".into());
    }

    let output = if let Some(bytes) = &original {
        let text = std::str::from_utf8(bytes)
            .map_err(|error| format!("WPS 登记不是 UTF-8：{error}"))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut reader = Reader::from_str(text);
        let mut writer = Writer::new(Vec::new());
        let mut skip_depth = 0usize;
        let mut root_seen = false;
        let mut in_root = false;
        let mut inserted = false;
        let mut changed = false;
        loop {
            let event = reader
                .read_event()
                .map_err(|error| format!("解析 WPS 登记失败：{error}"))?;
            match event {
                Event::Eof => break,
                Event::Start(element) => {
                    if skip_depth > 0 {
                        skip_depth += 1;
                        continue;
                    }
                    let is_online = element.name().as_ref() == b"jspluginonline";
                    let is_root = element.name().as_ref() == b"jsplugins";
                    if is_online {
                        let attrs = element_attributes(&element)?;
                        if let Some(kind) = managed_entry(&attrs) {
                            changed = true;
                            if kind == ManagedEntry::Current && !remove_current && !inserted {
                                let merged = merged_attributes(&attrs, expected);
                                writer
                                    .write_event(Event::Empty(entry_start(&merged)))
                                    .map_err(|error| error.to_string())?;
                                inserted = true;
                            }
                            skip_depth = 1;
                            continue;
                        }
                    } else if is_root && !root_seen {
                        root_seen = true;
                        in_root = true;
                    }
                    writer
                        .write_event(Event::Start(element))
                        .map_err(|error| error.to_string())?;
                }
                Event::Empty(element) => {
                    if skip_depth > 0 {
                        continue;
                    }
                    if element.name().as_ref() == b"jspluginonline" {
                        let attrs = element_attributes(&element)?;
                        if let Some(kind) = managed_entry(&attrs) {
                            changed = true;
                            if kind == ManagedEntry::Current && !remove_current && !inserted {
                                let merged = merged_attributes(&attrs, expected);
                                writer
                                    .write_event(Event::Empty(entry_start(&merged)))
                                    .map_err(|error| error.to_string())?;
                                inserted = true;
                            }
                            continue;
                        }
                    }
                    writer
                        .write_event(Event::Empty(element))
                        .map_err(|error| error.to_string())?;
                }
                Event::End(element) => {
                    if skip_depth > 0 {
                        skip_depth -= 1;
                        continue;
                    }
                    if element.name().as_ref() == b"jsplugins" && in_root {
                        if !remove_current && !inserted {
                            writer
                                .write_event(Event::Empty(entry_start(expected)))
                                .map_err(|error| error.to_string())?;
                            inserted = true;
                            changed = true;
                        }
                        in_root = false;
                    }
                    writer
                        .write_event(Event::End(element))
                        .map_err(|error| error.to_string())?;
                }
                other => {
                    writer
                        .write_event(other)
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        if !root_seen {
            return Err("WPS 登记文件缺少 jsplugins 根节点".into());
        }
        if in_root {
            return Err("WPS 登记文件根节点未闭合".into());
        }
        if !changed {
            return Ok(if remove_current {
                "没有需要清理的 WPS 登记".into()
            } else {
                "登记已是最新".into()
            });
        }
        writer.into_inner()
    } else {
        let mut writer = Writer::new(Vec::new());
        writer
            .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
            .map_err(|error| error.to_string())?;
        writer
            .write_event(Event::Start(BytesStart::new("jsplugins")))
            .map_err(|error| error.to_string())?;
        writer
            .write_event(Event::Empty(entry_start(expected)))
            .map_err(|error| error.to_string())?;
        writer
            .write_event(Event::End(BytesEnd::new("jsplugins")))
            .map_err(|error| error.to_string())?;
        writer.into_inner()
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建 WPS 登记目录失败：{error}"))?;
    }
    if let Some(original) = &original {
        if original == &output {
            return Ok("登记已是最新".into());
        }
        let backup = path.with_file_name(format!("publish.xml.sidera-backup-{}", now_ms()));
        std::fs::copy(path, &backup)
            .map_err(|error| format!("备份 WPS 登记失败：{error}"))?;
    }
    crate::persistence::atomic_write(path, &output)
        .map_err(|error| format!("写入 WPS 登记失败：{error}"))?;
    Ok(if remove_current {
        "已清理本项目 WPS 登记".into()
    } else {
        format!("已更新本项目登记：{}", path.display())
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hi = (b[i + 1] as char).to_digit(16);
            let lo = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        if b[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn query_m(query: &str) -> Option<String> {
    for kv in query.split('&') {
        if let Some(v) = kv.strip_prefix("m=") {
            return Some(percent_decode(v));
        }
    }
    None
}

fn accept_sequence(shared: &Arc<Mutex<WpsShared>>, query: &str) -> Result<bool, ()> {
    let field = |key: &str| query.split('&').find_map(|word| word.strip_prefix(key));
    let (sid, sequence) = (field("sid="), field("seq="));
    if sid.is_none() && sequence.is_none() { return Ok(true); } // Older add-ins.
    let sid = sid.filter(|id| !id.is_empty() && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).ok_or(())?;
    let sequence = sequence.and_then(|s| s.parse::<u64>().ok()).filter(|s| *s > 0).ok_or(())?;
    let mut state = shared.lock().unwrap();
    if state.sequences.get(sid).is_some_and(|previous| sequence <= *previous) { return Ok(false); }
    if !state.sequences.contains_key(sid) && state.sequences.len() >= 16 { state.sequences.pop_first(); }
    state.sequences.insert(sid.into(), sequence);
    Ok(true)
}

fn reply(stream: &mut TcpStream, body: &[u8]) {
    crate::bridge_http::reply(stream, 200, "text/plain; charset=utf-8", body);
}

fn reply_file(stream: &mut TcpStream, path: &str) {
    let Some(dir) = addin_dir() else {
        crate::bridge_http::reply(stream, 404, "text/plain", b"Not found");
        return;
    };
    let Some(rel) = crate::bridge_http::static_path(path) else {
        crate::bridge_http::reply(stream, 404, "text/plain", b"Not found");
        return;
    };
    let fp = dir.join(rel);
    if !fp.canonicalize().ok().zip(dir.canonicalize().ok()).is_some_and(|(file, root)| file.starts_with(root))
        || std::fs::metadata(&fp).map(|m| !m.is_file() || m.len() > 1024 * 1024).unwrap_or(true) {
        crate::bridge_http::reply(stream, 404, "text/plain", b"Not found"); return;
    }
    let Ok(body) = std::fs::read(&fp) else {
        crate::bridge_http::reply(stream, 404, "text/plain", b"Not found");
        return;
    };
    let ct = if rel.ends_with(".xml") {
        "application/xml"
    } else if rel.ends_with(".js") {
        "text/javascript"
    } else if rel.ends_with(".html") || rel.ends_with(".htm") {
        "text/html"
    } else if rel.ends_with(".svg") {
        "image/svg+xml"
    } else if rel.ends_with(".json") {
        "application/json"
    } else {
        "text/plain"
    };
    crate::bridge_http::reply(stream, 200, ct, &body);
}

fn handle_line(shared: &Arc<Mutex<WpsShared>>, raw: &str) {
    // Track the external presentation even while the independent whiteboard is
    // visible. UI consumers update only the saved presentation page in that mode.
    let line = raw.trim();
    if line.is_empty() {
        return;
    }
    if line.starts_with("SLOW ") {
        crate::diag_log::warn(line);
        return;
    }
    if line.starts_with("SSE ") || line.starts_with("CHANNEL ") || line.starts_with("WS ") {
        if line.contains("fallback") || line.contains("unavailable") || line.contains("failed") || line.contains("closed") {
            crate::diag_log::warn(line);
        } else {
            crate::diag_log::info(line);
        }
        return;
    }
    if line.starts_with("COMMAND_FAILED ") {
        let mut s = shared.lock().unwrap();
        s.command_failed_until = now_ms() + 3000;
        s.start_pending_until = 0;
        s.queue.clear();
        wps_log(line);
        return;
    }
    if line.starts_with("COMMAND_OK ") {
        wps_log(line);
        return;
    }
    if !line.starts_with("EVENT ") {
        return;
    }
    let pos = extract_num(line, "pos=");
    let name = line.split_whitespace().nth(1).unwrap_or("");
    if !matches!(name, "SlideShowState" | "SlideShowBegin" | "SlideShowEnd" | "SlideShowNextSlide" | "SlideShowNextClick" | "SlideShowOnNext" | "SlideShowOnPrevious") { return; }
    let mut s = shared.lock().unwrap();
    if s.events.len() >= 64 { s.events.drain(..32); }
    let next_deck = extract_num(line, "deck=");
    let changed_deck = name == "SlideShowState" && pos > 0 && s.real_pos > 0
        && next_deck > 0 && s.deck_epoch > 0 && next_deck != s.deck_epoch;
    let page_event = matches!(name, "SlideShowState" | "SlideShowNextSlide" | "SlideShowNextClick" | "SlideShowBegin" | "SlideShowOnNext" | "SlideShowOnPrevious");
    let transient_negative = name == "SlideShowState" && pos < 1 && s.real_pos > 0;
    if s.test_mode && page_event && !transient_negative {
        if name == "SlideShowState" {
            // Old add-ins do not send ready=; they must never advertise Start.
            s.can_start = pos < 1 && extract_num(line, "ready=") == 1;
        }
        // Page fields are valid on page-change events too, not only SlideShowState.
        let page = extract_num(line, "page=");
        let total = extract_num(line, "total=");
        if name == "SlideShowState" || page > 0 || total > 0 {
            s.page_total = if total > 0 { total } else { -1 };
            s.page_index = if page > 0 && (total < 1 || page <= total) { page } else { -1 };
            s.deck_epoch = extract_num(line, "deck=");
            s.can_jump = extract_num(line, "jump=") == 1 && s.deck_epoch > 0 && page > 0 && total >= page;
        }
        let (can_jump, epoch) = (s.can_jump, s.deck_epoch);
        s.queue.retain(|(_, cmd)| !cmd.starts_with("GOTO ") || (can_jump && jump_command(cmd).is_some_and(|(_, id)| id == epoch)));
    } else if !s.test_mode {
        // Default mode: only API paging and page/animation position.
        s.can_start = false;
        s.page_index = -1;
        s.page_total = -1;
        s.deck_epoch = -1;
        s.can_jump = false;
        let (can_jump, epoch) = (s.can_jump, s.deck_epoch);
        s.queue.retain(|(_, cmd)| !cmd.starts_with("GOTO ") || (can_jump && jump_command(cmd).is_some_and(|(_, id)| id == epoch)));
    }
    if name == "SlideShowEnd" || (name == "SlideShowState" && pos < 1) {
        if transient_negative {
            if s.negative_since == 0 { s.negative_since = now_ms(); }
            if now_ms() - s.negative_since < 1200 {
                // WPS can briefly return pos=-1 during a slide transition.
                // Do not tear down the presentation UI on a single sample.
                return;
            }
        }
        s.negative_since = 0;
        if s.real_pos > 0 {
            s.events.push(WpsEvent::SlideshowEnd);
            wps_log("放映结束");
        }
        s.real_pos = -1;
        if name == "SlideShowEnd" { s.can_start = false; s.start_pending_until = 0; s.can_jump = false; }
        let (can_start, can_jump) = (s.can_start, s.can_jump);
        s.queue.retain(|(_, cmd)| (can_start && cmd == "START") || (can_jump && cmd.starts_with("GOTO ")));
        return;
    }
    if pos > 0 && (pos != s.real_pos || changed_deck) {
        s.negative_since = 0;
        s.can_start = false;
        s.start_pending_until = 0;
        s.queue.retain(|(_, cmd)| cmd != "START");
        if s.real_pos < 1 || changed_deck {
            if changed_deck { s.queue.clear(); }
            s.events.push(WpsEvent::SlideshowBegin(pos));
        } else {
            s.events.push(WpsEvent::RealPos(pos));
        }
        s.real_pos = pos;
        wps_log(&format!("事件 {} pos={}", name, pos));
    }
}

fn extract_num(s: &str, key: &str) -> i32 {
    s.split_whitespace().find_map(|word| word.strip_prefix(key))
        .and_then(|value| value.parse::<i32>().ok()).filter(|n| *n > 0).unwrap_or(-1)
}

fn jump_command(cmd: &str) -> Option<(i32, i32)> {
    let mut words = cmd.split_whitespace();
    if words.next()? != "GOTO" { return None; }
    let page: i32 = words.next()?.parse().ok()?;
    let epoch: i32 = words.next()?.parse().ok()?;
    (page > 0 && epoch > 0 && words.next().is_none()).then_some((page, epoch))
}

fn take_command(s: &mut WpsShared) -> Option<String> {
    s.queue.retain(|(created, _)| now_ms() - created <= 1500);
    let playing = s.real_pos > 0;
    let can_start = s.can_start;
    let (can_jump, epoch, total) = (s.can_jump, s.deck_epoch, s.page_total);
    if s.whiteboard {
        s.queue.clear();
    } else {
        s.queue.retain(|(_, cmd)| {
            if let Some((page, id)) = jump_command(cmd) { can_jump && epoch == id && page <= total }
            else if cmd == "START" { can_start && !playing } else { playing }
        });
    }
    s.queue.pop_front().map(|(_, cmd)| cmd)
}

fn take_poll_command(shared: &Arc<Mutex<WpsShared>>) -> Option<String> {
    let mut s = shared.lock().unwrap();
    if !s.connected { wps_log("WPS 加载项已就绪"); }
    s.connected = true;
    s.last_seen = now_ms();
    take_command(&mut s)
}

fn sse_headers(stream: &mut TcpStream) -> std::io::Result<()> {
    let port = stream.local_addr().map(|address| address.port()).unwrap_or(16666);
    let header = format!(concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream; charset=utf-8\r\n",
        "Cache-Control: no-cache, no-store\r\n",
        "Connection: keep-alive\r\n",
        "Transfer-Encoding: chunked\r\n",
        "Access-Control-Allow-Origin: http://127.0.0.1:{port}\r\n",
        "Vary: Origin\r\n",
        "X-Accel-Buffering: no\r\n",
        "\r\n"
    ), port = port);
    stream.write_all(header.as_bytes())?;
    stream.flush()
}

fn sse_send(stream: &mut TcpStream, payload: &str) -> std::io::Result<()> {
    let chunk = format!("{:x}\r\n{}\r\n", payload.as_bytes().len(), payload);
    stream.write_all(chunk.as_bytes())?;
    stream.flush()
}

fn sse_end(stream: &mut TcpStream) {
    let _ = stream.write_all(b"0\r\n\r\n");
    let _ = stream.flush();
}

fn handle_sse(stream: &mut TcpStream, shared: &Arc<Mutex<WpsShared>>, shutdown: &Arc<AtomicBool>) {
    if sse_headers(stream).is_err() { return; }
    let clients = {
        let mut s = shared.lock().unwrap();
        s.sse_clients = s.sse_clients.saturating_add(1);
        s.connected = true;
        s.last_seen = now_ms();
        s.sse_clients
    };
    crate::diag_log::info(&format!("SSE 已连接（clients={clients}）"));
    let mut last_mode: Option<bool> = None;
    let mut last_beat = Instant::now();
    let mut last_state = Instant::now();
    loop {
        if shutdown.load(Ordering::Relaxed) { break; }
        let (mode, command) = {
            let mut s = shared.lock().unwrap();
            s.connected = true;
            s.last_seen = now_ms();
            (s.test_mode, take_command(&mut s))
        };
        let mut wrote = false;
        if last_mode != Some(mode) {
            let data = format!("data: MODE {}\n\n", if mode { "full" } else { "default" });
            if sse_send(stream, &data).is_err() { break; }
            last_mode = Some(mode);
            wrote = true;
        }
        if let Some(command) = command {
            wps_log(&format!("加载项取走 {}", command));
            if sse_send(stream, &format!("data: {command}\n\n")).is_err() { break; }
            wrote = true;
        }
        if last_state.elapsed() >= Duration::from_secs(5) {
            // Ask the add-in to push a fresh page/show snapshot over /push.
            // This catches a missed End event without recreating a polling loop.
            if sse_send(stream, "data: STATE\n\n").is_err() { break; }
            last_state = Instant::now();
            wrote = true;
        }
        if !wrote && last_beat.elapsed() >= Duration::from_secs(2) {
            if sse_send(stream, ": keepalive\n\n").is_err() { break; }
            last_beat = Instant::now();
        }
        if wrote { last_beat = Instant::now(); }
        std::thread::sleep(Duration::from_millis(20));
    }
    sse_end(stream);
    let clients = {
        let mut s = shared.lock().unwrap();
        s.sse_clients = s.sse_clients.saturating_sub(1);
        s.last_seen = now_ms();
        s.sse_clients
    };
    if shutdown.load(Ordering::Relaxed) {
        crate::diag_log::info(&format!("SSE 已关闭（程序退出，clients={clients}）"));
    } else {
        crate::diag_log::warn(&format!("SSE 已断开（clients={clients}）"));
    }
}

fn handle_ws_text(shared: &Arc<Mutex<WpsShared>>, text: &str) {
    let text = text.trim();
    if text.is_empty() { return; }
    if text.starts_with("WS ") {
        if text.contains("fallback") || text.contains("unavailable") || text.contains("closed") {
            crate::diag_log::warn(text);
        } else {
            crate::diag_log::info(text);
        }
        return;
    }
    if text.starts_with("EVENT ") || text.starts_with("COMMAND_OK ") || text.starts_with("COMMAND_FAILED ") {
        handle_line(shared, text);
    }
}

fn handle_ws(stream: &mut TcpStream, shared: &Arc<Mutex<WpsShared>>, shutdown: &Arc<AtomicBool>, key: &str) {
    if bridge_ws::handshake(stream, key).is_err() { return; }
    let mut read_stream = match stream.try_clone() { Ok(stream) => stream, Err(_) => return };
    let _ = read_stream.set_read_timeout(Some(Duration::from_secs(20)));
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || loop {
        match bridge_ws::read_frame(&mut read_stream) {
            Ok(frame) => {
                let close = frame.opcode == 0x8;
                if sender.send(frame).is_err() || close { break; }
            }
            Err(_) => {
                let _ = sender.send(bridge_ws::Frame { opcode: 0xff, payload: Vec::new() });
                break;
            }
        }
    });
    let clients = {
        let mut s = shared.lock().unwrap();
        s.ws_clients = s.ws_clients.saturating_add(1);
        s.connected = true;
        s.last_seen = now_ms();
        s.ws_clients
    };
    crate::diag_log::info(&format!("WS 已连接（clients={clients}）"));
    let mut last_mode: Option<bool> = None;
    let mut last_state = Instant::now() - Duration::from_secs(10);
    let mut last_ping = Instant::now() - Duration::from_secs(10);
    let mut last_pong = Instant::now();
    'ws: loop {
        if shutdown.load(Ordering::Relaxed) { break; }
        let (mode, command) = {
            let mut s = shared.lock().unwrap();
            s.connected = true;
            s.last_seen = now_ms();
            (s.test_mode, take_command(&mut s))
        };
        if last_mode != Some(mode) {
            if bridge_ws::write_text(stream, &format!("MODE {}", if mode { "full" } else { "default" })).is_err() { break; }
            last_mode = Some(mode);
        }
        if let Some(command) = command {
            wps_log(&format!("加载项取走 {}", command));
            if bridge_ws::write_text(stream, &command).is_err() { break; }
        }
        if last_state.elapsed() >= Duration::from_secs(5) {
            if bridge_ws::write_text(stream, "STATE").is_err() { break; }
            last_state = Instant::now();
        }
        if last_ping.elapsed() >= Duration::from_secs(5) {
            if bridge_ws::write_frame(stream, 0x9, &[]).is_err() { break; }
            last_ping = Instant::now();
        }
        loop {
            match receiver.try_recv() {
                Ok(frame) => match frame.opcode {
                    0x1 => {
                        if let Ok(text) = std::str::from_utf8(&frame.payload) {
                            handle_ws_text(shared, text);
                        }
                    }
                    0x9 => { if bridge_ws::write_frame(stream, 0xA, &frame.payload).is_err() { break 'ws; } }
                    0xA => { last_pong = Instant::now(); }
                    0x8 => break 'ws,
                    0xff => break 'ws,
                    _ => {}
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break 'ws,
            }
        }
        if last_pong.elapsed() > Duration::from_secs(15) { break; }
        std::thread::sleep(Duration::from_millis(20));
    }
    bridge_ws::write_close(stream);
    let clients = {
        let mut s = shared.lock().unwrap();
        s.ws_clients = s.ws_clients.saturating_sub(1);
        s.last_seen = now_ms();
        s.ws_clients
    };
    if shutdown.load(Ordering::Relaxed) {
        crate::diag_log::info(&format!("WS 已关闭（程序退出，clients={clients}）"));
    } else {
        crate::diag_log::warn(&format!("WS 已断开（clients={clients}）"));
    }
}

fn handle_conn(stream: &mut TcpStream, shared: &Arc<Mutex<WpsShared>>, shutdown: &Arc<AtomicBool>) {
    let request = match crate::bridge_http::read(stream) {
        Ok(request) => request,
        Err(status) => { crate::bridge_http::reply(stream, status, "text/plain", b"Request rejected"); return; }
    };
    let method = request.method.as_str();
    let target = request.target.as_str();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };
    let m = query_m(query);
    if m.as_ref().is_some_and(|m| m.len() > 2048 || m.chars().any(|ch| ch.is_control())) {
        crate::bridge_http::reply(stream, 400, "text/plain", b"Invalid message"); return;
    }

    if method == "OPTIONS" {
        if !matches!(path, "/hello" | "/push" | "/poll" | "/config" | "/events" | "/ws") && crate::bridge_http::static_path(path).is_none() {
            crate::bridge_http::reply(stream, 404, "text/plain", b"Not found"); return;
        }
        reply(stream, b"");
        return;
    }
    if matches!(path, "/push" | "/poll") {
        match accept_sequence(shared, query) {
            Ok(true) => (),
            Ok(false) => { reply(stream, if path == "/push" { b"OK" } else { b"" }); return; }
            Err(()) => { crate::bridge_http::reply(stream, 400, "text/plain", b"Invalid sequence"); return; }
        }
    }
    match path {
        "/ws" => {
            if method != "GET" {
                crate::bridge_http::reply(stream, 405, "text/plain", b"Method not allowed");
                return;
            }
            let headers = &request.headers;
            let values = |name: &str| headers.iter().filter(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str()).collect::<Vec<_>>();
            let upgrade = values("upgrade");
            let connection = values("connection");
            let version = values("sec-websocket-version");
            let keys = values("sec-websocket-key");
            if upgrade.len() != 1 || !upgrade[0].eq_ignore_ascii_case("websocket")
                || connection.len() != 1 || !connection[0].to_ascii_lowercase().contains("upgrade")
                || version.len() != 1 || version[0] != "13"
                || keys.len() != 1 || keys[0].is_empty() {
                crate::bridge_http::reply(stream, 400, "text/plain", b"Invalid websocket handshake");
                return;
            }
            handle_ws(stream, shared, shutdown, keys[0]);
        }
        "/events" => {
            if method != "GET" {
                crate::bridge_http::reply(stream, 405, "text/plain", b"Method not allowed");
                return;
            }
            handle_sse(stream, shared, shutdown);
        }
        "/hello" => {
            wps_log(&format!("加载项问候: {}", m.unwrap_or_default()));
            reply(stream, b"OK sidera");
        }
        "/config" => {
            let full = shared.lock().unwrap().test_mode;
            reply(stream, if full { b"mode=full" } else { b"mode=default" });
        }
        "/push" => {
            if let Some(m) = m {
                handle_line(shared, &m);
            }
            reply(stream, b"OK");
        }
        "/poll" => {
            // 只有带实际放映状态的轮询才算联动就绪。加载静态文件、
            // 启动探针或浏览器预检不能把按钮错误地显示为可用。
            let Some(state) = m.filter(|m| m.starts_with("EVENT SlideShowState pos=")) else {
                reply(stream, b"");
                return;
            };
            handle_line(shared, &state);
            let wait = query.split('&').any(|part| part == "wait=1");
            let deadline = Instant::now() + Duration::from_millis(900);
            loop {
                let cmd = take_poll_command(shared);
                if cmd.is_some() || !wait || Instant::now() >= deadline {
                    if let Some(c) = &cmd { wps_log(&format!("加载项取走 {}", c)); }
                    reply(stream, cmd.unwrap_or_default().as_bytes());
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        _ => reply_file(stream, path),
    }
}

impl WpsBridge {
    pub fn start() -> Option<WpsBridge> {
        let listener = TcpListener::bind(("127.0.0.1", 16666)).ok()?;
        let bridge = Self::from_listener(listener);
        wps_log("WPS 桥已启动，监听 127.0.0.1:16666");
        crate::diag_log::info("WPS 桥已启动，监听 127.0.0.1:16666");
        match addin_dir() {
            Some(d) => wps_log(&format!("加载项目录: {}", d.display())),
            None => wps_log("警告: 未找到 wps-addin 目录（请设置 WPS_ADDIN_DIR）"),
        }
        ensure_addin_registered_if_needed();
        Some(bridge)
    }

    pub(crate) fn from_listener(listener: TcpListener) -> WpsBridge {
        let _ = listener.set_nonblocking(true);
        let shared = Arc::new(Mutex::new(WpsShared::default()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let s2 = shared.clone();
        let sd = shutdown.clone();
        let handle = std::thread::spawn(move || {
            while !sd.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut st, _)) => {
                        if active.fetch_add(1, Ordering::Relaxed) >= 8 {
                            active.fetch_sub(1, Ordering::Relaxed);
                            drop(st);
                            continue;
                        }
                        // Windows may inherit the listener's nonblocking mode on
                        // accepted sockets; switch back so read timeouts work.
                        let _ = st.set_nonblocking(false);
                        let _ = st.set_read_timeout(Some(Duration::from_secs(1)));
                        let s3 = s2.clone();
                        let sd3 = sd.clone();
                        let active3 = active.clone();
                        std::thread::spawn(move || {
                            handle_conn(&mut st, &s3, &sd3);
                            active3.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
            }
            // listener 随线程结束被 drop，端口在此释放
        });
        WpsBridge {
            shared,
            shutdown,
            handle: Some(handle),
        }
    }

    pub fn connected(&self) -> bool {
        let s = self.shared.lock().unwrap();
        s.connected && now_ms() - s.last_seen <= 3000
    }

    pub fn ready(&self) -> bool {
        let s = self.shared.lock().unwrap();
        s.connected && now_ms() - s.last_seen <= 3000 && s.real_pos > 0
            && !s.whiteboard && now_ms() >= s.command_failed_until
    }

    pub fn showing(&self) -> bool {
        self.position().is_some()
    }

    pub fn position(&self) -> Option<i32> {
        let s = self.shared.lock().unwrap();
        (s.connected && now_ms() - s.last_seen <= 3000 && s.real_pos > 0 && !s.whiteboard).then_some(s.real_pos)
    }

    pub fn start_available(&self) -> bool {
        let s = self.shared.lock().unwrap();
        s.test_mode && s.connected && now_ms() - s.last_seen <= 3000 && s.can_start && s.real_pos < 1 && !s.whiteboard
    }

    pub fn page_progress(&self) -> (i32, i32) {
        let s = self.shared.lock().unwrap();
        if s.test_mode && s.connected && now_ms() - s.last_seen <= 3000 && !s.whiteboard {
            (s.page_index, s.page_total)
        } else {
            (-1, -1)
        }
    }

    pub fn enqueue(&self, cmd: &str) -> bool {
        let mut s = self.shared.lock().unwrap();
        let start = cmd == "START";
        if !matches!(cmd, "START" | "NEXT" | "PREV" | "EXIT") || !s.connected
            || now_ms() - s.last_seen > 3000 || s.whiteboard
            || (!s.test_mode && !matches!(cmd, "NEXT" | "PREV"))
            || (start && (!s.can_start || s.real_pos > 0 || now_ms() < s.start_pending_until))
            || (!start && s.real_pos < 1)
            || now_ms() < s.command_failed_until {
            return false;
        }
        s.queue.retain(|(created, _)| now_ms() - created <= 1500);
        if s.queue.len() >= 8 { return false; }
        s.queue.push_back((now_ms(), cmd.to_string()));
        if start { s.start_pending_until = now_ms() + 3000; }
        wps_log(&format!("入队指令 {}", cmd));
        true
    }

    pub fn jump_epoch(&self) -> Option<i32> {
        let s = self.shared.lock().unwrap();
        (s.test_mode && s.connected && now_ms() - s.last_seen <= 3000 && !s.whiteboard && s.can_jump
            && now_ms() >= s.command_failed_until).then_some(s.deck_epoch)
    }

    pub fn enqueue_jump(&self, page: i32, epoch: i32) -> bool {
        let mut s = self.shared.lock().unwrap();
        if !s.test_mode || !s.connected || now_ms() - s.last_seen > 3000 || s.whiteboard || !s.can_jump
            || epoch != s.deck_epoch || page < 1 || page > s.page_total || now_ms() < s.command_failed_until {
            return false;
        }
        // Keep only the newest absolute destination; do not replay a trail of
        // intermediate selections after a slow or temporarily blocked poll.
        s.queue.retain(|(created, cmd)| now_ms() - created <= 1500 && !cmd.starts_with("GOTO "));
        if s.queue.len() >= 8 { return false; }
        s.queue.push_back((now_ms(), format!("GOTO {page} {epoch}")));
        true
    }

    pub fn set_whiteboard(&self, on: bool) {
        let mut s = self.shared.lock().unwrap();
        if s.whiteboard != on {
            s.whiteboard = on;
            s.start_pending_until = 0;
            s.queue.clear();
        }
    }

    pub fn set_test_mode(&self, on: bool) {
        let mut s = self.shared.lock().unwrap();
        if s.test_mode == on { return; }
        s.test_mode = on;
        crate::diag_log::info(if on { "WPS 模式切换：测试模式" } else { "WPS 模式切换：默认模式" });
        s.start_pending_until = 0;
        if on {
            s.queue.clear();
        } else {
            s.page_index = -1;
            s.page_total = -1;
            s.can_jump = false;
            s.deck_epoch = -1;
            s.can_start = false;
            s.queue.retain(|(_, cmd)| matches!(cmd.as_str(), "NEXT" | "PREV"));
        }
    }

    pub fn take_events(&self) -> Vec<WpsEvent> {
        let mut s = self.shared.lock().unwrap();
        std::mem::take(&mut s.events)
    }

    /// 心跳检查：3 秒无请求判离线
    pub fn tick(&self) -> bool {
        let mut s = self.shared.lock().unwrap();
        if s.connected && now_ms() - s.last_seen > 3000 {
            s.connected = false;
            s.can_jump = false;
            s.can_start = false;
            s.start_pending_until = 0;
            s.queue.clear();
            // 保留已知页号，短暂断线后重连同一页不清除已有墨迹。
            drop(s);
            wps_log("客户端离线（3s 无请求）");
            crate::diag_log::warn("WPS 加载项 3 秒无请求，判定离线");
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn registration_timeout_does_not_block_the_bridge() {
        let directory = std::env::temp_dir().join(format!("sidera-register-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("install.sh");
        std::fs::write(&script, "printf registered").unwrap();
        let output = run_script(&script, &[], Duration::from_secs(1)).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"registered");
        std::fs::write(&script, "exec sleep 30").unwrap();
        let start = std::time::Instant::now();
        assert_eq!(run_script(&script, &[], Duration::from_millis(100)).unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ordered_requests_and_document_changes_preserve_latest_state() {
        let shared = Arc::new(Mutex::new(WpsShared::default()));
        assert_eq!(accept_sequence(&shared, "sid=classroom-1&seq=2"), Ok(true));
        handle_line(&shared, "EVENT SlideShowState pos=4 page=4 total=30 jump=1 deck=90");
        assert_eq!(accept_sequence(&shared, "sid=classroom-1&seq=1"), Ok(false));
        assert_eq!(accept_sequence(&shared, "sid=classroom-1&seq=2"), Ok(false));
        assert_eq!(accept_sequence(&shared, "sid=classroom-1&seq=3"), Ok(true));
        handle_line(&shared, "EVENT SlideShowState pos=4 page=4 total=30 jump=1 deck=91");
        assert_eq!(shared.lock().unwrap().events.iter().filter(|event| matches!(event, WpsEvent::SlideshowBegin(4))).count(), 2);
        assert!(accept_sequence(&shared, "sid=bad!&seq=1").is_err());
        assert!(accept_sequence(&shared, "sid=classroom&seq=nan").is_err());
        assert_eq!(accept_sequence(&shared, "m=legacy"), Ok(true));
        for page in 1..=500 { handle_line(&shared, &format!("EVENT SlideShowState pos={page}")); }
        assert!(shared.lock().unwrap().events.len() <= 64);
        assert_eq!(shared.lock().unwrap().real_pos, 500);
    }

    #[test]
    fn absolute_jump_requires_fresh_capability_and_replaces_stale_destinations() {
        let bridge = WpsBridge::from_listener(TcpListener::bind(("127.0.0.1", 0)).unwrap());
        {
            let mut s = bridge.shared.lock().unwrap();
            s.connected = true; s.last_seen = now_ms();
        }
        handle_line(&bridge.shared, "EVENT SlideShowState pos=-1 page=3 total=20 jump=1 deck=90");
        assert!(bridge.enqueue_jump(8, 90));
        assert!(bridge.enqueue_jump(12, 90));
        assert!(!bridge.enqueue_jump(21, 90));
        assert!(!bridge.enqueue_jump(5, 89));
        assert_eq!(bridge.shared.lock().unwrap().queue.len(), 1);
        assert_eq!(bridge.shared.lock().unwrap().queue[0].1, "GOTO 12 90");
        handle_line(&bridge.shared, "EVENT SlideShowState pos=-1 page=3 total=20 jump=1 deck=91");
        assert!(bridge.shared.lock().unwrap().queue.is_empty());
        assert!(!bridge.enqueue_jump(5, 90));
        handle_line(&bridge.shared, "EVENT SlideShowState pos=3 page=3 total=20");
        assert_eq!(bridge.jump_epoch(), None, "older add-ins cannot advertise an unsupported command");
        assert!(!bridge.enqueue_jump(5, 91));
    }
    use std::io::{Read, Write};

    fn request(port: u16, req: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let req = req.replace("Host: x\r\n", &format!("Host: 127.0.0.1:{port}\r\n"));
        s.write_all(req.as_bytes()).unwrap();
        s.flush().unwrap();
        let mut buf = String::new();
        let _ = s.read_to_string(&mut buf);
        buf
    }

    #[test]
    fn sse_stream_sends_mode_then_commands() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let bridge = WpsBridge::from_listener(listener);
        {
            let mut s = bridge.shared.lock().unwrap();
            s.connected = true; s.last_seen = now_ms(); s.real_pos = 1; s.test_mode = true;
        }
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            stream.write_all(format!(
                "GET /events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: keep-alive\r\n\r\n"
            ).as_bytes()).unwrap();
            let mut text = String::new();
            let mut buffer = [0u8; 4096];
            while !text.contains("data: MODE full") {
                let n = stream.read(&mut buffer).unwrap();
                if n == 0 { break; }
                text.push_str(&String::from_utf8_lossy(&buffer[..n]));
            }
            assert!(text.contains("Transfer-Encoding: chunked"), "SSE must be chunked: {text:?}");
            stream
        });
        let mut stream = client.join().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(bridge.enqueue("NEXT"));
        let mut text = String::new();
        let mut buffer = [0u8; 4096];
        while !text.contains("data: NEXT") {
            let n = stream.read(&mut buffer).unwrap();
            if n == 0 { break; }
            text.push_str(&String::from_utf8_lossy(&buffer[..n]));
        }
        assert!(text.contains("data: NEXT"), "SSE command missing: {text:?}");
    }

    fn read_server_frame(stream: &mut TcpStream) -> Option<(u8, Vec<u8>)> {
        let mut header = [0u8; 2];
        stream.read_exact(&mut header).ok()?;
        let opcode = header[0] & 0x0f;
        let length = (header[1] & 0x7f) as usize;
        let mut payload = vec![0u8; length];
        stream.read_exact(&mut payload).ok()?;
        Some((opcode, payload))
    }

    #[test]
    fn websocket_stream_delivers_mode_and_commands() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let bridge = WpsBridge::from_listener(listener);
        {
            let mut s = bridge.shared.lock().unwrap();
            s.connected = true; s.last_seen = now_ms(); s.real_pos = 1; s.test_mode = true;
        }
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            stream.write_all(format!(
                "GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
            ).as_bytes()).unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0u8; 1];
                stream.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            assert!(String::from_utf8_lossy(&header).starts_with("HTTP/1.1 101"));
            let mut text = String::new();
            while !text.contains("MODE full") {
                let (opcode, payload) = read_server_frame(&mut stream).expect("server frame");
                if opcode == 1 { text.push_str(&String::from_utf8_lossy(&payload)); }
            }
            ready_tx.send(()).unwrap();
            while !text.contains("NEXT") {
                let (opcode, payload) = read_server_frame(&mut stream).expect("server command");
                if opcode == 1 { text.push_str(&String::from_utf8_lossy(&payload)); }
            }
            text
        });
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(bridge.enqueue("NEXT"));
        let text = client.join().unwrap();
        assert!(text.contains("NEXT"), "WS command missing: {text:?}");
    }

    #[test]
    fn long_poll_delivers_a_command_before_its_deadline() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let bridge = WpsBridge::from_listener(listener);
        {
            let mut s = bridge.shared.lock().unwrap();
            s.connected = true; s.last_seen = now_ms(); s.real_pos = 1; s.test_mode = true;
        }
        let poll = std::thread::spawn(move || request(port,
            "GET /poll?wait=1&m=EVENT%20SlideShowState%20pos=1 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"));
        std::thread::sleep(Duration::from_millis(120));
        assert!(bridge.enqueue("NEXT"));
        let response = poll.join().unwrap();
        assert!(response.contains("NEXT"), "long poll missed command: {response:?}");
    }

    #[test]
    fn bridge_endpoints() {
        // 临时端口、不运行注册器：测试不能占用生产桥或修改用户 WPS 配置。
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let bridge = WpsBridge::from_listener(listener);
        let hello = request(port, "GET /hello?m=sidera-bridge HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        assert!(hello.contains("OK sidera"), "hello 响应异常: {hello:?}");
        assert!(!bridge.connected());
        assert!(!bridge.enqueue("NEXT"));

        let push = request(
            port,
            "GET /push?m=EVENT%20SlideShowBegin%20pos=3%20click=0 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert!(push.contains("OK"), "push 响应异常: {push:?}");

        let poll_request = "GET /poll?m=EVENT%20SlideShowState%20pos=3%20click=0 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
        request(port, poll_request);
        assert!(bridge.ready());
        assert!(bridge.enqueue("NEXT"));
        let poll = request(port, poll_request);
        assert!(poll.contains("NEXT"), "poll 响应异常: {poll:?}");

        // 事件应被记录
        let events = bridge.take_events();
        assert!(
            events.iter().any(|e| matches!(e, WpsEvent::SlideshowBegin(3))),
            "未收到 SlideshowBegin 事件: {events:?}"
        );

        // Missing fixtures must fail, not silently skip resource verification.
        assert!(addin_dir().is_some(), "run tests from the repository's rust directory");
        {
            let m = request(
                port,
                "GET /manifest.xml HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
            );
            assert!(
                m.starts_with("HTTP/1.1 200") && m.contains("<JsPlugin"),
                "静态 manifest 分发异常: {m:?}"
            );
        }

        assert!(bridge.enqueue("PREV"));
        bridge.shared.lock().unwrap().queue.front_mut().unwrap().0 -= 1600;
        assert!(!request(port, poll_request).contains("PREV"), "过期指令不得补执行");
        assert!(bridge.enqueue("EXIT"));
        request(port, "GET /push?m=EVENT%20SlideShowEnd%20pos=-1%20click=-1 HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(!bridge.ready());
        assert!(bridge.shared.lock().unwrap().queue.is_empty());
        assert!(bridge.take_events().iter().any(|e| matches!(e, WpsEvent::SlideshowEnd)));

        // 回归：Drop 停止线程并释放端口。
        drop(bridge);
        TcpListener::bind(("127.0.0.1", port)).expect("关闭桥后端口未释放");
    }

    #[test]
    fn readiness_disconnect_whiteboard_and_command_failure() {
        let bridge = WpsBridge::from_listener(TcpListener::bind(("127.0.0.1", 0)).unwrap());
        {
            let mut s = bridge.shared.lock().unwrap();
            s.connected = true; s.last_seen = now_ms(); s.real_pos = 2;
        }
        assert!(bridge.enqueue("NEXT"));
        bridge.shared.lock().unwrap().last_seen -= 4000;
        assert!(!bridge.ready());
        assert!(!bridge.enqueue("NEXT")); // 不依赖 UI 心跳定时器及时运行。
        assert!(bridge.tick());
        assert!(bridge.shared.lock().unwrap().queue.is_empty());
        {
            let mut s = bridge.shared.lock().unwrap();
            s.connected = true; s.last_seen = now_ms();
        }
        assert!(bridge.enqueue("PREV"));
        bridge.set_whiteboard(true);
        assert!(!bridge.ready());
        assert!(bridge.shared.lock().unwrap().queue.is_empty());
        bridge.set_whiteboard(false);
        assert!(bridge.ready());
        handle_line(&bridge.shared, "COMMAND_FAILED NEXT");
        assert!(!bridge.ready());
        assert!(!bridge.enqueue("NEXT"));
    }

    #[test]
    fn repeated_state_and_animation_do_not_switch_pages() {
        let shared = Arc::new(Mutex::new(WpsShared::default()));
        handle_line(&shared, "EVENT SlideShowState pos=1 click=0");
        handle_line(&shared, "EVENT SlideShowBegin pos=1 click=0");
        handle_line(&shared, "EVENT SlideShowState pos=1 click=1");
        assert_eq!(shared.lock().unwrap().events.len(), 1);
        handle_line(&shared, "EVENT SlideShowState pos=2 click=0");
        assert!(matches!(shared.lock().unwrap().events.last(), Some(WpsEvent::RealPos(2))));
        handle_line(&shared, "EVENT SlideShowEnd pos=-1 click=-1");
        handle_line(&shared, "EVENT SlideShowState pos=-1 click=-1");
        assert_eq!(shared.lock().unwrap().events.len(), 3);
        handle_line(&shared, "EVENT SlideShowState pos=2 click=0");
        assert!(matches!(shared.lock().unwrap().events.last(), Some(WpsEvent::SlideshowBegin(2))));
    }

    #[test]
    fn page_counters_are_separate_from_show_order_and_expire_with_the_link() {
        let bridge = WpsBridge::from_listener(TcpListener::bind(("127.0.0.1", 0)).unwrap());
        {
            let mut s = bridge.shared.lock().unwrap();
            s.connected = true; s.last_seen = now_ms();
        }
        handle_line(&bridge.shared, "EVENT SlideShowState pos=2 click=0 page=9 total=30");
        assert_eq!(bridge.page_progress(), (9, 30));
        assert_eq!(bridge.position(), Some(2));
        assert!(matches!(bridge.take_events().as_slice(), [WpsEvent::SlideshowBegin(2)]));
        handle_line(&bridge.shared, "EVENT SlideShowState pos=2 click=1 page=9 total=31");
        assert_eq!(bridge.page_progress(), (9, 31));
        assert!(bridge.take_events().is_empty(), "counter changes must not clear annotation pages");
        handle_line(&bridge.shared, "EVENT SlideShowEnd pos=-1 click=-1 ready=1 page=6 total=31");
        handle_line(&bridge.shared, "EVENT SlideShowState pos=-1 click=-1 ready=1 page=6 total=31");
        assert_eq!(bridge.page_progress(), (6, 31), "editing still has a real current slide");
        assert_eq!(bridge.position(), None);
        bridge.shared.lock().unwrap().last_seen -= 4000;
        assert_eq!(bridge.page_progress(), (-1, -1), "do not display stale metadata while offline");
        bridge.shared.lock().unwrap().last_seen = now_ms();
        bridge.set_whiteboard(true);
        handle_line(&bridge.shared, "EVENT SlideShowState pos=3 click=0 page=10 total=31");
        bridge.set_whiteboard(false);
        assert_eq!(bridge.page_progress(), (10, 31), "fresh external state survives whiteboard exit");
        handle_line(&bridge.shared, "EVENT SlideShowState pos=3 click=0 page=10 total=31");
        assert_eq!(bridge.page_progress(), (10, 31));
        handle_line(&bridge.shared, "EVENT SlideShowState pos=3 click=0");
        assert_eq!(bridge.page_progress(), (-1, -1), "older add-ins clear previous counters");
        handle_line(&bridge.shared, "EVENT SlideShowState pos=3 click=0 page=32 total=31");
        assert_eq!(bridge.page_progress(), (-1, 31));
    }

    #[test]
    fn registration_files_and_preflight_do_not_establish_a_connection() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let bridge = WpsBridge::from_listener(listener);
        for req in ["GET /ribbon.xml", "OPTIONS /poll", "GET /poll", "GET /hello"] {
            request(port, &format!("{req} HTTP/1.1\r\nHost: x\r\n\r\n"));
            assert!(!bridge.connected());
            assert!(!bridge.ready());
        }
    }

    #[test]
    fn page_fields_on_slide_events_update_without_creating_page_events() {
        let shared = Arc::new(Mutex::new(WpsShared::default()));
        {
            let mut s = shared.lock().unwrap();
            s.connected = true; s.last_seen = now_ms();
        }
        handle_line(&shared, "EVENT SlideShowState pos=1 click=0 page=8 total=30 jump=1 deck=90");
        assert_eq!(shared.lock().unwrap().page_index, 8);
        let events = shared.lock().unwrap().events.len();
        handle_line(&shared, "EVENT SlideShowNextSlide pos=2 click=0 page=9 total=30 jump=1 deck=90");
        assert_eq!(shared.lock().unwrap().page_index, 9);
        assert_eq!(shared.lock().unwrap().events.len(), events + 1);
    }

    #[test]
    fn transient_negative_state_does_not_replace_show_page() {
        let shared = Arc::new(Mutex::new(WpsShared::default()));
        {
            let mut s = shared.lock().unwrap();
            s.connected = true; s.last_seen = now_ms();
        }
        handle_line(&shared, "EVENT SlideShowState pos=5 click=0 page=5 total=30");
        handle_line(&shared, "EVENT SlideShowState pos=-1 click=-1 page=1 total=30");
        assert_eq!(shared.lock().unwrap().page_index, 5);
        handle_line(&shared, "EVENT SlideShowEnd pos=-1 click=-1 page=1 total=30");
        handle_line(&shared, "EVENT SlideShowState pos=-1 click=-1 page=1 total=30");
        assert_eq!(shared.lock().unwrap().page_index, 1);
    }

    #[test]
    fn default_mode_keeps_only_position_and_api_paging() {
        let bridge = WpsBridge::from_listener(TcpListener::bind(("127.0.0.1", 0)).unwrap());
        {
            let mut s = bridge.shared.lock().unwrap();
            s.connected = true; s.last_seen = now_ms(); s.test_mode = false;
        }
        handle_line(&bridge.shared,
            "EVENT SlideShowState pos=3 ready=1 page=5 total=10 jump=1 deck=9");
        assert_eq!(bridge.position(), Some(3));
        assert_eq!(bridge.page_progress(), (-1, -1));
        assert!(!bridge.start_available());
        assert!(!bridge.enqueue("START"));
        assert!(!bridge.enqueue("EXIT"));
        assert!(!bridge.enqueue_jump(5, 9));
        assert!(bridge.enqueue("NEXT"));
        assert!(bridge.enqueue("PREV"));
    }

    #[test]
    fn start_requires_a_fresh_ready_deck_and_survives_idle_poll() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let bridge = WpsBridge::from_listener(listener);
        let ready = "GET /poll?m=EVENT%20SlideShowState%20pos=-1%20click=-1%20ready=1 HTTP/1.1\r\nHost: x\r\n\r\n";
        assert!(!bridge.start_available());
        assert!(!bridge.enqueue("START"));
        request(port, ready);
        assert!(bridge.start_available());
        assert!(!bridge.enqueue("NEXT"));
        assert!(bridge.enqueue("START"));
        assert!(!bridge.enqueue("START"));
        assert!(request(port, ready).ends_with("START"));
        assert!(!bridge.enqueue("START"), "repeated taps must wait for the show or timeout");
        handle_line(&bridge.shared, "EVENT SlideShowState pos=1 click=0 ready=0");
        assert!(!bridge.start_available());
        assert!(bridge.ready());
        assert!(!bridge.enqueue("START"));
        handle_line(&bridge.shared, "EVENT SlideShowEnd pos=-1 click=-1 ready=1");
        handle_line(&bridge.shared, "EVENT SlideShowState pos=-1 click=-1 ready=1");
        assert!(bridge.enqueue("START"));
        request(port, "GET /poll?m=EVENT%20SlideShowState%20pos=-1%20click=-1 HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(!bridge.start_available(), "old add-ins never advertise START");
        assert!(bridge.shared.lock().unwrap().queue.is_empty());
        request(port, ready);
        bridge.shared.lock().unwrap().last_seen -= 4000;
        assert!(!bridge.start_available());
        assert!(!bridge.enqueue("START"));
    }
}

#[cfg(all(test, windows))]
mod windows_registration_tests {
    use super::*;

    fn expected() -> Vec<(String, String)> {
        vec![
            ("name".into(), "sidera-bridge".into()),
            ("type".into(), "wpp".into()),
            ("url".into(), "http://127.0.0.1:16666/".into()),
            ("debug".into(), "".into()),
            ("enable".into(), "enable".into()),
            ("install".into(), "null".into()),
        ]
    }

    #[test]
    fn merge_preserves_other_entries_and_removes_legacy() {
        let dir = std::env::temp_dir().join(format!(
            "sidera-wps-register-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("publish.xml");
        std::fs::write(
            &path,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<jsplugins>
  <jspluginonline name="other" url="https://example.test/"/>
  <jspluginonline name="screen-annotate-bridge" url="http://127.0.0.1:16666/"/>
</jsplugins>
"#,
        )
        .unwrap();

        let first = rewrite_publish(&path, &expected(), false).unwrap();
        assert!(first.contains("已更新"), "{first}");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("name=\"other\""), "{text}");
        assert!(text.contains("name=\"sidera-bridge\""), "{text}");
        assert!(!text.contains("screen-annotate-bridge"), "{text}");
        assert_eq!(text.matches("name=\"sidera-bridge\"").count(), 1);

        rewrite_publish(&path, &expected(), false).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("name=\"sidera-bridge\"").count(), 1);

        let removed = rewrite_publish(&path, &[], true).unwrap();
        assert!(removed.contains("已清理"), "{removed}");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("sidera-bridge"), "{text}");
        assert!(text.contains("name=\"other\""), "{text}");

        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod registration_state_tests {
    use super::*;

    #[test]
    fn addin_fingerprint_tracks_content_and_version_inputs() {
        let dir = std::env::temp_dir().join(format!(
            "sidera-addin-fingerprint-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ADDIN_FILES {
            let path = dir.join(name);
            if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).unwrap(); }
            std::fs::write(path, name.as_bytes()).unwrap();
        }
        let first = addin_fingerprint(&dir).unwrap();
        assert_eq!(first, addin_fingerprint(&dir).unwrap());
        std::fs::write(dir.join("main.js"), b"changed").unwrap();
        assert_ne!(first, addin_fingerprint(&dir).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
