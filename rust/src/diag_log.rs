//! 课堂诊断日志：固定在用户主目录下的单个文本文件，带严重程度前缀。
use std::io::Write;
use std::path::PathBuf;

pub fn path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("sidera-diagnostic.log"))
}

fn write(level: &str, message: &str) {
    if cfg!(test) { return; }
    let Some(path) = path() else { return };
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let Ok(mut file) = options.open(&path) else { return };
    if file.metadata().map(|m| m.len() > 1024 * 1024).unwrap_or(false) {
        let _ = file.set_len(0);
    }
    let _ = writeln!(file, "[{}] [{}] {}", crate::app::now_ms(), level, message);
}

pub fn info(message: &str) { write("INFO", message); }
pub fn warn(message: &str) { write("WARN", message); }
pub fn error(message: &str) { write("ERROR", message); }

pub fn tail(limit: usize) -> Vec<String> {
    let Some(path) = path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if lines.len() > limit { lines.drain(..lines.len() - limit); }
    lines
}

pub fn clear() {
    if let Some(path) = path() { let _ = std::fs::remove_file(path); }
}
