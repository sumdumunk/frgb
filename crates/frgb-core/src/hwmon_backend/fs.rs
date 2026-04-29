use std::io;
use std::path::{Path, PathBuf};

/// Filesystem operations the hwmon backend performs. Injectable so tests can
/// substitute a `tempfile`-backed tree.
pub trait HwmonFs {
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn write_str(&self, path: &Path, contents: &str) -> io::Result<()>;
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>>;
    fn path_exists(&self, path: &Path) -> bool;
    /// Type-erased downcast for test fixtures to recover the concrete
    /// `FakeFs` from a `&dyn HwmonFs`. **Tests only** — this method is
    /// `cfg(test)`-gated and must never be called from production paths.
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Real filesystem implementation used in production.
pub struct RealFs;

impl HwmonFs for RealFs {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
    fn write_str(&self, path: &Path, contents: &str) -> io::Result<()> {
        std::fs::write(path, contents)
    }
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(path)? {
            out.push(entry?.path());
        }
        Ok(out)
    }
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn real_fs_read_write_round_trip() {
        let dir = std::env::temp_dir().join(format!("frgb_hwmonfs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fs = RealFs;
        let path = dir.join("foo");
        fs.write_str(&path, "42\n").unwrap();
        assert_eq!(fs.read_to_string(&path).unwrap(), "42\n");
        assert!(fs.path_exists(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_fs_read_dir_lists_entries() {
        let dir = std::env::temp_dir().join(format!("frgb_hwmonfs_rd_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        let fs = RealFs;
        let mut entries: Vec<PathBuf> = fs.read_dir(&dir).unwrap();
        entries.sort();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].ends_with("a"));
        assert!(entries[1].ends_with("b"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
pub mod tests_only {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io;
    use std::path::{Path, PathBuf};

    /// Shared test fixture: in-memory fs with write capture. Exposed
    /// pub(crate) so tests in sibling modules (writer.rs, state.rs) can
    /// reuse it rather than each defining their own.
    pub struct FakeFs {
        pub files: RefCell<HashMap<PathBuf, String>>,
        pub dirs: RefCell<HashMap<PathBuf, Vec<PathBuf>>>,
        pub writes: RefCell<Vec<(PathBuf, String)>>,
    }

    impl Default for FakeFs {
        fn default() -> Self {
            Self {
                files: RefCell::new(HashMap::new()),
                dirs: RefCell::new(HashMap::new()),
                writes: RefCell::new(Vec::new()),
            }
        }
    }

    impl FakeFs {
        pub fn with_pwm_file(path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
            let me = Self::default();
            me.files.borrow_mut().insert(path.into(), contents.into());
            me
        }
        pub fn set_file(&self, path: impl Into<PathBuf>, contents: impl Into<String>) {
            self.files.borrow_mut().insert(path.into(), contents.into());
        }
        pub fn set_dir(&self, path: impl Into<PathBuf>, children: Vec<PathBuf>) {
            self.dirs.borrow_mut().insert(path.into(), children);
        }
        pub fn last_write(&self, path: &str) -> Option<String> {
            self.writes
                .borrow()
                .iter()
                .rev()
                .find(|(p, _)| p == Path::new(path))
                .map(|(_, v)| v.clone())
        }
        pub fn write_count(&self, path: &str) -> usize {
            self.writes
                .borrow()
                .iter()
                .filter(|(p, _)| p == Path::new(path))
                .count()
        }
    }

    impl HwmonFs for FakeFs {
        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            self.files
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{path:?}")))
        }
        fn write_str(&self, path: &Path, contents: &str) -> io::Result<()> {
            self.writes
                .borrow_mut()
                .push((path.to_path_buf(), contents.to_string()));
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), contents.to_string());
            Ok(())
        }
        fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
            self.dirs
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{path:?}")))
        }
        fn path_exists(&self, path: &Path) -> bool {
            self.files.borrow().contains_key(path) || self.dirs.borrow().contains_key(path)
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
}
