//! Offline artwork preview using the production painters. No desktop backend,
//! application event loop, WPS connection, input injection or test runner.
#![allow(dead_code)]
#[path = "../src/app.rs"] mod app;
#[path = "../src/persistence.rs"] mod persistence;
#[path = "../src/process_block.rs"] mod process_block;
#[cfg(unix)]
#[path = "../src/instance.rs"] mod instance;
#[cfg(not(unix))]
mod instance {}
#[path = "../src/page_cache.rs"] mod page_cache;
#[path = "../src/clear_history.rs"] mod clear_history;
#[path = "../src/paint.rs"] mod paint;
#[path = "../src/text.rs"] mod text;
#[path = "../src/toolbar.rs"] mod toolbar;
#[path = "../src/page_picker.rs"] mod page_picker;
#[path = "../src/ui.rs"] mod ui;
#[path = "../src/ui_icons.rs"] mod ui_icons;
#[path = "../src/ui_theme.rs"] mod ui_theme;
#[path = "../src/splash_paint.rs"] mod splash_paint;
#[path = "../src/launch.rs"] mod launch;
mod wps { pub fn wps_log(_: &str) {} }
mod diag_log { pub fn info(_: &str) {} pub fn warn(_: &str) {} }

use std::path::Path;
use tiny_skia::{Pixmap, PixmapPaint, Transform};
use app::App;
use text::Fonts;
use ui::{Btn, Ctx, R};
use ui_theme::c;
const TEXT: u32 = ui_theme::LIGHT.text;
const MUTED: u32 = ui_theme::LIGHT.muted;

fn fresh() -> App {
    let mut a = App::new(Pixmap::new(1280, 900).unwrap(), 1280, 900);
    a.mode = 1;
    a.drawing_tools.set_expanded(true);
    a.drawing_tools.settle();
    a.wps_connected = true;
    a.wps_page_index = 6;
    a.wps_page_total = 24;
    a.wps_navigation_ready = true;
    a.wps_can_jump = true;
    a.wps_deck_epoch = 1;
    set_presentation(&mut a, ui::PresentationAction::Start);
    a
}

fn set_presentation(a: &mut App, action: ui::PresentationAction) {
    a.presentation_action = action;
    a.presentation_transition.set_action_at(action, std::time::Instant::now());
    a.presentation_transition.settle();
}

fn label(pm: &mut Pixmap, fonts: &Fonts, text: &str, x: f32, y: f32, size: f32) {
    fonts.draw(pm, text, x, y, size, c(TEXT), Transform::identity());
}

fn crop(a: &App, fonts: &Fonts, icon: Option<&Pixmap>, r: R) -> Pixmap {
    ui::render_region(a, fonts, icon, r.x, r.y, r.w, r.h).unwrap()
}

fn paste(dest: &mut Pixmap, src: &Pixmap, x: f32, y: f32, scale: f32) {
    dest.draw_pixmap(0, 0, src.as_ref(), &PixmapPaint {
        quality: tiny_skia::FilterQuality::Bilinear, ..Default::default()
    },
        Transform::from_translate(x, y).pre_scale(scale, scale), None);
}

fn icon(size: u32) -> Option<Pixmap> {
    let image = image::open("../sidera.png").ok()?.to_rgba8();
    let scaled = image::imageops::resize(&image, size, size, image::imageops::FilterType::Lanczos3);
    let mut pm = Pixmap::new(size, size)?;
    for (dst, src) in pm.pixels_mut().iter_mut().zip(scaled.pixels()) {
        *dst = tiny_skia::ColorU8::from_rgba(src[0], src[1], src[2], src[3]).premultiply();
    }
    Some(pm)
}

