//! Scroll a document even while a touch or mouse grab belongs to the sidebar.
//! AT-SPI addresses the foreground application's own scrollbar. X11 clients
//! can also receive a wheel at their content window without warping the mouse.
use crate::document_scroll::ScrollTarget;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering::SeqCst},
    mpsc, Arc,
};
use std::time::{Duration, Instant};
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue, Value},
    Connection,
};

#[derive(Default)]
struct Shared {
    closed: AtomicBool,
    failed: AtomicBool,
    pending: AtomicI32,
    epoch: AtomicU64,
}

pub(crate) struct Document {
    shared: Arc<Shared>,
    wake: mpsc::SyncSender<()>,
}

impl Document {
    pub fn open() -> Self {
        let shared = Arc::<Shared>::default();
        let (wake, rx) = mpsc::sync_channel(1);
        let state = shared.clone();
        let spawn = std::thread::Builder::new()
            .name("document-scroll".into())
            .spawn(move || {
                let mut wheel = XWheel::capture();
                let target = futures_lite::future::block_on(futures_lite::future::race(
                    AccessibleDocument::find(&state),
                    async {
                        async_io::Timer::after(Duration::from_millis(1250)).await;
                        None
                    },
                ));
                if state.closed.load(SeqCst) {
                    return;
                }
                let mut target = target;
                while rx.recv().is_ok() && !state.closed.load(SeqCst) {
                    let epoch = state.epoch.load(SeqCst);
                    let pixels = f64::from(state.pending.swap(0, SeqCst)) / 256.0;
                    if pixels == 0.0 {
                        continue;
                    }
                    let live = || !state.closed.load(SeqCst) && state.epoch.load(SeqCst) == epoch;
                    if let Some(doc) = &mut target {
                        match futures_lite::future::block_on(doc.scroll(pixels, &live)) {
                            Outcome::Moved => continue,
                            Outcome::LostFocus => break,
                            Outcome::Unsupported => target = None,
                        }
                    }
                    if !live() {
                        continue;
                    }
                    if !wheel.as_mut().is_some_and(|x| x.scroll(pixels, &live)) {
                        break;
                    }
                }
                state.failed.store(true, SeqCst);
            });
        if spawn.is_err() {
            shared.failed.store(true, SeqCst);
        }
        Self { shared, wake }
    }
}

impl ScrollTarget for Document {
    fn scroll_by(&mut self, pixels: f64) -> bool {
        if self.shared.failed.load(SeqCst) {
            return false;
        }
        // Keep up to one short motion while the target is opening. Release and
        // direction changes discard it even if discovery is still in progress.
        let units = (pixels * 256.0).round() as i32;
        let _ = self.shared.pending.fetch_update(SeqCst, SeqCst, |old| {
            Some(old.saturating_add(units).clamp(-16384, 16384))
        });
        let _ = self.wake.try_send(());
        true
    }
    fn clear_pending(&mut self) {
        self.shared.epoch.fetch_add(1, SeqCst);
        self.shared.pending.store(0, SeqCst);
    }
}
impl Drop for Document {
    fn drop(&mut self) {
        self.shared.closed.store(true, SeqCst);
        self.clear_pending();
    }
}

type Object = (String, OwnedObjectPath);
const ACCESS: &str = "org.a11y.atspi.Accessible";
const COMPONENT: &str = "org.a11y.atspi.Component";
const VALUE: &str = "org.a11y.atspi.Value";
const PROPS: &str = "org.freedesktop.DBus.Properties";
const A11Y_BUS: &str = "org.a11y.Bus";
const A11Y_PATH: &str = "/org/a11y/bus";
const WINDOW_COORDS: u32 = 1;
const ACTIVE: u32 = 1 << 1;
const HORIZONTAL: u32 = 1 << 14;
const SCROLL_BAR: u32 = 48;
const NULL_PATH: &str = "/org/a11y/atspi/null";

async fn bounded<T, E>(request: impl Future<Output = Result<T, E>>) -> Option<T> {
    futures_lite::future::race(async { request.await.ok() }, async {
        async_io::Timer::after(Duration::from_millis(300)).await;
        None
    })
    .await
}

