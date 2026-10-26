//! Storage behind the mailbox. Durable Objects are one backend. Tests use
//! memory and a directory of files. Object files are named by object id.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub trait Kv: Send {
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn put(&mut self, key: &str, value: Vec<u8>);
    fn delete(&mut self, key: &str);
    fn list_prefix(&self, prefix: &str) -> Vec<String>;
}

#[derive(Clone, Default)]
pub struct MemoryKv {
    rows: BTreeMap<String, Vec<u8>>,
}

impl MemoryKv {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Kv for MemoryKv {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.rows.get(key).cloned()
    }

    fn put(&mut self, key: &str, value: Vec<u8>) {
        self.rows.insert(key.to_string(), value);
    }

    fn delete(&mut self, key: &str) {
        self.rows.remove(key);
    }

    fn list_prefix(&self, prefix: &str) -> Vec<String> {
        self.rows
            .range(prefix.to_string()..)
            .take_while(|(key, _)| key.starts_with(prefix))
            .map(|(key, _)| key.clone())
            .collect()
    }
}

pub struct FileKv {
    root: PathBuf,
}

impl FileKv {
    pub fn open(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        let _ = fs::create_dir_all(&root);
        Self { root }
    }

    fn path_for(&self, key: &str) -> Option<PathBuf> {
        if key.is_empty() || key.contains("..") || key.starts_with('/') {
            return None;
        }
        Some(self.root.join(key))
    }
}

impl Kv for FileKv {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.path_for(key)?;
        fs::read(path).ok()
    }

    fn put(&mut self, key: &str, value: Vec<u8>) {
        let Some(path) = self.path_for(key) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, value);
    }

    fn delete(&mut self, key: &str) {
        if let Some(path) = self.path_for(key) {
            let _ = fs::remove_file(path);
        }
    }

    fn list_prefix(&self, prefix: &str) -> Vec<String> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, prefix, &mut out);
        out.sort();
        out
    }
}

fn walk(root: &Path, dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, prefix, out);
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let key = rel.to_string_lossy().replace('\\', "/");
        if key.starts_with(prefix) {
            out.push(key);
        }
    }
}