fn scene(fonts: &Fonts, dark: bool) -> Pixmap {
    let mut pm = Pixmap::new(1280, 900).unwrap();
    pm.fill(c(if dark { 0x193E35 } else { 0xF7F8F3 }));
    let ink = c(if dark { 0xE6F1E7 } else { TEXT });
    fonts.draw(&mut pm, "课堂练习", 380.0, 240.0, 42.0, ink, Transform::identity());
    fonts.draw(&mut pm, "观察图形，想一想还可以怎样解答？", 380.0, 294.0, 25.0, ink, Transform::identity());
    let mut ctx = Ctx { palette: ui_theme::LIGHT, pm: &mut pm, tr: Transform::identity() };
    ctx.stroke_line((460.0, 565.0), (740.0, 565.0), ink, 2.2);
    ctx.stroke_line((460.0, 565.0), (600.0, 355.0), ink, 2.2);
    ctx.stroke_line((600.0, 355.0), (740.0, 565.0), ink, 2.2);
    ctx.text(fonts, "A", 590.0, 338.0, 24.0, ink);
    ctx.text(fonts, "B", 438.0, 592.0, 24.0, ink);
    ctx.text(fonts, "C", 744.0, 592.0, 24.0, ink);
    pm
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: cargo run --example render_ui -- OUTPUT_DIRECTORY");
    let out = Path::new(&out);
    std::fs::create_dir_all(out).unwrap();
    let mut fonts = Fonts::load();
    // Optional font copied from the target machine, useful on minimal builders
    // without CJK fonts. This changes only the artwork renderer, not the app.
    if let Some(path) = std::env::args().nth(2) {
        let data = std::fs::read(&path).expect("read preview font");
        fonts.list.insert(0, ab_glyph::FontArc::try_from_vec(data).expect("load preview font"));
    }
    let icon = icon(44);
    let intro_icon = self::icon(84);
    let mut startup_states = Pixmap::new(1200, 1060).unwrap();
    startup_states.fill(c(0xE8EDE6));
    for (index, state) in [launch::LinkState::Preparing, launch::LinkState::Waiting,
        launch::LinkState::Connected, launch::LinkState::Unavailable].into_iter().enumerate() {
        let start = std::time::Instant::now();
        let mut intro = launch::Launch::begin("GNOME 原生 Wayland · 1280 × 900".into(), true, true, start);
        intro.report_link(state, start);
        let card = splash_paint::draw(&fonts, intro_icon.as_ref(), &intro, 560, 448, ui_theme::LIGHT);
        let (x, y) = (20.0 + (index % 2) as f32 * 600.0, 48.0 + (index / 2) as f32 * 518.0);
        label(&mut startup_states, &fonts, state.label(), x + 6.0, y - 16.0, 20.0);
        paste(&mut startup_states, &card, x, y, 1.0);
        if state == launch::LinkState::Waiting { card.save_png(out.join("classroom-startup.png")).unwrap(); }
    }
    startup_states.save_png(out.join("classroom-startup-states.png")).unwrap();
    let mut reveal = Pixmap::new(1200, 1040).unwrap();
    reveal.fill(c(0xE8EDE6));
    label(&mut reveal, &fonts, "启动时从中间向上下展开 · 按钮完整显现", 26.0, 40.0, 27.0);
    for (row, right) in [false, true].into_iter().enumerate() {
        for (column, ms) in [1240, 1280, 1340, 1420, 1520, 1620].into_iter().enumerate() {
            let mut a = fresh();
            a.mode = 0; a.drawing_tools.set_expanded(false); a.drawing_tools.settle();
            let start = std::time::Instant::now();
            a.launch = launch::Launch::begin("GNOME 原生 Wayland · 1280 × 900".into(), true, true, start);
            a.launch.report_link(launch::LinkState::Waiting, start);
            a.launch.advance_at(start + std::time::Duration::from_millis(ms));
            let sb = ui::sidebar_rect(&a, right);
            let r = R { x: sb.x.saturating_sub(3), y: sb.y - 3, w: sb.w + 6, h: sb.h + 6 };
            let x = 28.0 + column as f32 * 197.0;
            let y = 90.0 + row as f32 * 470.0;
            label(&mut reveal, &fonts, &format!("{} · {} ms", if right { "右侧" } else { "左侧" }, ms - 1240), x, y, 18.0);
            paste(&mut reveal, &crop(&a, &fonts, intro_icon.as_ref(), r), x + 35.0, y + 18.0, 1.0);
        }
    }
    reveal.save_png(out.join("classroom-startup-reveal.png")).unwrap();
    let mut drag_states = Pixmap::new(960, 1040).unwrap();
    drag_states.fill(c(0xE8EDE6));
    label(&mut drag_states, &fonts, "拖拽交互反馈", 26.0, 44.0, 28.0);
    for (row, right) in [false, true].into_iter().enumerate() {
        for (column, title) in ["常态", "按住", "拖动中", "松开"].into_iter().enumerate() {
            let mut a = fresh();
            a.mode = 0;
            a.drawing_tools.set_expanded(false); a.drawing_tools.settle();
            a.sb_y = if column >= 2 { 195 } else { 180 };
            if column == 1 || column == 2 {
                a.sidebar_drag = Some(ui::SidebarDragFeedback {
                    right,
                });
            }
            let x = 24.0 + column as f32 * 234.0;
            let y = 90.0 + row as f32 * 470.0;
            label(&mut drag_states, &fonts, &format!("{} · {title}", if right { "右侧" } else { "左侧" }), x, y, 20.0);
            let r = R { x: if right { a.screen_w - 174 } else { 0 }, y: 150, w: 174, h: 430 };
            paste(&mut drag_states, &crop(&a, &fonts, None, r), x, y + 16.0, 1.0);
        }
    }
    drag_states.save_png(out.join("classroom-drag-feedback.png")).unwrap();
    let mut drawer = Pixmap::new(980, 760).unwrap();
    drawer.fill(c(0xE8EDE6));
    label(&mut drawer, &fonts, "绘图工具按需展开", 30.0, 44.0, 30.0);
    for (column, title) in ["默认精简", "展开中", "画笔已选中", "清除后", "白板清除后"].iter().enumerate() {
        let mut a = fresh();
        a.mode = 0;
        a.drawing_tools.set_expanded(false);
        a.drawing_tools.settle();
        let start = std::time::Instant::now();
        if column == 1 || column == 2 {
            a.mode = 1;
            a.drawing_tools.set_expanded_at(true, start);
            a.drawing_tools.advance_at(start + std::time::Duration::from_millis(if column == 1 { 55 } else { 220 }));
        }
        if column == 4 {
            a.toggle_whiteboard();
            a.clear_current_strokes();
            a.drawing_tools.settle();
        }
        let x = 26.0 + column as f32 * 194.0;
        label(&mut drawer, &fonts, title, x, 93.0, 22.0);
        let r = R { x: a.sb_x_left(), y: a.sb_y, w: a.sb_width(), h: app::sb_height_of(a.sb_scale) };
        paste(&mut drawer, &crop(&a, &fonts, None, r), x + 28.0, 119.0, 1.2);
    }
    drawer.save_png(out.join("classroom-drawer.png")).unwrap();

    // Fixed animation-time snapshots near the bottom edge; no desktop or input.
    let mut bottom = Pixmap::new(940, 620).unwrap();
    bottom.fill(c(0xE8EDE6));
    label(&mut bottom, &fonts, "贴底展开时向上让位，完整留在屏幕内", 26.0, 44.0, 28.0);
    for (i, ms) in [0, 35, 70, 120, 220].into_iter().enumerate() {
        let mut a = fresh();
        a.drawing_tools.set_expanded(false); a.drawing_tools.settle();
        a.sb_y = a.screen_h - a.sb_height();
        let start = std::time::Instant::now();
        a.drawing_tools.set_expanded_at(true, start);
        a.drawing_tools.advance_at(start + std::time::Duration::from_millis(ms));
        let mut r = ui::sidebar_transition_rect(&a, false);
        r.h = r.h.min(a.screen_h - r.y);
        let x = 44.0 + i as f32 * 182.0;
        label(&mut bottom, &fonts, &format!("{ms} ms"), x, 88.0, 20.0);
        paste(&mut bottom, &crop(&a, &fonts, None, r), x, 106.0, 1.0);
    }
    bottom.save_png(out.join("classroom-drawer-bottom.png")).unwrap();
    // Static comparison of real toolbar controls, without input or a backend.
    let mut selection = Pixmap::new(720, 410).unwrap();
    selection.fill(c(0xE8EDE6));
    label(&mut selection, &fonts, "工具选中状态", 30.0, 43.0, 28.0);
    for (column, title) in ["光标选中", "画笔选中", "橡皮选中", "白板模式"].iter().enumerate() {
        let mut a = fresh();
        a.mode = column.min(2) as i32;
        if column == 3 { a.toggle_whiteboard(); }
        let x = 36.0 + column as f32 * 174.0;
        label(&mut selection, &fonts, title, x, 87.0, 20.0);
        for (row, button) in [Btn::Cursor, Btn::Pen, Btn::Eraser, Btn::Whiteboard].iter().enumerate() {
            let r = ui::sidebar_buttons(&a, false).into_iter().find(|(b, _)| b == button).unwrap().1;
            paste(&mut selection, &crop(&a, &fonts, None, r), x + 18.0, 108.0 + row as f32 * 70.0, 1.5);
        }
    }
    selection.save_png(out.join("classroom-selection.png")).unwrap();

    let mut counters = Pixmap::new(840, 340).unwrap();
    counters.fill(c(0xE8EDE6));
    label(&mut counters, &fonts, "上下翻页之间的页码", 30.0, 42.0, 28.0);
    for (column, title) in ["WPS 联动", "未联动", "白板", "长文档", "小工具栏"].iter().enumerate() {
        let mut a = fresh();
        match column {
            1 => a.wps_connected = false,
            2 => { a.toggle_whiteboard(); a.current_slide = 3; },
            3 | 4 => { a.wps_page_index = 108; a.wps_page_total = 240; },
            _ => {},
        }
        if column == 4 { a.sb_scale = 0.6; }
        let buttons = ui::sidebar_buttons(&a, false);
        let prev = buttons.iter().find(|(b, _)| *b == Btn::Prev).unwrap().1;
        let next = buttons.iter().find(|(b, _)| *b == Btn::Next).unwrap().1;
        let sidebar = ui::sidebar_rect(&a, false);
        let r = R { x: sidebar.x, y: prev.y - 5, w: sidebar.w, h: next.y + next.h - prev.y + 10 };
        let x = 26.0 + column as f32 * 164.0;
        label(&mut counters, &fonts, title, x, 86.0, 20.0);
        paste(&mut counters, &crop(&a, &fonts, None, r), x + 24.0, 110.0, 1.6);
    }
    counters.save_png(out.join("classroom-pages.png")).unwrap();

    let mut transitions = Pixmap::new(1060, 950).unwrap();
    transitions.fill(c(0xE8EDE6));
    label(&mut transitions, &fonts, "放映按钮  /  240 毫秒淡出淡入", 32.0, 52.0, 30.0);
    let actions = [ui::PresentationAction::Escape, ui::PresentationAction::Start, ui::PresentationAction::End];
    let names = ["Esc", "开始放映", "结束放映"];
    for (column, ms) in [0, 60, 120, 180, 240].into_iter().enumerate() {
        label(&mut transitions, &fonts, &format!("{ms} ms"), 340.0 + column as f32 * 140.0, 98.0, 19.0);
    }
    for (row, (from, to)) in [(0, 1), (1, 2), (2, 1), (1, 0), (2, 0), (0, 2)].into_iter().enumerate() {
        let y = 130.0 + row as f32 * 132.0;
        label(&mut transitions, &fonts, &format!("{} → {}", names[from], names[to]), 32.0, y + 43.0, 21.0);
        for (column, ms) in [0, 60, 120, 180, 240].into_iter().enumerate() {
            let mut a = fresh();
            set_presentation(&mut a, actions[from]);
            let start = std::time::Instant::now();
            a.presentation_action = actions[to];
            a.presentation_transition.set_action_at(actions[to], start);
            a.presentation_transition.advance_at(start + std::time::Duration::from_millis(ms));
            let r = ui::sidebar_buttons(&a, false).into_iter().find(|(b, _)| *b == Btn::Exit).unwrap().1;
            paste(&mut transitions, &crop(&a, &fonts, None, r), 340.0 + column as f32 * 140.0, y, 2.0);
        }
    }
    transitions.save_png(out.join("classroom-presentation-transition.png")).unwrap();

    let mut wheels = Pixmap::new(1160, 520).unwrap();
    wheels.fill(c(0xE8EDE6));
    label(&mut wheels, &fonts, "页码拨轮  /  选择后确认跳转", 28.0, 44.0, 30.0);
    for (column, (title, page, total, fraction)) in [
        ("首页边界", 1, 24, 0.0), ("选中与相邻页", 12, 24, 0.0),
        ("滚动途中", 108, 240, 0.38), ("白板末页", 10, 10, 0.0),
    ].into_iter().enumerate() {
        let mut a = fresh();
        a.whiteboard = column == 3;
        let source = if a.whiteboard { page_picker::Source::Whiteboard } else { page_picker::Source::Wps(1) };
        let mut picker = page_picker::PagePicker::new(page, total, source);
        picker.position += fraction;
        a.page_picker = Some(picker);
        label(&mut wheels, &fonts, title, 24.0 + column as f32 * 286.0, 84.0, 20.0);
        let r = ui::page_picker_rect(&a);
        paste(&mut wheels, &crop(&a, &fonts, None, r), 24.0 + column as f32 * 286.0, 108.0, 1.0);
    }
    wheels.save_png(out.join("classroom-page-wheel.png")).unwrap();
    for right in [false, true] {
        let mut a = fresh();
        a.mode = 0; a.popup_on_right = right;
        a.page_picker = Some(page_picker::PagePicker::new(6, 24, page_picker::Source::Wps(1)));
        let mut pm = scene(&fonts, false);
        paste(&mut pm, &ui::render_region(&a, &fonts, None, 0, 0, 1280, 900).unwrap(), 0.0, 0.0, 1.0);
        pm.save_png(out.join(if right { "classroom-page-wheel-right.png" } else { "classroom-page-wheel-left.png" })).unwrap();
    }

    let mut board = Pixmap::new(1440, 1120).unwrap();
    board.fill(c(0xE8EDE6));
    label(&mut board, &fonts, "Sidera  /  课堂浅色界面", 44.0, 64.0, 40.0);
    fonts.draw(&mut board, "暖白面板 · 蓝绿选中态 · 清晰的触摸反馈", 46.0, 103.0, 22.0, c(MUTED), Transform::identity());
    label(&mut board, &fonts, "工具栏与按下状态", 44.0, 151.0, 21.0);
    for i in 0..3 {
        let mut a = fresh();
        a.mode = if i == 1 { 2 } else { 1 };
        a.cur_eraser = 2;
        set_presentation(&mut a, [ui::PresentationAction::Start, ui::PresentationAction::End, ui::PresentationAction::Escape][i]);
        if i == 2 { a.press = Some(ui::PressFeedback::new(ui::Control::Sidebar(Btn::Pen, false))); }
        let pm = crop(&a, &fonts, None, ui::sidebar_rect(&a, false));
        paste(&mut board, &pm, 48.0 + i as f32 * 76.0, 174.0, 1.0);
    }
    label(&mut board, &fonts, "画笔与橡皮", 300.0, 151.0, 21.0);
    let mut a = fresh();
    a.pen_popup_visible = true;
    paste(&mut board, &crop(&a, &fonts, None, ui::pen_popup_rect(&a)), 300.0, 174.0, 1.0);
    a.pen_popup_visible = false; a.eraser_popup_visible = true; a.cur_eraser = 1;
    paste(&mut board, &crop(&a, &fonts, None, ui::eraser_popup_rect(&a)), 582.0, 174.0, 1.0);
    a.eraser_popup_visible = false; a.more_menu_open = true;
    paste(&mut board, &crop(&a, &fonts, None, ui::more_menu_rect(&a)), 582.0, 384.0, 1.0);

    label(&mut board, &fonts, "笔尖跟随颜色（图标放大展示）", 300.0, 540.0, 21.0);
    for i in 0..10 {
        let mut a = fresh(); a.cur_color = i;
        let r = ui::sidebar_buttons(&a, false).into_iter().find(|(b, _)| *b == Btn::Pen).unwrap().1;
        paste(&mut board, &crop(&a, &fonts, None, r), 300.0 + i as f32 * 54.0, 562.0, 1.35);
    }
    label(&mut board, &fonts, "小 / 中 / 大 · 点击范围保持一致", 300.0, 658.0, 21.0);
    for i in 0..3 {
        let mut a = fresh(); a.mode = 2; a.cur_eraser = i;
        let r = ui::sidebar_buttons(&a, false).into_iter().find(|(b, _)| *b == Btn::Eraser).unwrap().1;
        paste(&mut board, &crop(&a, &fonts, None, r), 300.0 + i as f32 * 70.0, 680.0, 1.5);
    }
    label(&mut board, &fonts, "更多窗口沿用相同的视觉规则", 300.0, 831.0, 21.0);
    let mut a = fresh(); a.confirm_quit = true;
    paste(&mut board, &crop(&a, &fonts, None, ui::confirm_rect(&a)), 300.0, 858.0, 1.0);
    label(&mut board, &fonts, "设置", 908.0, 151.0, 21.0);
    let mut a = fresh(); a.settings_open = true;
    paste(&mut board, &crop(&a, &fonts, icon.as_ref(), ui::settings_rect(&a)), 908.0, 174.0, 1.0);
    a.set_scale(1.5);
    crop(&a, &fonts, icon.as_ref(), ui::settings_rect(&a)).save_png(out.join("classroom-settings.png")).unwrap();
    let large_icon = self::icon(84);
    let start = std::time::Instant::now();
    let mut intro = launch::Launch::begin("GNOME 原生 Wayland · 1280 × 900".into(), true, true, start);
    intro.report_link(launch::LinkState::Waiting, start);
    paste(&mut board, &splash_paint::draw(&fonts, large_icon.as_ref(), &intro, 560, 448, ui_theme::LIGHT), 990.0, 882.0, 0.46);
    board.save_png(out.join("classroom-overview.png")).unwrap();

    for dark in [false, true] {
        let mut a = fresh(); a.pen_popup_visible = !dark; a.eraser_popup_visible = dark;
        a.mode = if dark { 2 } else { 1 }; a.cur_eraser = 2;
        let mut pm = scene(&fonts, dark);
        paste(&mut pm, &ui::render_region(&a, &fonts, icon.as_ref(), 0, 0, 1280, 900).unwrap(), 0.0, 0.0, 1.0);
        pm.save_png(out.join(if dark { "classroom-dark-background.png" } else { "classroom-light-background.png" })).unwrap();
    }
    let mut a = fresh(); a.show_diag = true;
    let mut tucked = fresh(); tucked.mode = 0; tucked.collapsed = true;
    let mut collapsed_scene = scene(&fonts, false);
    paste(&mut collapsed_scene, &ui::render_region(&tucked, &fonts, None, 0, 0, 1280, 900).unwrap(), 0.0, 0.0, 1.0);
    collapsed_scene.save_png(out.join("classroom-collapsed.png")).unwrap();
    for right in [false, true] {
        let tab = ui::sidebar_rect(&tucked, right);
        crop(&tucked, &fonts, None, tab).save_png(out.join(if right { "classroom-tab-right.png" } else { "classroom-tab-left.png" })).unwrap();
    }
    a.diag_text = "Sidera\n桌面：GNOME 原生 Wayland\n分辨率：1920 × 1080\nWPS 联动：已启用\n一行很长的诊断信息用于静态排版预览：这段文字不应越过面板右边缘或覆盖旁边的桌面内容。".into();
    crop(&a, &fonts, None, ui::diag_rect(&a)).save_png(out.join("classroom-diagnostics.png")).unwrap();
    // 0.6 / 1.0 / 1.4 and fractional physical scales: static rasterization only.
    for (index, scale, dpi) in [(0, 0.6, 1.0), (1, 1.0, 1.25), (2, 1.4, 2.0)] {
        let mut a = fresh(); a.sb_scale = scale; a.set_scale(dpi); a.mode = 2; a.cur_eraser = 2;
        crop(&a, &fonts, None, ui::sidebar_rect(&a, false))
            .save_png(out.join(format!("classroom-toolbar-scale-{index}.png"))).unwrap();
    }
    println!("Static UI artwork written to {}", out.display());
}
