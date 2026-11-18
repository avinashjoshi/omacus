//! A backup of the whole library: pages, folders, settings, and Recently deleted.
//!
//! Locked pages are copied as ciphertext. The file is a zip a person can keep.
//! Nothing in it is a key, and a locked page's text is not written in the clear.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::json;

const KIND: &str = "omacus-backup";
const FORMAT: u32 = 1;

/// Write `dest` from the library at `root` (`pages/`, `trash/`, `state.json`).
/// `settings_file` is included when it exists. Returns how many pages were saved.
pub fn export_library(
    root: &Path,
    settings_file: Option<&Path>,
    dest: &Path,
) -> Result<usize, String> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let pages = read_dir_files(&root.join("pages"), "pages")?;
    let page_count = pages.len();
    let trash = read_dir_files(&root.join("trash"), "trash")?;
    if let Some(state) = read_optional(&root.join("state.json"))? {
        files.push(("state.json".into(), state.clone()));
        files.push(("library.json".into(), state));
    } else {
        files.push(("library.json".into(), b"{}".to_vec()));
    }
    if let Some(path) = settings_file
        && let Some(bytes) = read_optional(path)?
    {
        files.push(("settings.toml".into(), bytes));
    }
    files.extend(pages);
    files.extend(trash);
    let manifest = json!({
        "format": FORMAT,
        "kind": KIND,
        "version": env!("CARGO_PKG_VERSION"),
        "pages": page_count,
    });
    files.insert(
        0,
        (
            "manifest.json".into(),
            serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        ),
    );
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = zip_stored(&files)?;
    let mut out = File::create(dest).map_err(|e| e.to_string())?;
    out.write_all(&bytes).map_err(|e| e.to_string())?;
    Ok(page_count)
}

/// A validated desktop export. Keep the bytes reviewed by the user so the
/// archive can be moved or changed while the confirmation is open.
pub struct Backup {
    files: Vec<(String, Vec<u8>)>,
    pub pages: usize,
}

impl Backup {
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        let mut zip = zip::ZipArchive::new(file).map_err(|_| "This isn't an Omacus backup.")?;
        let mut files = Vec::new();
        let mut names = std::collections::HashSet::new();
        let mut total = 0u64;
        let mut pages = 0;
        for i in 0..zip.len() {
            let entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = entry
                .name()
                .map_err(|_| "A backup filename couldn't be read.")?
                .into_owned();
            let valid = match name.as_str() {
                "manifest.json" | "library.json" | "state.json" | "settings.toml" => true,
                _ => name.split_once('/').is_some_and(|(folder, file)| {
                    matches!(folder, "pages" | "trash")
                        && file.strip_suffix(".txt").is_some_and(|id| {
                            safe_name(id)
                                && !id.contains([':', '*', '?', '"', '<', '>', '|'])
                                && !id.chars().any(char::is_control)
                        })
                }),
            };
            let size = entry.size();
            total = total.saturating_add(size);
            if !valid
                || !names.insert(name.to_lowercase())
                || entry.compression() != zip::CompressionMethod::Stored
                || total > length
            {
                return Err("This ZIP isn't an Omacus desktop backup.".into());
            }
            let mut bytes = Vec::new();
            entry
                .take(size.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() as u64 != size {
                return Err("A backup file is incomplete.".into());
            }
            std::str::from_utf8(&bytes).map_err(|_| "A backup file couldn't be read.")?;
            pages += usize::from(name.starts_with("pages/"));
            files.push((name, bytes));
        }
        let find = |name| {
            files
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, bytes)| bytes.as_slice())
        };
        let manifest: serde_json::Value =
            serde_json::from_slice(find("manifest.json").ok_or("This isn't an Omacus backup.")?)
                .map_err(|_| "The backup manifest couldn't be read.")?;
        if manifest["kind"] != KIND || manifest["format"] != FORMAT || manifest["pages"] != pages {
            return Err("This backup format isn't supported or the backup is incomplete.".into());
        }
        let library = find("library.json").ok_or("The backup is missing the library.")?;
        if find("state.json").is_some_and(|state| state != library)
            || !crate::sheets::valid_backup_state(library)
        {
            return Err("The backup library couldn't be read.".into());
        }
        if let Some(settings) = find("settings.toml") {
            let settings = std::str::from_utf8(settings).map_err(|e| e.to_string())?;
            toml::from_str::<toml::Table>(settings)
                .map_err(|_| "The backup settings couldn't be read.")?;
        }
        Ok(Self { files, pages })
    }

    /// Stage on the same filesystems as the destinations. Move the prior
    /// entries aside, and put them back if any replacement fails.
    pub fn restore(&self, root: &Path, settings: Option<&Path>) -> Result<(), String> {
        use std::fs;
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let library = tempfile::tempdir_in(root).map_err(|e| e.to_string())?;
        let staged = library.path().join("new");
        fs::create_dir_all(staged.join("pages")).map_err(|e| e.to_string())?;
        fs::create_dir_all(staged.join("trash")).map_err(|e| e.to_string())?;
        let config = settings
            .map(|path| {
                let parent = path.parent().ok_or("No settings folder.")?;
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                tempfile::tempdir_in(parent).map_err(|e| e.to_string())
            })
            .transpose()?;
        for (name, bytes) in &self.files {
            let dest = match name.as_str() {
                "manifest.json" | "state.json" => continue,
                "library.json" => staged.join("state.json"),
                "settings.toml" => config
                    .as_ref()
                    .ok_or("No settings folder.")?
                    .path()
                    .join("new"),
                _ => staged.join(name),
            };
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dest)
                .and_then(|mut file| file.write_all(bytes))
                .map_err(|e| e.to_string())?;
        }

        let mut moves = Vec::new();
        for name in [
            "pages",
            "trash",
            "state.json",
            "sheets",
            "sheet.txt",
            "library.automerge",
            "sync",
        ] {
            moves.push((
                root.join(name),
                staged.join(name),
                library.path().join(name),
            ));
        }
        if let (Some(path), Some(config)) = (settings, &config) {
            moves.push((
                path.to_path_buf(),
                config.path().join("new"),
                config.path().join("old"),
            ));
        }
        let mut undo = Vec::new();
        for (dest, new, old) in moves {
            let result = (|| {
                match fs::rename(&dest, &old) {
                    Ok(()) => undo.push((old, dest.clone())),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
                if new.exists() {
                    fs::rename(&new, &dest)?;
                    undo.push((dest, new));
                }
                Ok(())
            })();
            if let Err(error) = result {
                let mut rollback_failed = false;
                for (from, to) in undo.into_iter().rev() {
                    rollback_failed |= fs::rename(from, to).is_err();
                }
                if rollback_failed {
                    let mut dirs = vec![library.keep()];
                    dirs.extend(config.map(tempfile::TempDir::keep));
                    let paths = dirs
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>();
                    return Err(format!(
                        "{error}. Previous files are in {}.",
                        paths.join(" and ")
                    ));
                }
                return Err(error.to_string());
            }
        }
        Ok(())
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(Some(bytes))
}

