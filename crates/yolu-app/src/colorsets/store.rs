use super::*;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

pub fn read_bounded(path: &Path) -> Result<Vec<u8>, Error> {
    if !fs::metadata(path).map_err(|_| Error::Io)?.is_file() {
        return Err(Error::Io);
    }
    let file = fs::File::open(path).map_err(|_| Error::Io)?;
    if !file.metadata().map_err(|_| Error::Io)?.is_file() {
        return Err(Error::Io);
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Io)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::Limit);
    }
    Ok(bytes)
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::Limit);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|_| Error::Io)?;
    let pending = parent.join(format!(
        ".colors-{}-{}.pending",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|_| Error::Io)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&pending, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&pending);
    }
    result.map_err(|_| Error::Io)
}
impl ColorSets {
    pub(super) fn save_palette(&self, id: u64, palette: &Palette) -> Result<(), Error> {
        if self.store_blocked {
            return Err(Error::Io);
        }
        let bytes = format::encode(palette)?;
        if let Some(dir) = &self.directory {
            atomic_write(&dir.join(format!("{id:016x}.ycolors")), &bytes)?;
        }
        Ok(())
    }
    pub fn attach(&mut self, directory: std::path::PathBuf, recent: &mut Vec<Rgba>) -> Vec<Error> {
        self.store_blocked = false;
        let mut problems = Vec::new();
        let mut entries = Vec::new();
        let mut next_id = 1;
        match fs::read_dir(&directory) {
            Ok(files) => {
                for (seen, file) in files.enumerate() {
                    if seen >= MAX_SETS * 2 {
                        problems.push(Error::Limit);
                        self.store_blocked = true;
                        break;
                    }
                    let Ok(file) = file else {
                        problems.push(Error::Io);
                        self.store_blocked = true;
                        continue;
                    };
                    let path = file.path();
                    if path.extension().and_then(|s| s.to_str()) != Some("ycolors") {
                        continue;
                    }
                    let Some(id) = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .filter(|s| s.len() == 16)
                        .and_then(|s| u64::from_str_radix(s, 16).ok())
                    else {
                        problems.push(Error::Name);
                        continue;
                    };
                    if path.file_name().and_then(|s| s.to_str())
                        != Some(format!("{id:016x}.ycolors").as_str())
                    {
                        problems.push(Error::Name);
                        continue;
                    }
                    next_id = next_id.max(id.saturating_add(1));
                    if next_id == u64::MAX {
                        self.store_blocked = true;
                        problems.push(Error::Limit);
                    }
                    if entries.len() >= MAX_SETS {
                        problems.push(Error::Limit);
                        self.store_blocked = true;
                        break;
                    }
                    match read_bounded(&path).and_then(|b| format::decode(&b)) {
                        Ok(palette) => entries.push(Entry { id, palette }),
                        Err(e) => problems.push(e),
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                problems.push(Error::Io);
                self.store_blocked = true;
                return problems;
            }
        }
        entries.sort_by_key(|e| e.id);
        // 読めないセットを上書きしない。読めた分がないときは新しい ID の既定セット。
        if entries.is_empty() {
            entries.push(Entry {
                id: next_id,
                palette: Palette::default(),
            });
            next_id = next_id.saturating_add(1);
        }
        self.entries = entries;
        self.next_id = next_id;
        self.active = 0;
        self.selected = None;
        self.directory = Some(directory.clone());
        let first = &self.entries[0];
        if !directory
            .join(format!("{:016x}.ycolors", first.id))
            .exists()
        {
            if let Err(e) = self.save_palette(first.id, &first.palette) {
                problems.push(e);
            }
        }
        self.state_read_failed = false;
        let state_path = directory.join("state.conf");
        match read_bounded(&state_path).and_then(|b| format::decode(&b)) {
            Ok(p) if (4..=68).contains(&p.colors.len()) && p.name.parse::<u64>().is_ok() => {
                let id = p.name.parse::<u64>().unwrap();
                self.active = self.entries.iter().position(|e| e.id == id).unwrap_or(0);
                for (corner, c) in self.corners.iter_mut().zip(&p.colors) {
                    *corner = c.rgba;
                }
                *recent = p.colors[4..].iter().map(|c| c.rgba).collect();
            }
            Err(Error::Io) if !state_path.exists() => {}
            Ok(_) => {
                self.state_read_failed = true;
                problems.push(Error::Header);
            }
            Err(e) => {
                self.state_read_failed = true;
                problems.push(e);
            }
        }
        self.saved_state = Some(self.state_palette(recent));
        problems
    }
    fn state_palette(&self, recent: &[Rgba]) -> Palette {
        Palette {
            name: self.entries[self.active].id.to_string(),
            colors: self
                .corners
                .iter()
                .chain(recent)
                .copied()
                .map(Swatch::new)
                .collect(),
        }
    }
    pub fn persist_state(&mut self, recent: &[Rgba]) -> Result<(), Error> {
        if self.store_blocked {
            return Ok(());
        }
        let Some(dir) = self.directory.as_ref() else {
            return Ok(());
        };
        let p = self.state_palette(recent);
        if self.saved_state.as_ref() == Some(&p) {
            return Ok(());
        }
        // 失敗時にも同じ状態を毎フレーム書かない。次の変更で再試行する。
        self.saved_state = Some(p.clone());
        if self.state_read_failed {
            return Err(Error::Header);
        }
        if recent.len() > crate::state::MAX_RECENT_COLORS {
            return Err(Error::Limit);
        }
        atomic_write(&dir.join("state.conf"), &format::encode(&p)?)
    }
}
