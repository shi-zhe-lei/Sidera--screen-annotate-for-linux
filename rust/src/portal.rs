//! Bounded Screenshot request: subscribe before calling, cancel on timeout,
//! and decode local file URIs. Never request RemoteDesktop for page navigation.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use futures_lite::{future, StreamExt};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

type Error = Box<dyn std::error::Error + Send + Sync>;
static BUSY: AtomicBool = AtomicBool::new(false);
static TOKEN: AtomicU64 = AtomicU64::new(0);

pub fn screenshot_nonblocking(saved: std::sync::Arc<AtomicBool>) {
    if BUSY.swap(true, Ordering::AcqRel) { return; }
    std::thread::spawn(move || {
        struct Reset;
        impl Drop for Reset { fn drop(&mut self) { BUSY.store(false, Ordering::Release); } }
        let _reset = Reset;
        match future::block_on(run()) {
            Ok(true) => saved.store(true, Ordering::Relaxed),
            Ok(false) => {}
            Err(error) => log::warn!("截图未完成：{error}"),
        }
    });
}

fn local_file(uri: &str) -> Result<std::path::PathBuf, Error> {
    let url = url::Url::parse(uri)?;
    if url.scheme() != "file" || url.query().is_some() || url.fragment().is_some()
        || url.host_str().is_some_and(|host| host != "localhost") {
        return Err("截图服务返回的地址不是本地文件".into());
    }
    url.to_file_path().map_err(|_| "无效的截图文件路径".into())
}

async fn run() -> Result<bool, Error> {
    let conn = future::race(async { Ok::<_, Error>(zbus::Connection::session().await?) }, async {
        async_io::Timer::after(Duration::from_secs(5)).await;
        Err("无法连接桌面截图服务".into())
    }).await?;
    let token = format!("sidera_{}_{}", std::process::id(), TOKEN.fetch_add(1, Ordering::Relaxed));
    let sender = conn.unique_name().ok_or("会话连接没有唯一名称")?.as_str().trim_start_matches(':').replace('.', "_");
    let mut handle = OwnedObjectPath::try_from(format!("/org/freedesktop/portal/desktop/request/{sender}/{token}"))?;
    let result: Result<Option<String>, Error> = future::race(async {
        let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.portal.Desktop")?
            .interface("org.freedesktop.portal.Request")?.member("Response")?.build();
        // Subscribe before the method call; older portals may return another path.
        let mut responses = zbus::MessageStream::for_match_rule(rule, &conn, Some(8)).await?;
        let proxy = zbus::Proxy::new(&conn, "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Screenshot").await?;
        let options = HashMap::from([
            ("handle_token", Value::from(token.as_str())),
            ("interactive", Value::Bool(false)), ("modal", Value::Bool(false)),
        ]);
        handle = proxy.call("Screenshot", &("", options)).await?;
        while let Some(message) = responses.next().await {
            let message = message?;
            if message.header().path().map(|p| p.as_str()) != Some(handle.as_str()) { continue; }
            let (code, values): (u32, HashMap<String, OwnedValue>) = message.body().deserialize()?;
            return match code {
                0 => Ok(Some(values.get("uri").and_then(|v| <&str>::try_from(v).ok())
                    .ok_or("截图服务没有返回文件地址")?.to_string())),
                1 => Ok(None),
                _ => Err("桌面拒绝或未能完成截图".into()),
            };
        }
        Err("截图服务连接已结束".into())
    }, async {
        async_io::Timer::after(Duration::from_secs(30)).await;
        Err("截图等待超时，已取消请求".into())
    }).await;
    if result.is_err() {
        future::race(async {
            if let Ok(request) = zbus::Proxy::new(&conn, "org.freedesktop.portal.Desktop", handle.as_str(), "org.freedesktop.portal.Request").await {
                let _: Result<(), _> = request.call("Close", &()).await;
            }
        }, async { async_io::Timer::after(Duration::from_secs(1)).await; }).await;
    }
    if let Some(uri) = result? {
        let bytes = std::fs::read(local_file(&uri)?)?;
        let image = image::load_from_memory(&bytes)?.to_rgba8();
        let (w, h) = image.dimensions();
        return Ok(crate::save_and_clipboard(image.into_raw(), w, h));
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_local_file_uris_are_accepted_and_decoded() {
        assert_eq!(super::local_file("file:///tmp/a%20b%23c.png").unwrap().to_str(), Some("/tmp/a b#c.png"));
        for uri in ["https://example.com/a.png", "file://remote/tmp/a", "file:///tmp/a?x=1", "file:///tmp/a#fragment"] {
            assert!(super::local_file(uri).is_err());
        }
    }
}
