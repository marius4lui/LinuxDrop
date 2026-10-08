//! Opened source authority: transfer code never re-resolves the selected path.
//! This pins the selected inode, not an immutable snapshot of concurrent writes.
use std::{
    fs::File,
    io,
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawFd,
    },
    path::Path,
    sync::Arc,
};

#[derive(Clone)]
pub struct SendSource {
    file: Arc<File>,
    name: String,
    stamp: Stamp,
}
#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Stamp {
    fn read(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::other("Only regular files can be shared"));
        }
        Ok(Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}
impl std::fmt::Debug for SendSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendSource")
            .field("name", &self.name)
            .field("size", &self.stamp.size)
            .finish()
    }
}
impl SendSource {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| io::Error::other("Choose a file with a UTF-8 name"))?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        Self::from_file(name.to_owned(), file)
    }
    pub fn from_file(name: String, file: File) -> io::Result<Self> {
        if name.is_empty()
            || name.len() > 255
            || name == "."
            || name == ".."
            || name
                .chars()
                .any(|c| c == '/' || c == '\\' || c.is_control())
        {
            return Err(io::Error::other("Invalid source filename"));
        }
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if flags & libc::O_ACCMODE == libc::O_WRONLY || flags & libc::O_PATH != 0 {
            return Err(io::Error::other("Source descriptor must be readable"));
        }
        let stamp = Stamp::read(&file)?;
        Ok(Self {
            file: Arc::new(file),
            name,
            stamp,
        })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn size(&self) -> u64 {
        self.stamp.size
    }
    pub fn verify(&self) -> io::Result<()> {
        let current = Stamp::read(&self.file)?;
        // A rename/unlink can change ctime without changing content. Only use
        // mtime+length for stability; the held descriptor itself fixes identity.
        if current.dev != self.stamp.dev
            || current.ino != self.stamp.ino
            || current.size != self.stamp.size
            || current.modified != self.stamp.modified
        {
            return Err(io::Error::other(
                "The selected file changed; select it again",
            ));
        }
        Ok(())
    }
    pub fn reader(&self) -> io::Result<File> {
        self.verify()?;
        // Opening our own held descriptor gives an independent offset. dup/
        // try_clone would share offsets between concurrent sends/downloads.
        let file = File::open(format!("/proc/self/fd/{}", self.file.as_raw_fd()))?;
        if Stamp::read(&file)? != Stamp::read(&self.file)? {
            return Err(io::Error::other("Source descriptor changed"));
        }
        self.verify()?;
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn path_replacement_cannot_replace_bytes_and_readers_have_independent_offsets() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chosen.txt");
        std::fs::write(&path, b"original contents").unwrap();
        let source = SendSource::open(&path).unwrap();
        std::fs::rename(&path, root.path().join("moved.txt")).unwrap();
        std::fs::write(&path, b"replacement secrets").unwrap();
        let mut first = source.reader().unwrap();
        let mut prefix = [0; 3];
        first.read_exact(&mut prefix).unwrap();
        let mut second = String::new();
        source
            .reader()
            .unwrap()
            .read_to_string(&mut second)
            .unwrap();
        assert_eq!(second, "original contents");
        let mut remainder = String::new();
        first.read_to_string(&mut remainder).unwrap();
        assert_eq!(remainder, "ginal contents");
        std::fs::remove_file(root.path().join("moved.txt")).unwrap();
        assert!(source.reader().is_ok());
        assert_eq!(source.name(), "chosen.txt");
    }
    #[test]
    fn changed_content_symlinks_and_non_files_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chosen.txt");
        std::fs::write(&path, b"original").unwrap();
        let source = SendSource::open(&path).unwrap();
        std::fs::write(&path, b"changed length").unwrap();
        assert!(source.reader().is_err());
        std::os::unix::fs::symlink(&path, root.path().join("link")).unwrap();
        assert!(SendSource::open(root.path().join("link")).is_err());
        assert!(SendSource::open(root.path()).is_err());
    }
}
