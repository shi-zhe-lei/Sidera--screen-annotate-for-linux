//! Settings navigation, draft colours, finite transitions and versioned appearance storage.
use crate::{app::{App, now_ms}, ui::Control, ui_theme::{Accent, Palette, Preference}};
use std::cell::RefCell;
use std::rc::Rc;

type InspectorCache = RefCell<Option<((u64, i32), Rc<Vec<String>>)>>;

pub const REPOSITORY: &str = "https://github.com/offduckling4825/Sidera--screen-annotate-for-linux";
pub const LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab { #[default] Appearance, Addins, General, Experimental, About }
impl Tab {
    pub const ALL: [Self; 5] = [Self::General, Self::Appearance, Self::Addins, Self::Experimental, Self::About];
    pub fn index(self) -> usize { Self::ALL.iter().position(|t| *t == self).unwrap_or(0) }
    pub fn label(self) -> &'static str { match self { Self::Appearance => "外观", Self::Addins => "加载项", Self::General => "常规", Self::Experimental => "实验性功能", Self::About => "关于" } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog { Addins, Register, Remove, ClearLogs }
impl Dialog {
    pub fn title(self) -> &'static str { match self {
        Self::Addins => "管理 WPS 加载项", Self::Register => "重新登记加载项？",
        Self::Remove => "清除加载项登记？", Self::ClearLogs => "清空运行日志？",
    } }
    pub fn body(self) -> &'static str { match self {
        Self::Addins => "修改登记可能导致 WPS 无法联动翻页。联动正常时请保留现有设置。",
        Self::Register => "Sidera 会重新写入当前用户的加载项登记。",
        Self::Remove => "清除后将停止 WPS 翻页和放映联动。重新登记即可恢复。",
        Self::ClearLogs => "清空后无法恢复。",
    } }
    pub fn action(self) -> &'static str { match self {
        Self::Addins => "进入管理", Self::Register => "重新登记", Self::Remove => "清除登记", Self::ClearLogs => "清空日志",
    } }
    pub fn destructive(self) -> bool { matches!(self, Self::Remove | Self::ClearLogs) }
    pub fn requires_wps_exit(self) -> bool { matches!(self, Self::Addins | Self::Register | Self::Remove) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogHit { Cancel, Continue }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerHit { Field, Hue, Edit(crate::value_input::Target), Preset(usize), Recent(usize), Cancel, Apply }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Inspector { #[default] Diagnostics, Logs, Notice }
impl Inspector {
    pub fn title(self) -> &'static str { match self { Self::Diagnostics => "系统诊断", Self::Logs => "运行日志", Self::Notice => "操作结果" } }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProcessView { Rules, #[default] Running }

#[derive(Clone, Debug)]
pub struct ColorDraft { pub hue: f32, pub saturation: f32, pub value: f32 }
impl ColorDraft {
    pub fn new(rgb: u32) -> Self {
        let r = ((rgb >> 16) & 255) as f32 / 255.0;
        let g = ((rgb >> 8) & 255) as f32 / 255.0;
        let b = (rgb & 255) as f32 / 255.0;
        let max = r.max(g).max(b); let min = r.min(g).min(b); let d = max - min;
        let hue = if d < 0.0001 { 0.0 } else if max == r { ((g - b) / d).rem_euclid(6.0) / 6.0 }
            else if max == g { ((b - r) / d + 2.0) / 6.0 } else { ((r - g) / d + 4.0) / 6.0 };
        Self { hue, saturation: if max == 0.0 { 0.0 } else { d / max }, value: max }
    }
    pub fn rgb(&self) -> u32 { hsv(self.hue, self.saturation, self.value) }
}
pub fn hsv(h: f32, s: f32, v: f32) -> u32 {
    let h = h.rem_euclid(1.0) * 6.0; let s = s.clamp(0.0, 1.0); let v = v.clamp(0.0, 1.0);
    let c = v * s; let x = c * (1.0 - (h % 2.0 - 1.0).abs()); let m = v - c;
    let (r,g,b) = match h as u32 { 0 => (c,x,0.0), 1 => (x,c,0.0), 2 => (0.0,c,x),
        3 => (0.0,x,c), 4 => (x,0.0,c), _ => (c,0.0,x) };
    (((r+m)*255.0).round() as u32) << 16 | (((g+m)*255.0).round() as u32) << 8 | ((b+m)*255.0).round() as u32
}

pub fn ease(start: i64, duration: i64) -> f32 {
    if start == 0 { return 1.0; }
    let t = ((now_ms() - start) as f32 / duration as f32).clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[derive(Default)]
pub struct State {
    #[cfg(unix)]
    pub(crate) scroll_access: crate::linux_scroll::ScrollAccess,
    #[cfg(unix)]
    pub(crate) window: Option<std::rc::Rc<crate::settings_window::Window>>,
    #[cfg(windows)]
    pub native_size: Option<(i32, i32)>,
    #[cfg(windows)]
    pub native_dirty: std::cell::Cell<bool>,
    pub tab: Tab,
    pub nav_from: f32,
    pub tab_at: i64,
    pub opened_at: i64,
    pub closing_at: i64,
    pub overlay_at: i64,
    pub overlay_closing_at: i64,
    pub dialog: Option<Dialog>,
    pub picker: Option<ColorDraft>,
    pub editor: Option<crate::value_input::Editor>,
    pub addins_unlocked: bool,
    pub autostart: bool,
    pub inspector: Inspector,
    pub inspector_page: usize,
    pub inspector_page_from: usize,
    pub inspector_page_at: i64,
    pub process_view: ProcessView,
    pub process_scroll_from: f32,
    pub process_scroll_at: i64,
    /// Wrapped diagnostics/log lines, keyed by (text hash, body width).
    pub inspector_cache: InspectorCache,
    pub recent: Vec<u32>,
    pub toast: String,
    pub toast_until: i64,
    pub theme_from: Option<(Palette, i64)>,
    pub toggle: Option<(Control, f32, i64)>,
    pub last_frame: i64,
    /// Scratch surface reused by fade transitions so animation frames do not
    /// allocate a full-window pixmap each time.
    pub tile: std::cell::RefCell<Option<tiny_skia::Pixmap>>,
}
impl State {
    pub fn window_size(&self) -> Option<(i32, i32)> {
        #[cfg(unix)]
        if let Some(window) = &self.window {
            return Some(window.logical_size());
        }
        #[cfg(windows)]
        if let Some(size) = self.native_size {
            return Some(size);
        }
        None
    }
    pub fn nav_position(&self) -> f32 { self.nav_from + (self.tab.index() as f32 - self.nav_from) * ease(self.tab_at, 210) }
    pub fn select(&mut self, tab: Tab) { self.nav_from = self.nav_position(); self.tab = tab; self.tab_at = now_ms(); }
    pub fn modal(&self) -> bool { self.dialog.is_some() || self.picker.is_some() || self.editor.is_some() }
    pub fn show_dialog(&mut self, dialog: Dialog) { self.dialog = Some(dialog); self.overlay_at = now_ms(); self.overlay_closing_at = 0; }
    pub fn notify(&mut self, text: impl Into<String>) { self.toast = text.into(); self.toast_until = now_ms() + 2200; }
    pub fn toggle_value(&self, control: Control, on: bool) -> f32 {
        let target = if on { 1.0 } else { 0.0 };
        if let Some((hit, from, start)) = self.toggle.filter(|(hit, _, _)| *hit == control) {
            let _ = hit; from + (target - from) * ease(start, 170)
        } else { target }
    }
    pub fn begin_toggle(&mut self, control: Control, old: bool) {
        let from = self.toggle_value(control, old); self.toggle = Some((control, from, now_ms()));
    }
    pub fn palette(&self, target: Palette) -> Palette {
        self.theme_from.map(|(from, start)| from.blend(target, ease(start, 220))).unwrap_or(target)
    }
    pub fn begin_inspector_page(&mut self, from: usize) {
        self.inspector_page_from = from;
        self.inspector_page_at = now_ms();
    }
    /// Current page and the progress (0..1) of a vertical page slide.
    pub fn inspector_slide(&self, page: usize) -> Option<(usize, f32)> {
        if self.inspector_page_at == 0 || self.inspector_page_from == page { return None; }
        let t = ease(self.inspector_page_at, 140);
        (t < 1.0).then_some((self.inspector_page_from, t))
    }
    pub fn begin_process_scroll(&mut self, from: f32) {
        self.process_scroll_from = from;
        self.process_scroll_at = now_ms();
    }
    pub fn process_scroll_visual(&self, target: usize) -> f32 {
        let target = target as f32;
        if self.process_scroll_at == 0 { return target; }
        self.process_scroll_from + (target - self.process_scroll_from) * ease(self.process_scroll_at, 140)
    }
    pub fn animating(&self) -> bool {
        let now = now_ms();
        [self.tab_at, self.opened_at, self.overlay_at, self.overlay_closing_at, self.closing_at]
            .iter().any(|start| *start > 0 && now - *start <= 230)
            // Scrolling finishes at 140ms; stop repainting immediately after.
            || [self.inspector_page_at, self.process_scroll_at]
                .iter().any(|start| *start > 0 && now - *start <= 140)
            || self.theme_from.is_some_and(|(_, start)| now - start <= 230)
            || self.toggle.is_some_and(|(_, _, start)| now - start <= 180)
            || self.editor.as_ref().is_some_and(|editor|now-editor.opened_at<=160)
            || (self.toast_until > 0 && now >= self.toast_until - 180 && now <= self.toast_until + 32)
    }
}

fn appearance_path() -> Option<std::path::PathBuf> {
    crate::app::config_path().map(|p| p.with_file_name("appearance-v1.json"))
}
pub fn load_appearance(app: &mut App) {
    let Some(path) = appearance_path() else { return };
    let Ok(bytes) = std::fs::read(path) else { return };
    let Ok(data) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return };
    if data["version"].as_u64() != Some(1) { return; }
    if let Some(preference) = data["mode"].as_str().and_then(Preference::parse) { app.theme.preference = preference; }
    if let Some(accent) = data["accent"].as_str().and_then(Accent::parse) { app.theme.accent = accent; }
    app.settings_ui.recent = data["recent"].as_array().into_iter().flatten()
        .filter_map(|v| v.as_str().and_then(Accent::parse).map(Accent::rgb)).take(8).collect();
    app.settings_ui.theme_from = None;
}
pub fn save_appearance(app: &App) -> Result<(), String> {
    if cfg!(test) { return Ok(()); }
    let path = appearance_path().ok_or("无法确定主题保存位置")?;
    // Preserve extra fields from a newer writer; never replace an unknown schema.
    let mut data = match std::fs::read(&path) {
        Ok(bytes) => {
            let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "主题配置损坏，已保留原文件")?;
            if value["version"].as_u64() != Some(1) { return Err("主题配置来自更新版本，已保留原文件".into()); }
            value
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({"version": 1}),
        Err(e) => return Err(e.to_string()),
    };
    data["mode"] = app.theme.preference.key().into();
    data["accent"] = app.theme.accent.key().into();
    data["recent"] = serde_json::json!(app.settings_ui.recent.iter().map(|rgb| format!("#{rgb:06X}")).collect::<Vec<_>>());
    let bytes = serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?;
    crate::persistence::atomic_write(&path, &bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_draft_round_trips_common_values() {
        for rgb in [0x216F62, 0xFFFFFF, 0x000000, 0xFF0000, 0x123456, 0x8CD6B5] {
            assert_eq!(ColorDraft::new(rgb).rgb(), rgb, "{rgb:06X}");
        }
    }

    #[test]
    fn hsv_clamps_out_of_range_inputs() {
        assert_eq!(hsv(-1.0, 2.0, 2.0), 0xFF0000);
        assert_eq!(hsv(0.0, -1.0, 1.0), 0xFFFFFF);
        assert_eq!(hsv(1.0, 1.0, 1.0), 0xFF0000);
    }
}
