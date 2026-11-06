//! Minimal StatusNotifierItem + DBusMenu tray integration.
//! Uses the existing zbus dependency so the overlay stays toolkit-free.
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Settings,
    Toggle,
    Whiteboard,
    Show,
    Quit,
}

static ACTIONS: Mutex<VecDeque<TrayAction>> = Mutex::new(VecDeque::new());

pub fn poll() -> Option<TrayAction> {
    ACTIONS.lock().ok().and_then(|mut queue| queue.pop_front())
}

fn push(action: TrayAction) {
    if let Ok(mut queue) = ACTIONS.lock() {
        queue.push_back(action);
    }
}

type Props = HashMap<String, OwnedValue>;
type Child = (i32, Props, Vec<OwnedValue>);
type Layout = (i32, Props, Vec<Child>);
type ToolTip = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

fn owned(value: Value<'_>) -> OwnedValue {
    OwnedValue::try_from(value).expect("owned tray value")
}

fn item_props(label: &str) -> HashMap<String, OwnedValue> {
    let mut props = HashMap::new();
    props.insert("label".into(), owned(Value::from(label)));
    props.insert("enabled".into(), owned(Value::from(true)));
    props.insert("visible".into(), owned(Value::from(true)));
    props.insert("type".into(), owned(Value::from("standard")));
    props
}

fn layout() -> (u32, Layout) {
    let root = (
        0,
        HashMap::new(),
        vec![
            (1, item_props("打开设置"), Vec::new()),
            (2, item_props("切换批注/光标"), Vec::new()),
            (3, item_props("白板"), Vec::new()),
            (4, item_props("显示 Sidera"), Vec::new()),
            (5, item_props("退出"), Vec::new()),
        ],
    );
    (1, root)
}

struct Sni;

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Sni {
    #[zbus(property, name = "Category")]
    fn category(&self) -> String {
        "ApplicationStatus".into()
    }
    #[zbus(property, name = "Id")]
    fn id(&self) -> String {
        "sidera".into()
    }
    #[zbus(property, name = "Title")]
    fn title(&self) -> String {
        "Sidera".into()
    }
    #[zbus(property, name = "Status")]
    fn status(&self) -> String {
        "Active".into()
    }
    #[zbus(property, name = "WindowId")]
    fn window_id(&self) -> u32 {
        0
    }
    #[zbus(property, name = "IconName")]
    fn icon_name(&self) -> String {
        "sidera".into()
    }
    #[zbus(property, name = "IconPixmap")]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }
    #[zbus(property, name = "OverlayIconName")]
    fn overlay_icon_name(&self) -> String {
        String::new()
    }
    #[zbus(property, name = "AttentionIconName")]
    fn attention_icon_name(&self) -> String {
        String::new()
    }
    #[zbus(property, name = "AttentionIconPixmap")]
    fn attention_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }
    #[zbus(property, name = "AttentionMovieName")]
    fn attention_movie_name(&self) -> String {
        String::new()
    }
    #[zbus(property, name = "ToolTip")]
    fn tool_tip(&self) -> ToolTip {
        (
            "sidera".into(),
            Vec::new(),
            "Sidera".into(),
            "课堂批注".into(),
        )
    }
    #[zbus(property, name = "ItemIsMenu")]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property, name = "Menu")]
    fn menu(&self) -> OwnedObjectPath {
        "/MenuBar".try_into().expect("menu path")
    }

    #[zbus(name = "ContextMenu")]
    fn context_menu(&self, _x: i32, _y: i32) {}
    #[zbus(name = "Activate")]
    fn activate(&self, _x: i32, _y: i32) {
        push(TrayAction::Settings);
    }
    #[zbus(name = "SecondaryActivate")]
    fn secondary_activate(&self, _x: i32, _y: i32) {
        push(TrayAction::Settings);
    }
    #[zbus(name = "Scroll")]
    fn scroll(&self, _delta: i32, _orientation: String) {}
}

struct Menu;

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> String {
        "ltr".into()
    }
    #[zbus(property)]
    fn status(&self) -> String {
        "normal".into()
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(name = "GetLayout")]
    fn get_layout(
        &self,
        _parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, Layout) {
        layout()
    }

    #[zbus(name = "GetGroupProperties")]
    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let labels = [
            (1, "打开设置"),
            (2, "切换批注/光标"),
            (3, "白板"),
            (4, "显示 Sidera"),
            (5, "退出"),
        ];
        labels
            .into_iter()
            .filter(|(id, _)| ids.is_empty() || ids.contains(id))
            .map(|(id, label)| (id, item_props(label)))
            .collect()
    }

    #[zbus(name = "GetProperty")]
    fn get_property(&self, id: i32, name: String) -> OwnedValue {
        let label = match id {
            1 => "打开设置",
            2 => "切换批注/光标",
            3 => "白板",
            4 => "显示 Sidera",
            5 => "退出",
            _ => "",
        };
        item_props(label)
            .remove(&name)
            .unwrap_or_else(|| owned(Value::from(false)))
    }

    #[zbus(name = "Event")]
    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        push(match id {
            1 => TrayAction::Settings,
            2 => TrayAction::Toggle,
            3 => TrayAction::Whiteboard,
            4 => TrayAction::Show,
            5 => TrayAction::Quit,
            _ => return,
        });
    }

    #[zbus(name = "EventGroup")]
    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        for (id, event_id, data, timestamp) in events {
            self.event(id, event_id, data, timestamp);
        }
        Vec::new()
    }

    #[zbus(name = "AboutToShow")]
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }
    #[zbus(name = "AboutToShowGroup")]
    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), ids)
    }
}

pub fn start() {
    std::thread::Builder::new()
        .name("sidera-tray".into())
        .spawn(|| {
            if let Err(error) = run() {
                crate::diag_log::warn(&format!("托盘菜单不可用：{error}"));
            }
        })
        .ok();
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let connection = zbus::blocking::Connection::session()?;
    connection.object_server().at("/StatusNotifierItem", Sni)?;
    connection.object_server().at("/MenuBar", Menu)?;
    let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    let _ = connection.request_name(name.as_str());
    let watcher = zbus::blocking::Proxy::new(
        &connection,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )?;
    let bus = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?;
    let mut registered_owner: Option<String> = None;
    let mut logged_failure = false;
    loop {
        // Quickshell/waybar may start after Sidera. Retry when the watcher
        // appears, and re-register if its D-Bus owner changes after a restart.
        let owner = bus
            .call::<_, _, String>("GetNameOwner", &("org.kde.StatusNotifierWatcher",))
            .ok();
        match owner {
            Some(owner) if registered_owner.as_deref() != Some(owner.as_str()) => {
                match watcher.call::<_, _, ()>(
                    "RegisterStatusNotifierItem",
                    &(name.as_str(),),
                ) {
                    Ok(()) => {
                        crate::diag_log::info("托盘菜单已注册");
                        registered_owner = Some(owner);
                        logged_failure = false;
                    }
                    Err(error) => {
                        if !logged_failure {
                            crate::diag_log::warn(&format!("托盘菜单注册失败：{error}"));
                            logged_failure = true;
                        }
                    }
                }
            }
            Some(_) => {}
            None => {
                registered_owner = None;
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
