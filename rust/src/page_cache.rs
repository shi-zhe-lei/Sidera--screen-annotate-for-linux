//! Lossless page snapshots: compressed in memory, then private temporary files.
//! A storage error keeps the snapshot in memory; page-count limits never discard ink.
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use image::ImageEncoder;
use tiny_skia::{IntSize, Pixmap};

/// 进程级页缓存上限。以前是"每个实例 32 MiB"，而每个输出各有批注和白板
/// 两份缓存，内存上限会随屏幕数量相乘；这里改成全进程共享一份预算。
#[cfg(not(test))]
const GLOBAL_BUDGET: usize = 64 * 1024 * 1024;
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

/// 单元测试各自构造私有缓存，关掉全局预算以免互相干扰。
#[cfg(test)]
fn global_budget() -> usize { usize::MAX }
#[cfg(not(test))]
fn global_budget() -> usize { GLOBAL_BUDGET }

fn live_bytes() -> usize { LIVE_BYTES.load(Ordering::Relaxed) }

/// 是否需要把页快照落盘：本实例超份额，或全进程超总预算。
fn should_spill(local_bytes: usize, local_budget: usize, live: usize, limit: usize) -> bool {
    local_bytes > local_budget || live > limit
}

fn account_add(bytes: usize) {
    if bytes > 0 { LIVE_BYTES.fetch_add(bytes, Ordering::Relaxed); }
}

fn account_sub(bytes: usize) {
    if bytes == 0 { return; }
    let mut current = LIVE_BYTES.load(Ordering::Relaxed);
    loop {
        let next = current.saturating_sub(bytes);
        match LIVE_BYTES.compare_exchange_weak(
            current, next, Ordering::Relaxed, Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(observed) => current = observed,
        }
    }
}

enum Snapshot { Png(Vec<u8>), Raw(Pixmap), File(PathBuf) }

