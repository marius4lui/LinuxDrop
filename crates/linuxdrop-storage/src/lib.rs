//! File creation relative to a held directory descriptor. Network names never
//! become paths, partials are exclusive, and publication cannot replace a file.
use anyhow::{bail, Context, Result};
use std::{
    ffi::CString,
    fs::File,
    os::fd::{AsRawFd, FromRawFd},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct ReceiveStore {
    directory: Arc<File>,
    path: PathBuf,
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 200
        || name
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
    {
        bail!("Invalid file name");
    }
    Ok(())
}

impl ReceiveStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        std::fs::create_dir_all(path).context("Create receive directory")?;
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .context("Open receive directory without following a symlink")?;
        Ok(Self {
            directory: Arc::new(directory),
            path: path.canonicalize()?,
        })
    }

    pub fn create(&self, name: &str) -> Result<PendingFile> {
        validate_name(name)?;
        let temporary = CString::new(format!(".linuxdrop-{}.part", Uuid::new_v4()))?;
        // SAFETY: directory and C string remain live for openat; returned fd is owned.
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                temporary.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        Ok(PendingFile {
            file: tokio::fs::File::from_std(file),
            directory: self.directory.clone(),
            path: self.path.clone(),
            temporary,
            name: name.into(),
            published: false,
        })
    }
}

pub struct PendingFile {
    pub file: tokio::fs::File,
    directory: Arc<File>,
    path: PathBuf,
    temporary: CString,
    name: String,
    published: bool,
}

impl PendingFile {
    pub async fn commit(mut self) -> Result<PathBuf> {
        self.file.sync_all().await?;
        for index in 0..10_000 {
            let name = if index == 0 {
                self.name.clone()
            } else {
                let p = Path::new(&self.name);
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
                match p.extension().and_then(|s| s.to_str()) {
                    Some(ext) => format!("{stem} ({index}).{ext}"),
                    None => format!("{stem} ({index})"),
                }
            };
            let target = CString::new(name.clone())?;
            // linkat is atomic and fails when *anything*, including a symlink,
            // already occupies the destination. Both names use the same held fd.
            let result = unsafe {
                libc::linkat(
                    self.directory.as_raw_fd(),
                    self.temporary.as_ptr(),
                    self.directory.as_raw_fd(),
                    target.as_ptr(),
                    0,
                )
            };
            if result == 0 {
                self.published = true;
                unsafe {
                    libc::unlinkat(self.directory.as_raw_fd(), self.temporary.as_ptr(), 0);
                }
                self.directory.sync_all()?;
                return Ok(self.path.join(name));
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EEXIST) {
                return Err(error.into());
            }
        }
        bail!("Too many name collisions")
    }
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        if !self.published {
            unsafe {
                libc::unlinkat(self.directory.as_raw_fd(), self.temporary.as_ptr(), 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn collision_and_symlink_never_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let store = ReceiveStore::open(temp.path()).unwrap();
        let outside = temp.path().join("important.txt");
        std::fs::write(&outside, "original").unwrap();
        std::os::unix::fs::symlink(&outside, temp.path().join("photo.txt")).unwrap();
        let mut incoming = store.create("photo.txt").unwrap();
        incoming.file.write_all(b"received").await.unwrap();
        let saved = incoming.commit().await.unwrap();
        assert_eq!(saved.file_name().unwrap(), "photo (1).txt");
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "original");
        assert_eq!(std::fs::read_to_string(saved).unwrap(), "received");
    }

    #[test]
    fn traversal_rejected_and_partials_removed() {
        let temp = tempfile::tempdir().unwrap();
        let store = ReceiveStore::open(temp.path()).unwrap();
        for name in ["../escape", "/etc/passwd", "a\\b", "..", "nul\0file"] {
            assert!(store.create(name).is_err());
        }
        drop(store.create("valid.txt").unwrap());
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}