fn read_dir_files(dir: &Path, prefix: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    names.sort();
    let mut out = Vec::new();
    for path in names {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !safe_name(name) {
            continue;
        }
        let Some(bytes) = read_optional(&path)? else {
            continue;
        };
        out.push((format!("{prefix}/{name}"), bytes));
    }
    Ok(out)
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.starts_with('.')
}

fn zip_stored(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut locals = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let name_bytes = name.as_bytes();
        if name_bytes.len() > u16::MAX as usize {
            return Err("A backup name is too long.".into());
        }
        if data.len() > u32::MAX as usize {
            return Err("A page is too large to export.".into());
        }
        let crc = crc32(data);
        let offset = u32::try_from(locals.len()).map_err(|_| "The backup is too large.")?;
        write_u32(&mut locals, 0x0403_4b50);
        write_u16(&mut locals, 20);
        write_u16(&mut locals, 0x0800);
        write_u16(&mut locals, 0);
        write_u16(&mut locals, 0);
        write_u16(&mut locals, 0);
        write_u32(&mut locals, crc);
        write_u32(&mut locals, data.len() as u32);
        write_u32(&mut locals, data.len() as u32);
        write_u16(&mut locals, name_bytes.len() as u16);
        write_u16(&mut locals, 0);
        locals.extend_from_slice(name_bytes);
        locals.extend_from_slice(data);

        write_u32(&mut central, 0x0201_4b50);
        write_u16(&mut central, 20);
        write_u16(&mut central, 20);
        write_u16(&mut central, 0x0800);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u32(&mut central, crc);
        write_u32(&mut central, data.len() as u32);
        write_u32(&mut central, data.len() as u32);
        write_u16(&mut central, name_bytes.len() as u16);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u32(&mut central, 0);
        write_u32(&mut central, offset);
        central.extend_from_slice(name_bytes);
    }
    let cd_offset = u32::try_from(locals.len()).map_err(|_| "The backup is too large.")?;
    let cd_size = u32::try_from(central.len()).map_err(|_| "The backup is too large.")?;
    let count = u16::try_from(files.len()).map_err(|_| "Too many files to export.")?;
    locals.extend_from_slice(&central);
    write_u32(&mut locals, 0x0605_4b50);
    write_u16(&mut locals, 0);
    write_u16(&mut locals, 0);
    write_u16(&mut locals, count);
    write_u16(&mut locals, count);
    write_u32(&mut locals, cd_size);
    write_u32(&mut locals, cd_offset);
    write_u16(&mut locals, 0);
    Ok(locals)
}

fn write_u16(buf: &mut Vec<u8>, n: u16) {
    buf.extend_from_slice(&n.to_le_bytes());
}

