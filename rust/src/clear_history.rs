//! 清空后的恢复快照。容量先限制为 1，底层复用 PageCache 的压缩/落盘，
//! 后续要支持多级恢复时只需调大 MAX_RESTORE。
use std::collections::VecDeque;

use tiny_skia::Pixmap;

use crate::page_cache::PageCache;

const MAX_RESTORE: usize = 1;

struct Entry {
    key: i32,
    page: i32,
    whiteboard: bool,
}

#[derive(Default)]
pub struct ClearHistory {
    snapshots: PageCache,
    entries: VecDeque<Entry>,
    next_key: i32,
}

impl ClearHistory {
    pub fn push(&mut self, page: i32, whiteboard: bool, pixmap: Pixmap) {
        while self.entries.len() >= MAX_RESTORE {
            if let Some(old) = self.entries.pop_front() {
                self.snapshots.remove(&old.key);
            }
        }
        let key = self.next_key;
        self.next_key = self.next_key.wrapping_add(1).max(1);
        self.snapshots.insert(key, pixmap);
        self.entries.push_back(Entry { key, page, whiteboard });
    }

    pub fn available(&self, page: i32, whiteboard: bool) -> bool {
        self.entries.back().is_some_and(|entry| entry.page == page && entry.whiteboard == whiteboard)
    }

    pub fn take(&mut self, page: i32, whiteboard: bool) -> Option<Pixmap> {
        if !self.available(page, whiteboard) {
            return None;
        }
        let key = self.entries.back()?.key;
        let snapshot = self.snapshots.get(&key)?;
        self.entries.pop_back();
        self.snapshots.remove(&key);
        Some(snapshot)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.snapshots.clear();
    }

    pub fn clear_whiteboard(&mut self) {
        if self.entries.back().is_some_and(|entry| entry.whiteboard) {
            self.clear();
        }
    }
}
