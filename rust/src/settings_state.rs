//! Settings navigation, draft colours, finite transitions and versioned appearance storage.
use crate::{app::{App, now_ms}, ui::Control, ui_theme::{Accent, Palette, Preference}};

pub const REPOSITORY: &str = "https://github.com/offduckling4825/Sidera--screen-annotate-for-linux";
pub const LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab { #[default] Appearance, Addins, General, About }
impl Tab {
    pub const ALL: [Self; 4] = [Self::General, Self::Appearance, Self::Addins, Self::About];
    pub fn index(self) -> usize { Self::ALL.iter().position(|t| *t == self).unwrap_or(0) }
    pub fn label(self) -> &'static str { match self { Self::Appearance => "外观", Self::Addins => "加载项", Self::General => "常规", Self::About => "关于" } }
    pub fn subtitle(self) -> &'static str { match self {
        Self::Appearance => "让颜色与工具栏适合你的课堂",
        Self::Addins => "修改不当可能导致 WPS 联动异常",
        Self::General => "启动、书写与课堂环境",
        Self::About => "一起把课堂工具做得更好",
    } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog { Addins, Register, Remove, ClearLogs }
impl Dialog {
    pub fn title(self) -> &'static str { match self {
        Self::Addins => "可能导致 WPS 联动异常", Self::Register => "重新登记 WPS 加载项？",
        Self::Remove => "清除 WPS 加载项？", Self::ClearLogs => "清空运行日志？",
    } }
    pub fn body(self) -> &'static str { match self {
        Self::Addins => "不当修改加载项登记，可能影响 WPS 连接、翻页与放映同步。联动正常时无需修改。",
        Self::Register => "将重新写入 Sidera 的加载项登记，用于修复 WPS 联动。操作不当可能影响连接与放映同步；不会修改课件内容。",
        Self::Remove => "清除后，WPS 页码与放映联动将停止，屏幕批注仍可使用。可通过“重新登记”恢复；不会删除课件。",
        Self::ClearLogs => "将删除已有的运行日志，无法撤回。\n诊断功能、批注和个人设置不受影响。",
    } }
    pub fn action(self) -> &'static str { match self {
        Self::Addins => "了解风险，继续", Self::Register => "重新登记", Self::Remove => "确认清除", Self::ClearLogs => "清空日志",
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
    pub(crate) window: Option<std::rc::Rc<crate::settings_window::Window>>,
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
    pub process_view: ProcessView,
    pub recent: Vec<u32>,
    pub toast: String,
    pub toast_until: i64,
    pub theme_from: Option<(Palette, i64)>,
    pub toggle: Option<(Control, f32, i64)>,
    pub last_frame: i64,
}
impl State {
    pub fn window_size(&self) -> Option<(i32, i32)> {
        #[cfg(unix)]
        if let Some(window) = &self.window {
            return Some(window.logical_size());
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
    pub fn animating(&self) -> bool {
        let now = now_ms();
        [self.tab_at, self.opened_at, self.overlay_at, self.overlay_closing_at, self.closing_at]
            .iter().any(|start| *start > 0 && now - *start <= 230)
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
