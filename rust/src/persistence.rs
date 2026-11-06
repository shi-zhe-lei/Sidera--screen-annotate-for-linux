//! Small atomic file replacement shared by user settings and temporary snapshots.
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or_else(|| io::Error::other("missing parent directory"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".sidera-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let result = write_temporary(path, &temporary, content);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn write_temporary(path: &Path, temporary: &Path, content: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let permissions = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => meta.permissions().mode() & 0o777,
        Ok(_) => return Err(io::Error::other("refusing to replace a non-regular file")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => 0o600,
        Err(e) => return Err(e),
    };
    // create_new also refuses symlinks and concurrent writers using the same name.
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(temporary)?;
    file.write_all(content)?;
    if let Err(error) = file.set_permissions(fs::Permissions::from_mode(permissions)) {
        // FAT/exFAT and some network shares do not implement chmod. The data is
        // still valid; do not discard settings just because metadata is limited.
        log::warn!("无法保留设置文件权限：{error}");
    }
    if let Err(error) = file.sync_all() {
        log::warn!("无法同步设置文件：{error}");
    }
    fs::rename(temporary, path)?;
    match fs::File::open(temporary.parent().unwrap_or(Path::new("."))) {
        Ok(directory) => {
            if let Err(error) = directory.sync_all() {
                log::warn!("无法同步设置目录：{error}");
            }
        }
        Err(error) => log::warn!("无法打开设置目录：{error}"),
    }
    Ok(())
}

#[cfg(windows)]
fn write_temporary(path: &Path, temporary: &Path, content: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(temporary)?;
    file.write_all(content)?;
    file.sync_all()?;
    // Windows cannot rename over an existing file. Replace it explicitly;
    // a future MoveFileExW wrapper can make this atomic.
    let _ = fs::remove_file(path);
    fs::rename(temporary, path)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn replacement_preserves_permissions_and_refuses_symlinks() {
        let dir = std::env::temp_dir().join(format!("sidera-atomic-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        atomic_write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        atomic_write(&path, b"complete new settings").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"complete new settings");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
        let link = dir.join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(atomic_write(&link, b"unexpected").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"complete new settings");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
}