/// Firefox watches this session property even when it is already running.
/// Only the explicit settings action may change it. Opening a rail is read-only.
async fn toolkit_access(enable: bool) -> Option<bool> {
    let session = bounded(Connection::session()).await?;
    let property = ("org.a11y.Status", "IsEnabled");
    if enable {
        bounded(session.call_method(Some(A11Y_BUS), A11Y_PATH, Some(PROPS), "Set",
            &(property.0, property.1, Value::Bool(true)))).await?;
    }
    let reply = bounded(session.call_method(Some(A11Y_BUS), A11Y_PATH, Some(PROPS), "Get", &property)).await?;
    bool::try_from(reply.body().deserialize::<OwnedValue>().ok()?).ok()
}

#[derive(Default)]
pub(crate) struct ScrollAccess {
    pub enabled: Option<bool>,
    pending: Option<(bool, mpsc::Receiver<Option<bool>>)>,
}

impl ScrollAccess {
    pub fn busy(&self) -> bool { self.pending.is_some() }

    pub fn request(&mut self, enable: bool) {
        if self.busy() { return; }
        let (tx, rx) = mpsc::channel();
        self.pending = Some((enable, rx));
        let _ = std::thread::Builder::new().name("scroll-access".into()).spawn(move || {
            let _ = tx.send(futures_lite::future::block_on(toolkit_access(enable)));
        });
    }

