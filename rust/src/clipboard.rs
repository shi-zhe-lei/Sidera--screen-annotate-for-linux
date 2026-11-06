//! Keep the selection owner alive, including desktops without a clipboard manager.
use std::sync::{mpsc, OnceLock};

pub fn copy_image(bytes: Vec<u8>, width: u32, height: u32) {
    type Image = (Vec<u8>, u32, u32);
    static SERVICE: OnceLock<mpsc::SyncSender<Image>> = OnceLock::new();
    let sender = SERVICE.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel::<Image>(1);
        std::thread::spawn(move || {
            let mut clipboard = None;
            while let Ok((bytes, width, height)) = receiver.recv() {
                if clipboard.is_none() {
                    match arboard::Clipboard::new() {
                        Ok(value) => clipboard = Some(value),
                        Err(error) => { log::warn!("截图已保存，但无法连接剪贴板：{error}"); continue; }
                    }
                }
                let data = arboard::ImageData { width: width as usize, height: height as usize,
                    bytes: std::borrow::Cow::Owned(bytes) };
                match clipboard.as_mut().unwrap().set_image(data) {
                    Ok(()) => log::info!("截图已复制到剪贴板"),
                    Err(error) => { log::warn!("截图已保存，但复制失败：{error}"); clipboard = None; }
                }
            }
        });
        sender
    });
    if let Err(error) = sender.try_send((bytes, width, height)) {
        log::warn!("截图已保存，剪贴板仍在处理上一张图片：{error}");
    }
}

pub fn get_text() -> Option<String> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    clipboard.get_text().ok()
}
