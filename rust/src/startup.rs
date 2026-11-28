//! 显示后端选择和启动失败反馈。自动模式尊重当前会话，不把 XWayland 当作原生 X11。

#[derive(Debug, PartialEq, Eq)]
pub enum DisplayBackend {
    X11,
    Wayland,
    Gnome,
}

pub fn detect_backend() -> Result<DisplayBackend, String> {
    let backend = match std::env::var("SIDERA_BACKEND") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err("SIDERA_BACKEND 必须是 auto、x11、wayland 或 gnome".into()),
    };
    select_backend(
        backend.as_deref(),
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("WAYLAND_SOCKET").ok().as_deref(),
    )
}

/// Only auto mode may use XWayland when the compositor lacks layer-shell.
/// Connection errors and explicit Wayland requests must remain errors.
pub fn allow_x11_fallback() -> bool {
    matches!(std::env::var("SIDERA_BACKEND").unwrap_or_default().trim().to_ascii_lowercase().as_str(), "" | "auto")
        && std::env::var("DISPLAY").is_ok_and(|value| !value.trim().is_empty())
}

/// Xwayland 24.1.10 can crash the entire desktop when mapping an ARGB window.
/// Check the connected server, not a package installed on the client machine.
pub fn validate_x11_server(is_xwayland: bool, release: u32, vendor: &[u8]) -> Result<(), String> {
    const FIXED_VENDOR: &[u8] = b"The X.Org Foundation (Sidera damage-report fix)";
    if is_xwayland && release == 12_401_010 && vendor != FIXED_VENDOR {
        return Err("Xwayland 24.1.10 显示透明窗口时可能导致桌面退出。请升级到 24.1.11 或更高版本，或安装 Sidera 的 damage_report 修复包。".into());
    }
    Ok(())
}

fn select_backend(
    requested: Option<&str>,
    session: Option<&str>,
    wayland_display: Option<&str>,
    wayland_socket: Option<&str>,
) -> Result<DisplayBackend, String> {
    match requested.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
        "x11" => return Ok(DisplayBackend::X11),
        "wayland" => return Ok(DisplayBackend::Wayland),
        "gnome" => return Ok(DisplayBackend::Gnome),
        "" | "auto" => (),
        _ => return Err("SIDERA_BACKEND 必须是 auto、x11、wayland 或 gnome".into()),
    }
    match session.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "x11" => return Ok(DisplayBackend::X11),
        "wayland" => return Ok(DisplayBackend::Wayland),
        _ => (),
    }
    if [wayland_display, wayland_socket]
        .iter()
        .any(|value| value.is_some_and(|v| !v.trim().is_empty()))
    {
        Ok(DisplayBackend::Wayland)
    } else {
        Ok(DisplayBackend::X11)
    }
}

pub fn report_error(error: &dyn std::fmt::Display) {
    let message = format!("Sidera 无法启动：{error}");
    log::error!("{message}");

    // 应用菜单通常不显示 stderr，保留一份可直接反馈的诊断。
    if let Some(dir) = dirs::state_dir().map(|path| path.join("sidera")) {
        let path = dir.join("startup-error.log");
        match std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&path, format!("{message}\n")))
        {
            Ok(()) => log::error!("启动诊断已保存到 {}", path.display()),
            Err(e) => log::warn!("无法保存启动诊断：{e}"),
        }
    }

    // 可选桌面通知；缺少 notify-send 或通知服务不会遮蔽原始错误。
    if std::env::var("SIDERA_NO_STARTUP_NOTIFICATION").as_deref() != Ok("1") {
        if let Ok(mut child) = std::process::Command::new("notify-send")
            .args(["--app-name=Sidera", "--urgency=critical", "--", "Sidera 无法启动", &error.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            // 等待通知送达，避免桌面启动器结束进程组时一起清理通知进程。
            // 通知服务无响应时最多等待两秒，不妨碍应用退出并返回原始错误。
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{select_backend, DisplayBackend::*};

    #[test]
    fn unsafe_xwayland_is_rejected_before_window_creation() {
        assert!(super::validate_x11_server(true, 12_401_010, b"The X.Org Foundation").is_err());
    }

    #[test]
    fn fixed_xwayland_and_native_x11_remain_available() {
        for (wayland, version, vendor) in [
            (true, 12_401_011, &b"The X.Org Foundation"[..]),
            (true, 12_401_009, &b"The X.Org Foundation"[..]),
            (false, 12_401_010, &b"The X.Org Foundation"[..]),
            (true, 12_401_010, &b"The X.Org Foundation (Sidera damage-report fix)"[..]),
        ] {
            assert!(super::validate_x11_server(wayland, version, vendor).is_ok());
        }
    }

    #[test]
    fn native_x11_session_wins_over_inherited_wayland_variables() {
        assert_eq!(select_backend(None, Some("x11"), Some("wayland-0"), Some("9")), Ok(X11));
    }

    #[test]
    fn wayland_session_uses_wayland_even_without_named_socket() {
        assert_eq!(select_backend(None, Some("wayland"), None, None), Ok(Wayland));
    }

    #[test]
    fn empty_wayland_variables_do_not_select_wayland() {
        for value in [None, Some(""), Some(" ")] {
            assert_eq!(select_backend(None, None, value, value), Ok(X11));
        }
    }

    #[test]
    fn unknown_session_uses_available_wayland_connection() {
        assert_eq!(select_backend(None, None, Some("wayland-1"), None), Ok(Wayland));
        assert_eq!(select_backend(None, Some("tty"), None, Some("7")), Ok(Wayland));
    }

    #[test]
    fn explicit_backend_overrides_session_without_rewriting_environment() {
        assert_eq!(select_backend(Some("x11"), Some("wayland"), Some("wayland-0"), None), Ok(X11));
        assert_eq!(select_backend(Some("wayland"), Some("x11"), None, None), Ok(Wayland));
        assert_eq!(select_backend(Some("gnome"), Some("wayland"), None, None), Ok(Gnome));
    }

    #[test]
    fn auto_and_case_whitespace_are_supported() {
        assert_eq!(select_backend(Some(" AUTO "), Some(" X11 "), Some("wayland-0"), None), Ok(X11));
        assert_eq!(select_backend(Some(" Wayland "), None, None, None), Ok(Wayland));
    }

    #[test]
    fn invalid_override_is_an_error_instead_of_silent_fallback() {
        for value in ["wl", "xorg", "typo"] {
            assert!(select_backend(Some(value), Some("x11"), None, None).is_err());
        }
    }
}
