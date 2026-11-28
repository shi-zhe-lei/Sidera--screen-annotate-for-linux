use super::{connect_socket, control, transport, Transport};
use crate::document_scroll::ScrollTarget;
use serde_json::{json, Value};
use std::{cell::Cell, rc::Rc, sync::{atomic::Ordering, mpsc::Receiver}, time::{Duration, Instant}};

#[derive(Default)]
pub(super) struct Gesture(Rc<Cell<u64>>);

impl Gesture {
    pub(super) fn cancel(&self) { self.0.set(0); }

    pub(super) fn receive(&self, message: &Value) {
        let id = message["gesture"].as_u64().unwrap_or(0);
        match message["type"].as_str() {
            Some("press") if message["button"] == 1 => self.0.set(id),
            Some("touch") if message["kind"] == "begin" && id != 0 => self.0.set(id),
            Some("cancel") => self.cancel(),
            Some("release") if id != 0 && id == self.0.get() => self.cancel(),
            Some("touch") if id == self.0.get() &&
                matches!(message["kind"].as_str(),Some("end" | "cancel")) => self.cancel(),
            Some("wheelFailed") if id == self.0.get() => self.cancel(),
            _ => {}
        }
    }

    pub(super) fn target(&self, link: &Transport, generation: u32, index: u32) -> Option<Box<dyn ScrollTarget>> {
        let id = self.0.get();
        (id != 0).then(|| Box::new(Wheel {
            link: link.clone(), current: self.0.clone(), id, index, generation, pending: 0.0,
        }) as Box<dyn ScrollTarget>)
    }
}

struct Wheel {
    link: Transport,
    current: Rc<Cell<u64>>,
    id: u64,
    index: u32,
    generation: u32,
    pending: f64,
}

impl Wheel {
    fn send(&self, pixels: f64) -> bool {
        self.link.send(vec![control(&json!({"type":"documentWheel","generation":self.generation,
            "index":self.index,"gesture":self.id,"pixels":pixels}))])
    }
}

impl ScrollTarget for Wheel {
    fn scroll_by(&mut self, pixels: f64) -> bool {
        if self.current.get() != self.id || self.link.broken.load(Ordering::Relaxed) { return false; }
        if self.pending.signum() != pixels.signum() { self.pending = 0.0; }
        self.pending = (self.pending + pixels).clamp(-64.0, 64.0);
        if self.send(self.pending) { self.pending = 0.0; }
        true
    }
    fn clear_pending(&mut self) {
        self.pending = 0.0;
        if self.current.get() == self.id { self.send(0.0); }
    }
}

// Only XWayland on Mutter uses this connection. Shell owns the two narrow
// input regions while X11 draws their contents and runs the usual drag logic.
#[derive(Default)]
pub(crate) struct CompatRail {
    rails: Option<[[i32; 4]; 2]>,
    sent: Option<(i32, i32, [[i32; 4]; 2])>,
    link: Option<(Transport, Receiver<Value>)>,
    last_attempt: Option<Instant>,
    ping: Option<Instant>,
    gesture: Gesture,
    finger: Option<i64>,
}

pub(crate) enum Contact { Down(i32,i32), Move(i32,i32), Up(i32,i32), Cancel }

impl CompatRail {
    pub(crate) fn geometry(&mut self, rails: Option<[[i32; 4]; 2]>) { self.rails = rails; }

    pub(crate) fn target(&self) -> Option<Box<dyn ScrollTarget>> {
        self.gesture.target(&self.link.as_ref()?.0, 0, 0)
    }

    pub(crate) fn poll(&mut self, size: (i32,i32)) -> Vec<Contact> {
        let mut contacts = Vec::new();
        if self.rails.is_none() || self.link.as_ref().is_some_and(|(tx,_)| tx.broken.load(Ordering::Relaxed)) {
            if self.finger.take().is_some() { contacts.push(Contact::Cancel); }
            self.gesture.cancel();
            self.link = None;
            self.sent = None;
        }
        let Some(rails) = self.rails else { self.last_attempt = None; return contacts };
        if self.link.is_none() && self.last_attempt.is_none_or(|t| t.elapsed() >= Duration::from_secs(2)) {
            self.last_attempt = Some(Instant::now());
            self.link = connect_socket("scroll.sock").ok().flatten().and_then(|stream| transport(stream).ok());
            self.ping = None;
        }
        let Some((tx, rx)) = &self.link else { return contacts };
        let (width,height) = size;
        if self.sent != Some((width,height,rails)) &&
            tx.send(vec![control(&json!({"type":"rails","width":width,"height":height,"rails":rails}))]) {
            self.sent = Some((width,height,rails));
        }
        if self.ping.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) &&
            tx.send(vec![control(&json!({"type":"ping"}))]) { self.ping = Some(Instant::now()); }
        for v in rx.try_iter().take(128) {
            let kind = v["type"].as_str();
            if kind == Some("wheelFailed") { self.gesture.receive(&v); continue; }
            if kind == Some("cancel") {
                if self.finger.take().is_some() { self.gesture.cancel(); contacts.push(Contact::Cancel); }
                continue;
            }
            let (Some(x),Some(y)) = (v["x"].as_i64(),v["y"].as_i64()) else { continue };
            let (x,y) = (x.clamp(-32768,32768) as i32,y.clamp(-32768,32768) as i32);
            let (finger, phase) = match kind {
                Some("press") if v["button"] == 1 => (-1, "begin"),
                Some("release") if v["button"] == 1 => (-1, "end"),
                Some("motion") => (-1, "update"),
                Some("touch") => (v["touch"].as_i64().unwrap_or(-2), v["kind"].as_str().unwrap_or("")),
                _ => continue,
            };
            match phase {
                "begin" if self.finger.is_none() && v["gesture"].as_u64().unwrap_or(0) != 0 => {
                    self.finger = Some(finger); self.gesture.receive(&v); contacts.push(Contact::Down(x,y));
                    // Let on_press capture this gesture before consuming another contact.
                    break;
                }
                "update" if self.finger == Some(finger) => contacts.push(Contact::Move(x,y)),
                "end" if self.finger == Some(finger) => {
                    self.finger = None; self.gesture.cancel(); contacts.push(Contact::Up(x,y)); break;
                }
                "cancel" if self.finger == Some(finger) => {
                    self.finger = None; self.gesture.cancel(); contacts.push(Contact::Cancel); break;
                }
                _ => {}
            }
        }
        contacts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_scroll_stale_release_and_failure_cannot_stop_a_new_gesture() {
        let gesture = Gesture::default();
        gesture.receive(&json!({"type":"press","button":1,"gesture":5}));
        gesture.receive(&json!({"type":"release","button":1,"gesture":5}));
        assert_eq!(gesture.0.get(), 0);
        gesture.receive(&json!({"type":"touch","kind":"begin","gesture":6}));
        gesture.receive(&json!({"type":"wheelFailed","gesture":5}));
        gesture.receive(&json!({"type":"touch","kind":"end","gesture":0}));
        assert_eq!(gesture.0.get(), 6);
        gesture.receive(&json!({"type":"touch","kind":"cancel","gesture":6}));
        assert_eq!(gesture.0.get(), 0);
    }
}
