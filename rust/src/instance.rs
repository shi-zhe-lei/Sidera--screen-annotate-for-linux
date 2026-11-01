//! Serialize startup before inspecting/removing the single-instance socket.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::ops::Deref;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct Instance { listener: UnixListener, path: PathBuf, _lock: File }
impl Deref for Instance { type Target = UnixListener; fn deref(&self) -> &UnixListener { &self.listener } }
impl Drop for Instance { fn drop(&mut self) { let _ = fs::remove_file(&self.path); } }

pub fn runtime_directory() -> PathBuf {
    let uid = unsafe { libc::geteuid() };
    dirs::runtime_dir()
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/sidera-{uid}")))
}

/// Check the GUI lock without connecting to the single-instance socket.
/// Connecting would be accepted as a "show" command by the running instance.
pub fn running() -> bool {
    let lock_path = runtime_directory().join("sidera.lock");
    let Ok(file) = OpenOptions::new().read(true).write(true).open(lock_path) else {
        return false;
    };
    let fd = file.as_raw_fd();
    if unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        let _ = unsafe { libc::flock(fd, libc::LOCK_UN) };
        false
    } else {
        io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK)
    }
}

pub fn acquire(command: &[u8]) -> io::Result<Option<Instance>> {
    acquire_in(&runtime_directory(), command)
}

fn acquire_in(directory: &Path, command: &[u8]) -> io::Result<Option<Instance>> {
    match fs::DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e),
    }
    let meta = fs::symlink_metadata(directory)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        return Err(io::Error::other("单实例目录必须由当前用户独占"));
    }
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(directory.join("sidera.lock"))?;
    if !lock.metadata()?.is_file() || lock.metadata()?.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::other("无效单实例锁文件"));
    }
    let path = directory.join("sidera.sock");
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::WouldBlock { return Err(error); }
        // The owner may still be binding its socket. Never unlink it in this race.
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Ok(mut stream) = UnixStream::connect(&path) {
                stream.set_write_timeout(Some(Duration::from_millis(100)))?;
                stream.write_all(command)?;
                return Ok(None);
            }
            if Instant::now() >= deadline { return Err(io::Error::other("Sidera 已在启动，请稍后重试")); }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    // Cooperate with an older running build which did not acquire our new lock.
    if let Ok(mut stream) = UnixStream::connect(&path) {
        stream.set_write_timeout(Some(Duration::from_millis(100)))?;
        stream.write_all(command)?;
        return Ok(None);
    }
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_socket() && meta.uid() == unsafe { libc::geteuid() } => fs::remove_file(&path)?,
        Ok(_) => return Err(io::Error::other("拒绝删除非套接字通信文件")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(Some(Instance { listener, path, _lock: lock }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn contenders_wake_owner_without_unlinking_and_stale_socket_recovers() {
        let directory = std::env::temp_dir().join(format!("sidera-instance-test-{}", std::process::id()));
        let owner = acquire_in(&directory, b"show").unwrap().unwrap();
        let inode = fs::metadata(&owner.path).unwrap().ino();
        assert!(acquire_in(&directory, b"toggle").unwrap().is_none());
        assert_eq!(fs::metadata(&owner.path).unwrap().ino(), inode);
        let (mut peer, _) = owner.accept().unwrap();
        let mut message = String::new(); peer.read_to_string(&mut message).unwrap();
        assert_eq!(message, "toggle");
        drop(owner);
        let stale = UnixListener::bind(directory.join("sidera.sock")).unwrap();
        drop(stale);
        drop(acquire_in(&directory, b"show").unwrap().unwrap());
        fs::write(directory.join("sidera.sock"), b"keep").unwrap();
        assert!(acquire_in(&directory, b"show").is_err());
        assert_eq!(fs::read(directory.join("sidera.sock")).unwrap(), b"keep");
        fs::remove_dir_all(directory).unwrap();
    }
}