    /// Return whether the completed request was the user's enable action.
    pub fn poll(&mut self) -> Option<bool> {
        let (enable, rx) = self.pending.as_ref()?;
        self.enabled = match rx.try_recv() {
            Ok(value) => value,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        let enable = *enable;
        self.pending = None;
        Some(enable)
    }
}

macro_rules! method {
    ($c:expr, $o:expr, $iface:expr, $name:expr, $args:expr, $ty:ty) => {{
        let object = &$o;
        bounded($c.call_method(
            Some(object.0.as_str()),
            object.1.as_str(),
            Some($iface),
            $name,
            $args,
        ))
        .await
        .and_then(|message| message.body().deserialize::<$ty>().ok())
    }};
}

async fn children(c: &Connection, o: &Object) -> Vec<Object> {
    method!(c, o, ACCESS, "GetChildren", &(), Vec<Object>).unwrap_or_default()
}

async fn states(c: &Connection, o: &Object) -> u32 {
    method!(c, o, ACCESS, "GetState", &(), Vec<u32>)
        .and_then(|s| s.first().copied())
        .unwrap_or(0)
}

type Bounds = (i32, i32, i32, i32);
async fn bounds(c: &Connection, o: &Object) -> Option<Bounds> {
    method!(c, o, COMPONENT, "GetExtents", &(WINDOW_COORDS,), Bounds)
        .filter(|&(x, y, w, h)| x > -100_000 && y > -100_000 && w > 0 && h > 0)
}

async fn value(c: &Connection, o: &Object, name: &str) -> Option<f64> {
    method!(c, o, PROPS, "Get", &(VALUE, name), OwnedValue)
        .and_then(|v| f64::try_from(v).ok())
        .filter(|v| v.is_finite())
}

/// Prefer the scrollbar belonging to the pane covering the document centre,
/// rather than a narrower outline, thumbnails list, or unrelated sidebar.
async fn scrollbar(c: &Connection, window: &Object, centre: (i32, i32), state: &Shared) -> Option<Object> {
    // Collection.GetMatches uses a four-word role bitmap and two state words.
    let mut roles = [0_i32; 4];
    roles[(SCROLL_BAR / 32) as usize] = 1 << (SCROLL_BAR % 32);
    let rule = (
        vec![0_i32; 2],
        1_i32,
        HashMap::<String, String>::new(),
        1_i32,
        roles.to_vec(),
        1_i32,
        Vec::<String>::new(),
        1_i32,
        false,
    );
    let mut bars = method!(
        c,
        window,
        "org.a11y.atspi.Collection",
        "GetMatches",
        &(rule, 1_u32, 32_i32, true),
        Vec<Object>
    )
    .unwrap_or_default();
    if bars.is_empty() {
        // Qt often puts the scrollbar in a separate sibling container.
        // Search chrome only, never enumerate document text or table cells.
        let mut containers = VecDeque::from([window.clone()]);
        let mut inspected = 0;
        while let Some(item) = containers.pop_front() {
            if inspected >= 96 || state.closed.load(SeqCst) {
                break;
            }
            for child in children(c, &item).await.into_iter().take(32) {
                if inspected >= 96 || state.closed.load(SeqCst) {
                    break;
                }
                inspected += 1;
                match method!(c, child, ACCESS, "GetRole", &(), u32) {
                    Some(SCROLL_BAR) => bars.push(child),
                    // Filler, frame, layered pane, page tab, panel, root pane,
                    // scroll pane, split pane, viewport, window, grouping.
                    Some(20 | 23 | 30 | 37 | 38 | 39 | 46 | 49 | 53 | 68 | 69 | 99) => {
                        containers.push_back(child)
                    }
                    _ => (),
                }
            }
        }
    }
    let mut best = None;
    for bar in bars {
        if state.closed.load(SeqCst) {
            return None;
        }
        let Some((bar_x, bar_y, w, h)) = bounds(c, &bar).await else {
            continue;
        };
        if h <= w || states(c, &bar).await & HORIZONTAL != 0 {
            continue;
        }
        if centre.1 < bar_y || centre.1 >= bar_y.saturating_add(h) { continue; }
        let mut ancestor = bar.clone();
        let mut pane_rank = 0;
        for rank in (1..=5).rev() {
            let Some(parent) = method!(c, ancestor, PROPS, "Get", &(ACCESS, "Parent"), OwnedValue)
                .and_then(|v| <Object>::try_from(v).ok()) else { break };
            if parent == ancestor || parent == *window { break; }
            if bounds(c, &parent).await.is_some_and(|(x, y, w, h)| {
                (x..x.saturating_add(w)).contains(&centre.0) && (y..y.saturating_add(h)).contains(&centre.1)
            }) {
                pane_rank = rank;
                break;
            }
            ancestor = parent;
        }
        let score = (pane_rank, h, bar_x >= centre.0);
        if best.as_ref().is_none_or(|(previous, _)| score > *previous) {
            if let (Some(max), Some(min)) = (
                value(c, &bar, "MaximumValue").await,
                value(c, &bar, "MinimumValue").await,
            ) {
                if max > min {
                    best = Some((score, bar));
                }
            }
        }
    }
    best.map(|(_, bar)| bar)
}

async fn at_point(c: &Connection, o: &Object, point: (i32, i32)) -> Option<Object> {
    method!(
        c,
        o,
        COMPONENT,
        "GetAccessibleAtPoint",
        &(point.0, point.1, WINDOW_COORDS),
        Object
    )
    .filter(|o| !o.0.is_empty() && o.1.as_str() != NULL_PATH)
}

enum Outcome {
    Moved,
    LostFocus,
    Unsupported,
}
enum Surface {
    Bar {
        object: Object,
        min: f64,
        max: f64,
        position: f64,
    },
    Content,
}
struct AccessibleDocument {
    bus: Connection,
    window: Object,
    viewport: Bounds,
    surface: Surface,
    fraction: f64,
    anchor: Option<Object>,
    blocked: Option<(i32, Instant)>,
    opened: Instant,
}

impl AccessibleDocument {
    async fn find(state: &Shared) -> Option<Self> {
        let session = bounded(Connection::session()).await?;
        let reply = bounded(session.call_method(
            Some(A11Y_BUS),
            A11Y_PATH,
            Some(A11Y_BUS),
            "GetAddress",
            &(),
        ))
        .await?;
        let address: String = reply.body().deserialize().ok()?;
        let bus = bounded(zbus::connection::Builder::address(address.as_str()).ok()?.build()).await?;
        let root = (
            "org.a11y.atspi.Registry".into(),
            OwnedObjectPath::try_from("/org/a11y/atspi/accessible/root").ok()?,
        );
        // Gecko may register after the Shell, even if the registry was already populated.
        for attempt in 0..6 {
            if state.closed.load(SeqCst) { return None; }
            if attempt != 0 { async_io::Timer::after(Duration::from_millis(55)).await; }
            for window in Self::windows(&bus, &root, state).await {
                if state.closed.load(SeqCst) { return None; }
                let Some((x, y, w, h)) = bounds(&bus, &window).await else { continue };
                let centre = (x + w / 2, y + h / 2);
                let surface = match scrollbar(&bus, &window, centre, state).await {
                    Some(bar) => Surface::Bar {
                        min: value(&bus, &bar, "MinimumValue").await?,
                        max: value(&bus, &bar, "MaximumValue").await?,
                        position: value(&bus, &bar, "CurrentValue").await?,
                        object: bar,
                    },
                    None => Surface::Content,
                };
                return Some(Self {
                    bus,
                    window,
                    viewport: (x, y, w, h),
                    surface,
                    fraction: 0.0,
                    anchor: None,
                    blocked: None,
                    opened: Instant::now(),
                });
            }
        }
        None
    }

