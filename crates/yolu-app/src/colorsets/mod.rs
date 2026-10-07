//! 個人設定のカラーセット・色の履歴・中間色。文書の Undo・保存には含めない。
pub mod format;
mod store;
use crate::{lang::Lang, state::Rgba};
pub use store::{atomic_write, read_bounded};

pub const MAX_COLORS: usize = 4096;
pub const MAX_SETS: usize = 128;
pub const MAX_NAME_BYTES: usize = 256;
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Limit,
    Name,
    Color,
    Header,
    Encoding,
    Version,
    Truncated,
    Trailing,
    Mismatch,
    ColorSpace(u16),
    Alpha,
    Io,
    Selection,
    LastSet,
}
impl Error {
    pub fn message(&self, lang: Lang) -> String {
        match self {
            Self::Limit => lang.pick("カラーセットの上限超過", "Color set limit exceeded"),
            Self::Name => lang.pick("色またはセットの名前が不正", "Invalid color or set name"),
            Self::Color => lang.pick("色の値が範囲外", "Color value out of range"),
            Self::Header => lang.pick("カラーセットの形式が不正", "Invalid color set header"),
            Self::Encoding => lang.pick("名前の文字コードが不正", "Invalid name encoding"),
            Self::Version => lang.pick("未対応のカラーセットの版", "Unsupported color set version"),
            Self::Truncated => lang.pick("カラーセットのデータ不足", "Truncated color set"),
            Self::Trailing => lang.pick("カラーセットに余分なデータ", "Trailing color set data"),
            Self::Mismatch => lang.pick("ACO の色の一覧が不一致", "Inconsistent ACO color lists"),
            Self::ColorSpace(space) => {
                return format!(
                    "{}: {space}",
                    lang.pick("未対応の ACO 色空間", "Unsupported ACO color space")
                )
            }
            Self::Alpha => lang.pick("GPL は透明度に未対応", "GPL does not support alpha"),
            Self::Io => lang.pick("カラー設定の読み書きに失敗", "Color settings I/O failed"),
            Self::Selection => lang.pick("色が未選択", "No color selected"),
            Self::LastSet => lang.pick("最後のカラーセット", "Last color set"),
        }
        .into()
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Swatch {
    pub name: String,
    pub rgba: Rgba,
}
impl Swatch {
    pub fn new(rgba: Rgba) -> Self {
        Self {
            name: String::new(),
            rgba,
        }
    }
    pub fn label(&self) -> String {
        let c = self.rgba.map(crate::ui::widgets::to_byte);
        format!(
            "{} #{:02X}{:02X}{:02X}{:02X}",
            self.name, c[0], c[1], c[2], c[3]
        )
        .trim()
        .into()
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub name: String,
    pub colors: Vec<Swatch>,
}
impl Default for Palette {
    fn default() -> Self {
        // 自作の基本色。第三者のセットは同梱しない。
        let rgb = [
            [24, 28, 36],
            [78, 87, 103],
            [165, 174, 187],
            [247, 245, 239],
            [202, 61, 72],
            [236, 139, 57],
            [241, 206, 86],
            [111, 166, 83],
            [51, 151, 148],
            [62, 117, 187],
            [124, 87, 173],
            [203, 115, 156],
            [100, 65, 53],
            [167, 116, 78],
            [213, 165, 130],
            [236, 207, 178],
        ];
        Self {
            name: "Yolu".into(),
            colors: rgb
                .into_iter()
                .map(|c| {
                    Swatch::new([
                        c[0] as f32 / 255.0,
                        c[1] as f32 / 255.0,
                        c[2] as f32 / 255.0,
                        1.0,
                    ])
                })
                .collect(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u64,
    pub palette: Palette,
}
#[derive(Clone, Debug)]
pub enum Edit {
    Add(Rgba),
    Replace(usize, Rgba),
    Remove(usize),
    Move(usize, usize),
    Rename(String),
}

#[derive(Debug)]
pub struct ColorSets {
    entries: Vec<Entry>,
    active: usize,
    pub selected: Option<usize>,
    pub corners: [Rgba; 4],
    pub dragging: Option<usize>,
    pub rename: Option<String>,
    directory: Option<std::path::PathBuf>,
    next_id: u64,
    saved_state: Option<Palette>,
    state_read_failed: bool,
    store_blocked: bool,
}
impl Default for ColorSets {
    fn default() -> Self {
        Self {
            entries: vec![Entry {
                id: 1,
                palette: Palette::default(),
            }],
            active: 0,
            selected: None,
            corners: [
                [1.0, 0.0, 0.0, 1.0],
                [1.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
            ],
            dragging: None,
            rename: None,
            directory: None,
            next_id: 2,
            saved_state: None,
            state_read_failed: false,
            store_blocked: false,
        }
    }
}
impl ColorSets {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn active_index(&self) -> usize {
        self.active
    }
    pub fn palette(&self) -> &Palette {
        &self.entries[self.active].palette
    }
    pub fn switch(&mut self, index: usize) {
        if index < self.entries.len() {
            self.active = index;
            self.selected = None;
            self.dragging = None;
            self.rename = None;
        }
    }
    pub fn edit(&mut self, edit: Edit) -> Result<(), Error> {
        let mut p = self.palette().clone();
        let mut selected = self.selected;
        match edit {
            Edit::Add(color) => {
                p.colors.push(Swatch::new(color));
                selected = Some(p.colors.len() - 1);
            }
            Edit::Replace(i, color) => p.colors.get_mut(i).ok_or(Error::Selection)?.rgba = color,
            Edit::Remove(i) => {
                if i >= p.colors.len() {
                    return Err(Error::Selection);
                }
                p.colors.remove(i);
                selected = None;
            }
            Edit::Move(from, to) => {
                if from >= p.colors.len() || to >= p.colors.len() {
                    return Err(Error::Selection);
                }
                let c = p.colors.remove(from);
                p.colors.insert(to, c);
                selected = Some(to);
            }
            Edit::Rename(name) => {
                if name.trim().is_empty() {
                    return Err(Error::Name);
                }
                p.name = name;
            }
        }
        format::validate(&p)?;
        self.save_palette(self.entries[self.active].id, &p)?;
        self.entries[self.active].palette = p;
        self.selected = selected;
        Ok(())
    }
    pub fn add_set(&mut self, palette: Palette) -> Result<(), Error> {
        if self.entries.len() >= MAX_SETS || self.next_id == u64::MAX {
            return Err(Error::Limit);
        }
        format::validate(&palette)?;
        let id = self.next_id;
        self.save_palette(id, &palette)?;
        self.next_id += 1;
        self.entries.push(Entry { id, palette });
        self.switch(self.entries.len() - 1);
        Ok(())
    }
    pub fn duplicate(&mut self, lang: Lang) -> Result<(), Error> {
        let mut p = self.palette().clone();
        let suffix = lang.pick(" 複製", " Copy");
        while p.name.len() + suffix.len() > MAX_NAME_BYTES {
            p.name.pop();
        }
        p.name += suffix;
        self.add_set(p)
    }
    pub fn remove_set(&mut self) -> Result<(), Error> {
        if self.store_blocked {
            return Err(Error::Io);
        }
        if self.entries.len() <= 1 {
            return Err(Error::LastSet);
        }
        if let Some(dir) = &self.directory {
            let path = dir.join(format!("{:016x}.ycolors", self.entries[self.active].id));
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::Io),
            }
        }
        self.entries.remove(self.active);
        self.switch(self.active.min(self.entries.len() - 1));
        Ok(())
    }
    pub fn import(&mut self, path: &std::path::Path) -> Result<(), Error> {
        let bytes = read_bounded(path)?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Palette");
        let p = match path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "gpl" => format::read_gpl(&bytes, name)?,
            "aco" => format::read_aco(&bytes, name)?,
            _ => return Err(Error::Header),
        };
        self.add_set(p)
    }
    pub fn export(&self, path: &std::path::Path) -> Result<(), Error> {
        atomic_write(path, &format::write_gpl(self.palette())?)
    }
}

/// 左上・右上・左下・右下。sRGB の符号値と straight alpha をそれぞれ双線形補間する。
pub fn intermediate(c: [Rgba; 4], x: f32, y: f32) -> Rgba {
    let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
    std::array::from_fn(|i| {
        (c[0][i] * (1.0 - x) + c[1][i] * x) * (1.0 - y) + (c[2][i] * (1.0 - x) + c[3][i] * x) * y
    })
}
/// 24 点の色と 4 点の隙間。非常に狭い欄でも少なくとも 1 列。
pub fn columns(width: f32) -> usize {
    ((width + 4.0) / 28.0).floor().max(1.0) as usize
}

/// 設定のフォルダのカラーセットを読む。読めなかった物があれば、その理由の文（起動時の知らせに添える）。
pub fn attach(app: &mut crate::state::AppState, directory: std::path::PathBuf) -> Option<String> {
    let problems = app.colorsets.attach(directory, &mut app.color.recent);
    (!problems.is_empty()).then(|| {
        problems
            .iter()
            .map(|p| p.message(app.lang))
            .collect::<Vec<_>>()
            .join(" / ")
    })
}
pub fn persist(app: &mut crate::state::AppState) {
    if let Err(e) = app.colorsets.persist_state(&app.color.recent) {
        let text = e.message(app.lang);
        app.fail(crate::notice::Source::Color, text);
    }
}
