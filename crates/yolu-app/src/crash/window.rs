//! 伏せた記録をそのまま表示し、利用者が明示した操作だけを外へ渡す。
use crate::lang::Lang;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Request {
    Folder,
    GitHub,
}

#[derive(Default)]
pub struct Report {
    dir: Option<PathBuf>,
    filename: Option<String>,
    pub text: String,
    pub unread: bool,
    pub open: bool,
    pub request: Option<Request>,
    pub error: Option<String>,
}
impl Report {
    pub fn load(dir: PathBuf) -> Self {
        let seen = std::fs::read_to_string(dir.join("seen")).unwrap_or_default();
        let latest = super::files(&dir, "crash-")
            .into_iter()
            .rev()
            .find_map(|path| {
                let text = super::read_report(&path)?;
                let name = path.file_name()?.to_str()?.to_owned();
                Some((name, text))
            });
        let mut report = Self {
            dir: Some(dir),
            ..Default::default()
        };
        if let Some((name, text)) = latest {
            report.unread = name != seen;
            report.filename = Some(name);
            report.text = text;
        }
        report
    }
    pub fn close(&mut self) {
        self.open = false;
        self.unread = false;
        if let (Some(dir), Some(name)) = (&self.dir, &self.filename) {
            let _ = std::fs::write(dir.join("seen"), name);
        }
    }
    pub fn indicator(&mut self, ui: &mut egui::Ui, lang: Lang) {
        if self.unread
            && ui
                .button("!")
                .on_hover_text(lang.pick("クラッシュの報告", "Crash Report"))
                .clicked()
        {
            self.open = true;
        }
    }
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang) {
        if !self.open {
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new(lang.pick("クラッシュの報告", "Crash Report")).id(egui::Id::new("crash-report")).open(&mut open).default_width(650.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.text.as_str()).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
            });
            if let Some(error) = &self.error { ui.label(error); }
            ui.horizontal(|ui| {
                if ui.button(lang.pick("コピー", "Copy")).clicked() { ctx.copy_text(self.text.clone()); }
                if ui.button(lang.pick("フォルダを開く", "Open Folder")).clicked() { self.request = Some(Request::Folder); }
                if ui.button(lang.pick("GitHub に報告", "Report on GitHub")).on_hover_text(lang.pick("自動送信はありません。本文はコピーして貼り付けてください", "Nothing is sent automatically. Copy and paste the report into the issue")).clicked() { self.request = Some(Request::GitHub); }
                if ui.button(lang.pick("閉じる", "Close")).clicked() { close = true; }
            });
        });
        if !open || close {
            self.close();
        }
    }
    pub fn execute_request(&mut self, lang: Lang) {
        let result = match self.request.take() {
            Some(Request::Folder) => self
                .dir
                .clone()
                .or_else(super::directory)
                .ok_or_else(|| std::io::Error::other("no settings directory"))
                .and_then(|dir| super::open_folder(&dir)),
            Some(Request::GitHub) => crate::update::launch::open_page(&issue_url()),
            None => return,
        };
        self.error = result
            .err()
            .map(|_| lang.pick("開けません", "Cannot open").into());
    }
}

pub fn issue_url() -> String {
    fn encode(text: &str) -> String {
        text.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    }
    format!(
        "https://github.com/YozoraKurage/YoluPainter/issues/new?title={}&body={}",
        encode("Crash report"),
        encode(&format!(
            "Version: {}\nOS: {} {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    )
}