    async fn windows(bus: &Connection, root: &Object, state: &Shared) -> Vec<Object> {
        let mut active = Vec::new();
        for app in children(bus, root).await {
            if state.closed.load(SeqCst) { break; }
            for window in children(bus, &app).await {
                if states(bus, &window).await & ACTIVE != 0 { active.push(window); }
            }
        }
        active
    }

    async fn scroll(&mut self, pixels: f64, live: &impl Fn() -> bool) -> Outcome {
        let c = &self.bus;
        if states(c, &self.window).await & ACTIVE == 0 {
            return Outcome::LostFocus;
        }
        if !live() {
            return Outcome::Moved;
        }
        if let Surface::Bar {
            object,
            min,
            max,
            position,
        } = &mut self.surface
        {
            // Most adjustments use document pixels. Normalised controls expose
            // a small range, so avoid moving from end to end in one frame.
            let units = if *max - *min <= 1.0 {
                (*max - *min) / 1200.0
            } else {
                1.0
            };
            let next = (*position + pixels * units).clamp(*min, *max);
            if next == *position {
                return Outcome::Moved;
            }
            if method!(
                c,
                object,
                PROPS,
                "Set",
                &(VALUE, "CurrentValue", Value::F64(next)),
                ()
            )
            .is_some()
            {
                *position = next;
                return Outcome::Moved;
            }
            self.surface = Surface::Content;
        }
        // Web/PDF views may hide the scrollbar but implement ScrollToPoint on
        // a content child. Move that child's top edge by the requested delta.
        self.fraction += pixels;
        let step = self.fraction.trunc() as i32;
        if step == 0 {
            return Outcome::Moved;
        }
        self.fraction -= f64::from(step);
        if self.blocked.is_some_and(|(dir, until)| dir == step.signum() && Instant::now() < until) {
            return Outcome::Moved;
        }
        let (x, y, w, h) = self.viewport;
        let mut tried = Vec::new();
        let mut accepted = false;
        if let Some(anchor) = self.anchor.take() {
            if let Some(moved) = self.move_anchor(&anchor, step, live).await {
                if moved { self.anchor = Some(anchor); self.blocked = None; return Outcome::Moved; }
                accepted = true;
            }
            tried.push(anchor);
        }
        // Hit-testing at a second point also works when the centre is over a
        // sticky header, a floating control, or the gap between two paragraphs.
        for point in [(x+w/2,y+h/2), (x+w/3,y+h*2/3), (x+w*2/3,y+h/3)] {
            let mut path = Vec::new();
            let mut item = &self.window;
            while live() && path.len() < 12 {
                let Some(child) = at_point(c, item, point).await else { break };
                if &child == item || path.contains(&child) { break; }
                path.push(child);
                item = path.last().unwrap();
            }
            for item in path.into_iter().rev() {
                if !live() { return Outcome::Moved; }
                if tried.contains(&item) { continue; }
                if let Some(moved) = self.move_anchor(&item, step, live).await {
                    if moved { self.anchor = Some(item); self.blocked = None; return Outcome::Moved; }
                    accepted = true;
                }
                tried.push(item);
            }
        }
        if accepted {
            // At a document edge, avoid probing on every frame. A reversal
            // bypasses the delay immediately.
            self.blocked = Some((step.signum(), Instant::now()+Duration::from_millis(160)));
            Outcome::Moved
        } else if self.opened.elapsed() < Duration::from_millis(450) {
            // A Firefox frame can be active before its remote web document is attached.
            self.blocked = Some((step.signum(), Instant::now()+Duration::from_millis(35)));
            Outcome::Moved
        } else {
            Outcome::Unsupported
        }
    }

