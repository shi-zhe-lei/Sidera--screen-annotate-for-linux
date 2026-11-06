// ============================================================
// Sidera - 后端抽象（X11 / Wayland）
// 逻辑层只依赖这里的 Key/IRect 与 Rt 的分发方法
// ============================================================
use tiny_skia::Pixmap;

#[derive(Clone, Copy, Debug)]
pub enum Key {
    Up,
    Down,
    Escape,
}

/// 输入区域矩形（逻辑像素）
#[derive(Clone, Copy, Debug)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl IRect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        IRect { x, y, w, h }
    }
}

/// 后端必须提供的能力。渲染仍复用 ui::render_region，后端只负责"上屏/输入区域/注入"。
pub trait Backend: 'static {
    fn as_any(&self) -> &dyn std::any::Any;
    fn screen_size(&self) -> (i32, i32);
    /// 把渲染好的 pixmap 贴到屏幕 (x,y)
    fn present(&self, pm: &Pixmap, x: i32, y: i32);
    /// 设置输入区域；full=true 表示整屏可输入
    fn set_input_region(&self, rects: &[IRect], full: bool);
    /// 临时请求/释放文本键盘焦点（进程屏蔽输入框使用）。
    fn set_text_input(&self, _on: bool) {}
    /// 注入一次按键（翻页/退出）
    fn fake_key(&self, key: Key);
    /// 用演示软件快捷键从当前页开始放映。false 表示当前后端无法注入。
    fn start_presentation(&self) -> bool { false }
    /// 当前后端能否直接注入按键，而不触发桌面远程控制授权。
    fn can_inject_keys_without_prompt(&self) -> bool {
        false
    }
    /// 在 (x,y) 注入一次鼠标右键
    fn virtual_right_click(&self, x: i32, y: i32);
    fn can_forward_right_click(&self) -> bool { false }
    /// 全屏截图，返回 RGBA
    fn screenshot(&self) -> Option<(Vec<u8>, u32, u32)>;
    /// 后端自己处理异步截图；true 表示请求已受理。
    fn request_screenshot(&self) -> bool { false }
    fn description(&self) -> &'static str { "X11" }
    /// 是否处于办公软件全屏放映
    fn is_presentation_fullscreen(&self) -> bool;
    /// 办公软件/本覆盖层是否在前台。None 表示后端无法可靠判断。
    fn presentation_foreground(&self) -> Option<bool> { None }
    /// 是否有合成器（透明是否可用）
    fn has_compositor(&self) -> bool;
    /// XShape/输入法扩展是否可用（诊断用）
    fn shape_available(&self) -> bool {
        true
    }
    /// 抓取全局热键，返回是否成功
    fn grab_hotkey(&self) -> bool {
        false
    }
    /// 窗口尺寸变化（Wayland layer 配置回调），逻辑尺寸
    fn on_configure(&self, _w: i32, _h: i32) {}
    /// 设备缩放变化（X11 DPI / Wayland fractional scale）
    fn set_scale(&self, _scale: f64) {}
    /// 逻辑像素 → 物理像素的比例；用于把事件坐标从物理换算回逻辑。
    /// Wayland 事件本身就是逻辑坐标，故默认 1.0；X11 在 DPI 缩放下 >1。
    fn device_pixel_ratio(&self) -> f64 {
        1.0
    }
    /// 把累积的脏区域上屏（Wayland 用，X11 即时上屏故默认空实现）
    fn flush(&self) {}
}