fn write_u32(buf: &mut Vec<u8>, n: u32) {
    buf.extend_from_slice(&n.to_le_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheets::Store;
    use std::io::Cursor;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "omacus-backup-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn export_keeps_a_locked_page_sealed_and_an_open_page_intact() {
        let root = temp_root("round");
        let mut store = Store::open_at(root.clone());
        store.new_sheet("# Trip Budget\nferry = 40\n");
        let locked_id = store.sheets[0].id.clone();
        store.new_sheet("# Groceries\nmilk $3\n");
        store.lock_page(&locked_id, "correct horse").unwrap();
        store.lock_now();
        store.save().unwrap();
        let settings = root.join("settings.toml");
        std::fs::write(&settings, "theme = \"paper\"\n").unwrap();
        let dest = root.join("backup.zip");
        let count = export_library(&root, Some(&settings), &dest).unwrap();
        assert_eq!(count, 2);

        let bytes = std::fs::read(&dest).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("correct horse"), "{text}");
        assert!(!text.contains("ferry"), "{text}");
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut names = Vec::new();
        for i in 0..archive.len() {
            names.push(archive.by_index(i).unwrap().name().unwrap().into_owned());
        }
        assert!(names.iter().any(|n| n == "manifest.json"));
        assert!(names.iter().any(|n| n == "library.json"));
        assert!(names.iter().any(|n| n == "settings.toml"));
        let mut manifest = String::new();
        archive
            .by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut manifest)
            .unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        assert_eq!(manifest["kind"], KIND);
        assert_eq!(manifest["format"], FORMAT);
        assert_eq!(manifest["pages"], 2);

        let locked_name = format!("pages/{locked_id}.txt");
        let mut sealed = String::new();
        archive
            .by_name(&locked_name)
            .unwrap()
            .read_to_string(&mut sealed)
            .unwrap();
        assert!(sealed.starts_with("OMACUS-LOCK/1\n"), "{sealed}");
        assert!(
            !sealed.contains("ferry") && !sealed.contains("Trip"),
            "{sealed}"
        );

        let open = names
            .iter()
            .find(|n| n.starts_with("pages/") && *n != &locked_name)
            .unwrap()
            .clone();
        let mut plain = String::new();
        archive
            .by_name(&open)
            .unwrap()
            .read_to_string(&mut plain)
            .unwrap();
        assert!(plain.contains("milk $3"), "{plain}");
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn restore_replaces_the_desktop_library_and_keeps_locks() {
        let source = tempfile::tempdir().unwrap();
        let mut store = Store::open_at(source.path().to_path_buf());
        let folder = store.create_folder(None).unwrap();
        store.rename_folder(&folder, "Trips");
        store.new_sheet_in(Some(&folder), "# Ferry\n2 tickets");
        let locked = store.sheets[store.active].id.clone();
        store.lock_page(&locked, "correct horse").unwrap();
        store.new_sheet("# Pantry\nbeans 4");
        store.delete(store.active);
        store.save().unwrap();
        let config = source.path().join("settings.toml");
        std::fs::write(&config, "theme = \"paper\"\n").unwrap();
        let zip = source.path().join("backup.zip");
        export_library(source.path(), Some(&config), &zip).unwrap();

        let dest = tempfile::tempdir().unwrap();
        let mut old = Store::open_at(dest.path().to_path_buf());
        old.new_sheet("# Old page");
        old.save().unwrap();
        std::fs::create_dir(dest.path().join("sync")).unwrap();
        std::fs::write(dest.path().join("sync/device.bin"), b"previous key").unwrap();
        let settings = dest.path().join("config/settings.toml");
        let backup = Backup::read(&zip).unwrap();
        assert_eq!(backup.pages, 1);
        backup.restore(dest.path(), Some(&settings)).unwrap();

        let restored = Store::open_at(dest.path().to_path_buf());
        assert_eq!(restored.sheets.len(), 1);
        assert!(restored.sheets[0].locked());
        assert_eq!(
            restored.folder_of(&locked).as_deref(),
            Some(folder.as_str())
        );
        assert_eq!(restored.trashed()[0].title, "Pantry");
        assert_eq!(
            std::fs::read(&settings).unwrap(),
            std::fs::read(config).unwrap()
        );
        assert!(!dest.path().join("sync").exists());
    }

    #[test]
    fn restore_rejects_paths_and_incomplete_backups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup.zip");
        let manifest = (
            "manifest.json".into(),
            br#"{"kind":"omacus-backup","format":1,"pages":1}"#.to_vec(),
        );
        let library = ("library.json".into(), b"{}".to_vec());
        for name in [
            "pages/../outside.txt",
            "pages/trip.txt:other",
            "trash/pantry.txt",
        ] {
            let bytes = zip_stored(&[
                manifest.clone(),
                library.clone(),
                (name.into(), b"page".to_vec()),
            ])
            .unwrap();
            std::fs::write(&path, bytes).unwrap();
            assert!(Backup::read(&path).is_err(), "accepted {name}");
        }
    }
}