    /// Gecko can return true for a stationary browser-chrome or fixed element.
    /// Only remember it as a scroll anchor when its actual position has moved.
    async fn move_anchor(&self, item: &Object, step: i32, live: &impl Fn() -> bool) -> Option<bool> {
        let (x, y, _, height) = bounds(&self.bus, item).await?;
        let (_, top, _, view_height) = self.viewport;
        if y >= top + view_height || y.saturating_add(height) <= top || !live() { return None; }
        if method!(&self.bus, item, COMPONENT, "ScrollToPoint",
            &(WINDOW_COORDS, x, y.saturating_sub(step)), bool) != Some(true) { return None; }
        for attempt in 0..3 {
            if attempt != 0 { async_io::Timer::after(Duration::from_millis(12)).await; }
            let (_, current, _, _) = bounds(&self.bus, item).await?;
            if y.saturating_sub(current).signum() == step.signum() { return Some(true); }
            if !live() { break; }
        }
        Some(false)
    }
}

use x11rb::connection::Connection as XConnection;
use x11rb::protocol::xproto::{self, AtomEnum, ConnectionExt, EventMask, Window};
use x11rb::rust_connection::RustConnection;

struct XWheel {
    conn: RustConnection,
    root: Window,
    atom: u32,
    window: Window,
    remainder: f64,
}
impl XWheel {
    fn capture() -> Option<Self> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let atom = conn
            .intern_atom(true, b"_NET_ACTIVE_WINDOW")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let mut target = Self {
            conn,
            root,
            atom,
            window: 0,
            remainder: 0.0,
        };
        target.window = target.active()?;
        Some(target)
    }

    fn active(&self) -> Option<Window> {
        let prop = self
            .conn
            .get_property(false, self.root, self.atom, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let active = prop.value32()?.next().filter(|&w| w != 0 && w != self.root)?;
        // Xwayland can retain an old EWMH active window while a native Wayland
        // client owns focus. Never address that old window.
        let mut focus = self.conn.get_input_focus().ok()?.reply().ok()?.focus;
        for _ in 0..24 {
            if focus == active {
                return Some(active);
            }
            if focus <= 1 || focus == self.root {
                break;
            }
            focus = self.conn.query_tree(focus).ok()?.reply().ok()?.parent;
        }
        None
    }

    fn scroll(&mut self, pixels: f64, live: &impl Fn() -> bool) -> bool {
        if self.remainder.signum() != pixels.signum() {
            self.remainder = 0.0;
        }
        self.remainder += pixels;
        if self.remainder.abs() < 48.0 {
            return true;
        }
        if self.active() != Some(self.window) {
            return false;
        }
        let Some(geo) = self
            .conn
            .get_geometry(self.window)
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            return false;
        };
        let Some(pos) = self
            .conn
            .translate_coordinates(
                self.window,
                self.root,
                (geo.width / 2) as i16,
                (geo.height / 2) as i16,
            )
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            return false;
        };
        let mut child = self.window;
        let mut local = (0, 0);
        for _ in 0..16 {
            let Some(p) = self
                .conn
                .translate_coordinates(self.root, child, pos.dst_x, pos.dst_y)
                .ok()
                .and_then(|c| c.reply().ok())
            else {
                return false;
            };
            local = (p.dst_x, p.dst_y);
            if p.child == 0 {
                break;
            }
            child = p.child;
        }
        let button = if self.remainder > 0.0 { 5 } else { 4 };
        while self.remainder.abs() >= 48.0 && live() {
            for ty in [xproto::BUTTON_PRESS_EVENT, xproto::BUTTON_RELEASE_EVENT] {
                let event = xproto::ButtonPressEvent {
                    response_type: ty,
                    detail: button,
                    sequence: 0,
                    time: x11rb::CURRENT_TIME,
                    root: self.root,
                    event: child,
                    child: 0,
                    root_x: pos.dst_x,
                    root_y: pos.dst_y,
                    event_x: local.0,
                    event_y: local.1,
                    state: 0_u16.into(),
                    same_screen: true,
                };
                // Deliver to the content window owner. XI2 toolkits need not
                // select core ButtonPressMask, even when they handle wheel events.
                if self.conn.send_event(false, child, EventMask::NO_EVENT, event).is_err() {
                    return false;
                }
            }
            self.remainder -= self.remainder.signum() * 48.0;
        }
        self.conn.flush().is_ok()
    }
}
