//! Offline light/dark coverage sheet from production painters. This never opens
//! a desktop, starts WPS, reads saved settings or runs the application loop.
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

use std::{path::Path, time::{Duration, Instant}};
use tiny_skia::{Pixmap, PixmapPaint, Transform};
use ui::{Ctx, R, Btn};
use ui_theme::{c, Preference, Theme};

fn fresh(preference: Preference) -> app::App {
    let mut a = app::App::new(Pixmap::new(1280, 900).unwrap(), 1280, 900);
    a.apply_theme(Theme { preference, system_dark: false, ..Default::default() });
    a.mode = 1;
    a.cur_color = 8;
    a.drawing_tools.set_expanded(true);
    a.drawing_tools.settle();
    a.wps_connected = true;
    a.wps_page_index = 6;
    a.wps_page_total = 24;
    a.wps_can_jump = true;
    a.wps_deck_epoch = 1;
    a.wps_navigation_ready = true;
    a
}

fn capture(a: &app::App, fonts: &text::Fonts, icon: Option<&Pixmap>, r: R) -> Pixmap {
    ui::render_region(a, fonts, icon, r.x, r.y, r.w, r.h).unwrap()
}

fn put(dest: &mut Pixmap, src: &Pixmap, x: i32, y: i32) {
    dest.draw_pixmap(x, y, src.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: render_themes OUTPUT_DIRECTORY");
    let out = Path::new(&out);
    std::fs::create_dir_all(out).unwrap();
    let fonts = text::Fonts::load();
    let original = Pixmap::load_png("../sidera.png").ok();
    let resize_icon = |size| original.as_ref().map(|original| {
        let mut icon = Pixmap::new(size, size).unwrap();
        icon.draw_pixmap(0, 0, original.as_ref(), &PixmapPaint {
            quality: tiny_skia::FilterQuality::Bicubic, ..Default::default()
        }, Transform::from_scale(size as f32 / original.width() as f32, size as f32 / original.height() as f32), None);
        icon
    });
    let icon = resize_icon(44);
    let intro_icon = resize_icon(84);

    if std::env::args().nth(2).as_deref() == Some("--settings") {
        for preference in [Preference::Light, Preference::Dark] {
            for (w, h) in [(980, 512), (640, 360), (320, 240), (1280, 900)] {
                let mut a = app::App::new(Pixmap::new(w, h).unwrap(), w as i32, h as i32);
                a.apply_theme(Theme { preference, system_dark: false, ..Default::default() });
                a.settings_open = true;
                for page in 0..ui::settings_layout(&a).pages {
                    a.settings_page = page;
                    capture(&a, &fonts, icon.as_ref(), ui::settings_rect(&a))
                        .save_png(out.join(format!("{}-{w}x{h}-{}.png", preference.key(), page + 1))).unwrap();
                }
            }
        }
        return;
    }

    for preference in [Preference::Light, Preference::Dark] {
        let theme = Theme { preference, system_dark: false, ..Default::default() };
        let palette = theme.palette();
        let mut sheet = Pixmap::new(1760, 1100).unwrap();
        sheet.fill(c(palette.surface));
        let mut add = |name: &str, title: &str, image: Pixmap, x: i32, y: i32| {
            image.save_png(out.join(format!("{}-{name}.png", preference.key()))).unwrap();
            fonts.draw(&mut sheet, title, x as f32, y as f32 - 14.0, 21.0,
                c(palette.text), Transform::identity());
            put(&mut sheet, &image, x, y);
        };
        let mut a = fresh(preference);
        a.settings_open = true;
        add("settings", "设置 · 三档主题与触屏控件", capture(&a, &fonts, icon.as_ref(), ui::settings_rect(&a)), 24, 48);

        let mut a = fresh(preference);
        let now = Instant::now();
        a.launch = launch::Launch::begin("GNOME 原生 Wayland · 1280 × 900".into(), true, true, now);
        a.launch.report_link(launch::LinkState::Waiting, now);
        add("startup", "启动准备", splash_paint::draw(&fonts, intro_icon.as_ref(), &a.launch, 560, 448, palette), 516, 48);

        let mut a = fresh(preference);
        a.pen_popup_visible = true;
        add("pen", "画笔 · 原色与笔尖", capture(&a, &fonts, None, ui::pen_popup_rect(&a)), 1110, 48);
        a.pen_popup_visible = false;
        a.eraser_popup_visible = true;
        add("eraser", "橡皮 · 清空", capture(&a, &fonts, None, ui::eraser_popup_rect(&a)), 1400, 48);
        a.eraser_popup_visible = false;
        a.more_menu_open = true;
        add("more", "更多操作", capture(&a, &fonts, None, ui::more_menu_rect(&a)), 1400, 300);

        let mut a = fresh(preference);
        a.page_picker = Some(page_picker::PagePicker::new(6, 24, page_picker::Source::Wps(1)));
        add("wheel", "页码拨轮", capture(&a, &fonts, None, ui::page_picker_rect(&a)), 1110, 398);

        let mut a = fresh(preference);
        a.show_diag = true;
        a.diag_text = "Sidera 3.0 · 课堂定制分支\n桌面：GNOME 原生 Wayland\n屏幕：1280 × 900\n界面：静态预览\n联动：等待 WPS 接入\n此图不代表桌面验收结果".into();
        add("diagnostics", "诊断与关闭", capture(&a, &fonts, None, ui::diag_rect(&a)), 516, 554);
        a.show_diag = false;
        a.confirm_quit = true;
        add("confirm", "退出确认", capture(&a, &fonts, None, ui::confirm_rect(&a)), 1110, 896);

        let mut strip = Pixmap::new(700, 740).unwrap();
        strip.fill(c(palette.surface));
        for (column, title) in ["光标", "画笔", "橡皮", "禁用", "拖动", "折叠", "收起", "按下"].into_iter().enumerate() {
            let mut a = fresh(preference);
            a.sidebar_alpha = 220;
            a.sb_y = 0;
            if column < 3 { a.mode = column as i32; }
            if column == 3 { a.whiteboard = true; }
            if column == 4 { a.sidebar_drag = Some(ui::SidebarDragFeedback { right: false }); }
            if column == 5 { a.drawing_tools.set_expanded(false); a.drawing_tools.settle(); }
            if column == 6 { a.set_sidebar_collapsed(true); a.sidebar_anim_start = None; }
            if column == 7 { a.press = Some(ui::PressFeedback::new(ui::Control::Sidebar(Btn::Pen, false))); }
            let image = capture(&a, &fonts, None, R { x: 0, y: 0, w: 68, h: 660 });
            let x = column as i32 * 86 + 8;
            fonts.draw(&mut strip, title, x as f32, 28.0, 17.0, c(palette.text), Transform::identity());
            put(&mut strip, &image, x, 44);
        }
        strip.save_png(out.join(format!("{}-toolbars.png", preference.key()))).unwrap();

        // Presentation states and transition samples use production animation.
        let mut actions = Pixmap::new(400, 190).unwrap();
        actions.fill(c(palette.surface));
        for (i, (action, title)) in [(ui::PresentationAction::Escape, "Esc"),
            (ui::PresentationAction::Start, "可放映"), (ui::PresentationAction::End, "放映中")].into_iter().enumerate() {
            let mut a = fresh(preference);
            a.presentation_action = action;
            a.presentation_transition.set_action_at(action, now);
            a.presentation_transition.settle();
            let r = ui::sidebar_buttons(&a, false).into_iter().find(|(b, _)| *b == Btn::Exit).unwrap().1;
            fonts.draw(&mut actions, title, (i * 130 + 18) as f32, 27.0, 18.0, c(palette.text), Transform::identity());
            put(&mut actions, &capture(&a, &fonts, None, r), i as i32 * 130 + 25, 46);
            a.presentation_transition.set_action_at(ui::PresentationAction::Start, now);
            a.presentation_transition.advance_at(now + Duration::from_millis(80));
            put(&mut actions, &capture(&a, &fonts, None, r), i as i32 * 130 + 25, 108);
        }
        add("actions", "放映状态 · 切换分帧", actions, 24, 822);
        sheet.save_png(out.join(format!("{}-panels.png", preference.key()))).unwrap();

        for (background, name) in [(0, "green"), (1, "white")] {
            let mut a = fresh(preference);
            a.whiteboard = true;
            a.whiteboard_bg_index = background;
            paint::stroke_tapered(&mut a, (360, 420), (820, 420), 8, 8);
            a.pen_popup_visible = true;
            let image = capture(&a, &fonts, None, R { x: 0, y: 0, w: 1280, h: 900 });
            image.save_png(out.join(format!("{}-board-{name}.png", preference.key()))).unwrap();
        }
        // Readability against an arbitrary bright desktop remains independent
        // of the dark UI, just as in the actual transparent overlay.
        let mut a = fresh(preference);
        a.popup_on_right = true;
        a.pen_popup_visible = true;
        let overlay = capture(&a, &fonts, None, R { x: 0, y: 0, w: 1280, h: 900 });
        let mut classroom = Pixmap::new(1280, 900).unwrap();
        classroom.fill(c(if theme.dark() { 0x162C23 } else { 0xF3F6EE }));
        let mut ctx = Ctx { pm: &mut classroom, tr: Transform::identity(), palette };
        ctx.text(&fonts, "课堂练习", 280.0, 280.0, 42.0, c(palette.text));
        ctx.text(&fonts, "界面随主题变化，笔迹保留原色。", 280.0, 340.0, 25.0, c(palette.muted));
        put(&mut classroom, &overlay, 0, 0);
        classroom.save_png(out.join(format!("{}-classroom.png", preference.key()))).unwrap();
    }
}