fn snapshot_len(snapshot: &Snapshot) -> usize {
    match snapshot {
        Snapshot::Png(data) => data.len(),
        Snapshot::Raw(pixmap) => pixmap.data().len(),
        Snapshot::File(_) => 0,
    }
}
pub struct PageCache {
    pages: BTreeMap<i32, Snapshot>,
    directory: Option<PathBuf>,
    budget: usize,
}
impl Default for PageCache {
    fn default() -> Self { Self { pages: BTreeMap::new(), directory: None, budget: 32 * 1024 * 1024 } }
}
impl PageCache {
    pub fn contains_key(&self, page: &i32) -> bool { self.pages.contains_key(page) }
    #[cfg(test)]
    pub fn is_empty(&self) -> bool { self.pages.is_empty() }
    pub fn remove(&mut self, page: &i32) {
        if let Some(snapshot) = self.pages.remove(page) {
            account_sub(snapshot_len(&snapshot));
            if let Snapshot::File(path) = snapshot { let _ = fs::remove_file(path); }
        }
    }
    pub fn clear(&mut self) {
        for snapshot in self.pages.values() {
            account_sub(snapshot_len(snapshot));
            if let Snapshot::File(path) = snapshot { let _ = fs::remove_file(path); }
        }
        self.pages.clear();
    }
    pub fn get(&self, page: &i32) -> Option<Pixmap> {
        let result = match self.pages.get(page)? {
            Snapshot::Raw(pixmap) => return Some(pixmap.clone()),
            Snapshot::Png(data) => decode(data),
            Snapshot::File(path) => fs::read(path).ok().and_then(|data| decode(&data)),
        };
        if result.is_none() { log::error!("无法读取第 {page} 页笔迹缓存"); }
        result
    }
    pub fn insert(&mut self, page: i32, pixmap: Pixmap) {
        let mut data = Vec::new();
        // Encode the premultiplied bytes verbatim, unlike exported PNGs; restore
        // them verbatim too, so translucent strokes do not change after paging.
        let encoder = image::codecs::png::PngEncoder::new_with_quality(&mut data,
            image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Sub);
        let snapshot = match encoder.write_image(pixmap.data(), pixmap.width(), pixmap.height(), image::ExtendedColorType::Rgba8) {
            Ok(()) => Snapshot::Png(data),
            Err(error) => { log::warn!("笔迹压缩失败，保留原始快照：{error}"); Snapshot::Raw(pixmap) }
        };
        let new_len = snapshot_len(&snapshot);
        self.remove(&page);
        self.pages.insert(page, snapshot);
        account_add(new_len);
        let mut bytes: usize = self.pages.values().map(snapshot_len).sum();
        if !should_spill(bytes, self.budget, live_bytes(), global_budget()) { return; }
        let directory = match self.directory() {
            Ok(path) => path,
            Err(error) => { log::warn!("临时缓存不可用，笔迹继续保留在内存：{error}"); return; }
        };
        for (key, snapshot) in self.pages.iter_mut() {
            if !should_spill(bytes, self.budget, live_bytes(), global_budget()) { break; }
            if let Snapshot::Png(data) = snapshot {
                let len = data.len();
                let path = directory.join(format!("{key}.png"));
                match crate::persistence::atomic_write(&path, data) {
                    Ok(()) => { bytes -= len; account_sub(len); *snapshot = Snapshot::File(path); }
                    Err(error) => { log::warn!("写入缓存失败，笔迹继续保留在内存：{error}"); break; }
                }
            }
        }
    }
    fn directory(&mut self) -> std::io::Result<PathBuf> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        if let Some(path) = &self.directory { return Ok(path.clone()); }
        for _ in 0..100 {
            let path = std::env::temp_dir().join(format!("sidera-pages-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            #[allow(unused_mut)]
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => { self.directory = Some(path.clone()); return Ok(path); }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::other("无法创建独立缓存目录"))
    }
}
fn decode(data: &[u8]) -> Option<Pixmap> {
    let image = image::load_from_memory_with_format(data, image::ImageFormat::Png).ok()?.into_rgba8();
    let size = IntSize::from_wh(image.width(), image.height())?;
    Pixmap::from_vec(image.into_raw(), size)
}
impl Drop for PageCache {
    fn drop(&mut self) {
        self.clear();
        if let Some(path) = self.directory.take() { let _ = fs::remove_dir(path); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn more_than_twenty_pages_spill_restore_replace_and_clear_losslessly() {
        let mut cache = PageCache::default(); cache.budget = 512;
        for page in 1..=45 {
            let mut pixmap = Pixmap::new(32, 24).unwrap();
            pixmap.fill(tiny_skia::Color::from_rgba8(page, 150, 40, 97));
            cache.insert(page as i32, pixmap);
        }
        assert!(cache.pages.values().any(|p| matches!(p, Snapshot::File(_))));
        for page in (1..=45).rev() {
            let mut expected = Pixmap::new(32, 24).unwrap();
            expected.fill(tiny_skia::Color::from_rgba8(page, 150, 40, 97));
            assert_eq!(cache.get(&(page as i32)).unwrap().data(), expected.data());
        }
        cache.insert(3, Pixmap::new(7, 9).unwrap());
        let restored = cache.get(&3).unwrap();
        assert_eq!((restored.width(), restored.height()), (7, 9));
        cache.remove(&2); assert!(cache.get(&2).is_none());
        let directory = cache.directory.clone().unwrap();
        drop(cache); assert!(!directory.exists());
    }
    #[cfg(unix)]
    #[test]
    fn unavailable_spill_directory_keeps_ink_in_memory() {
        let mut cache = PageCache::default();
        cache.budget = 0; cache.directory = Some(PathBuf::from("/dev/null/cache"));
        let mut pixmap = Pixmap::new(8, 8).unwrap(); pixmap.fill(tiny_skia::Color::WHITE);
        cache.insert(30, pixmap.clone());
        assert_eq!(cache.get(&30).unwrap().data(), pixmap.data());
    }
    #[test]
    fn spill_decision_honours_local_and_global_budgets() {
        // 双屏时总占用会超过单实例份额：只要全进程超预算就要落盘。
        assert!(!should_spill(10, 100, 10, 100));
        assert!(should_spill(101, 100, 10, 1_000));
        assert!(should_spill(10, 100, 101, 100));
    }
}
