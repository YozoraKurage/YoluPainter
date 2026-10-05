//! 状態の帯の右端に出す、版・ビルドと、使っているメモリの量（ユーザーの頼み。開発用の数を画面に出さない決まりの、この 2 つだけの例外）。
//!
//! - 版とビルド: Cargo の版と、ビルドに埋めた git の短い ID（`build.rs`。無ければ出さない）。試験の窓は出さない（画像がコミットごとに
//!   変わらないように）。実際の窓だけが `Usage::build` に入れる。
//! - 使っているメモリ: このプロセスの実メモリ（Windows は作業セット、Linux は VmRSS。分からない OS は出さない）。GPU は wgpu の確保済みの
//!   量が分かるときだけ（Vulkan・D3D12。OpenGL は分からない）。数は短く（「812 MB」「1.4 GB」）、内訳はツールチップ。
//!   測るのは実際の窓だけで、1.5 秒おき（毎フレームは測らない）。試験は自分で値を入れる。

use crate::lang::Lang;
use crate::state::AppState;

/// 測る間隔（秒）。
pub const INTERVAL: f64 = 1.5;

/// 状態の帯の右端が出す値。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    /// 版とビルド（「0.1.0 · a1b2c3d」。実際の窓だけ）。
    pub build: Option<String>,
    /// このプロセスの実メモリ（バイト。分からなければ None）。
    pub process: Option<u64>,
    /// GPU の確保済みのメモリ（バイト。分からなければ None）。
    pub gpu: Option<u64>,
    /// 全テクスチャセットのレイヤーのメモリ（バイト）。
    pub layers: u64,
    /// 全テクスチャセットの取り消しの履歴（バイト）。
    pub history: u64,
    /// 最後に測った時刻（egui の時刻。まだ測っていなければ None）。
    pub sampled_at: Option<f64>,
}

impl Usage {
    /// 測る頃か。
    pub fn due(&self, now: f64) -> bool {
        self.sampled_at.is_none_or(|at| now - at >= INTERVAL || now < at)
    }
}

/// 版とビルドの文字（「0.1.0 · a1b2c3d」。ビルドの ID が埋まっていなければ版だけ）。
pub fn build_label() -> String {
    match option_env!("YOLU_GIT_REV") {
        Some(rev) if !rev.is_empty() => format!("{} · {rev}", env!("CARGO_PKG_VERSION")),
        _ => env!("CARGO_PKG_VERSION").to_owned(),
    }
}

/// 版の文字（「0.4.0-rc.1」や、「0.4.0-rc.1 · a1b2c3d」の先頭）が試験版（`alpha.N`・`beta.N`・`rc.N` の版）か。
fn is_beta_label(text: &str) -> bool {
    text.split_whitespace()
        .next()
        .and_then(|version| yolu_update::Version::parse(version).ok())
        .is_some_and(|version| yolu_update::is_beta_version(&version))
}

/// 「について」の知らせ。試験版には、版のあとに試験版の印を付ける（正式版は製品名と版だけ）。
pub fn about_text(lang: Lang, version: &str) -> String {
    if is_beta_label(version) {
        format!("YoluPainter {version}{}", lang.pick("（試験版）", " (beta)"))
    } else {
        format!("YoluPainter {version}")
    }
}

/// 量の短い文字（1 GB 未満は「812 MB」、以上は「1.4 GB」。1 MB 未満も「0 MB」でなく「1 MB」から）。
pub fn format_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = bytes as f64 / MB;
    if mb < 1024.0 {
        format!("{} MB", (mb.round() as u64).max(1))
    } else {
        format!("{:.1} GB", mb / 1024.0)
    }
}

/// このプロセスの実メモリ（バイト）。測れない OS は None。
pub fn process_bytes() -> Option<u64> {
    platform::process_bytes()
}

#[cfg(target_os = "linux")]
mod platform {
    pub fn process_bytes() -> Option<u64> {
        resident_from_status(&std::fs::read_to_string("/proc/self/status").ok()?)
    }

    /// `/proc/self/status` の VmRSS（kB）をバイトに。
    pub fn resident_from_status(text: &str) -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with("VmRSS:"))?;
        let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kib * 1024)
    }
}

#[cfg(windows)]
mod platform {
    pub fn process_bytes() -> Option<u64> {
        use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
        use windows::Win32::System::Threading::GetCurrentProcess;
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        // SAFETY: cb を設定した PROCESS_MEMORY_COUNTERS と、自分のプロセスの疑似ハンドルを渡す（Win32 の呼び方どおり）。
        let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
        ok.as_bool().then_some(counters.WorkingSetSize as u64)
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    pub fn process_bytes() -> Option<u64> {
        None
    }
}

/// 試験が、`/proc/self/status` の読み方を確かめる。
#[cfg(target_os = "linux")]
pub fn resident_from_status(text: &str) -> Option<u64> {
    platform::resident_from_status(text)
}

impl AppState {
    /// 時刻 `now`（egui の時刻）に、測る頃なら使っているメモリを測り直す。`gpu` は wgpu の確保済みの量（分からなければ None）を
    /// 返す関数（測る頃にだけ呼ぶ）。測ったら true。
    pub fn refresh_usage(&mut self, now: f64, gpu: impl FnOnce() -> Option<u64>) -> bool {
        if !self.usage.due(now) {
            return false;
        }
        let (mut layers, mut history) = (0u64, 0u64);
        let current = self.sets.current_index();
        for i in 0..self.sets.len() {
            let doc = if i == current { &self.doc } else { self.set_doc(i) };
            layers = layers.saturating_add(doc.allocated_bytes());
            history = history.saturating_add(doc.history_bytes());
        }
        self.usage = Usage {
            build: self.usage.build.clone(),
            process: process_bytes(),
            gpu: gpu(),
            layers,
            history,
            sampled_at: Some(now),
        };
        true
    }
}

/// 状態の帯の右端に並べる 1 つの項目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// 部品の名前（ツールの識別）。
    pub key: &'static str,
    pub text: String,
    pub tip: String,
}

impl Usage {
    /// 右端から左へ並べる項目（版とビルド・試験版の印（試験版のときだけ）・GPU・メモリの順。測れていない値は無い）。
    pub fn items(&self, lang: Lang) -> Vec<Item> {
        let mut items = Vec::new();
        if let Some(build) = &self.build {
            items.push(Item {
                key: "build",
                text: build.clone(),
                tip: lang.pick("版とビルド", "Version and build").to_owned(),
            });
            // 試験版には、版の左に印（「試験版」「Beta」）。正式版は何も足さない。
            if is_beta_label(build) {
                items.push(Item {
                    key: "beta",
                    text: lang.pick("試験版", "Beta").to_owned(),
                    tip: lang
                        .pick("試験版（正式版より前の版）", "Beta version (before the stable release)")
                        .to_owned(),
                });
            }
        }
        if self.process.is_some() || self.gpu.is_some() {
            let tip = tooltip(self, lang);
            if let Some(gpu) = self.gpu {
                items.push(Item { key: "gpu", text: format!("GPU {}", format_size(gpu)), tip: tip.clone() });
            }
            if let Some(bytes) = self.process {
                items.push(Item { key: "memory", text: format_size(bytes), tip });
            }
        }
        items
    }
}

/// 使っているメモリのツールチップ（内訳。分かる行だけ）。
pub fn tooltip(usage: &Usage, lang: Lang) -> String {
    let mut lines = Vec::new();
    if let Some(bytes) = usage.process {
        lines.push(format!("{}: {}", lang.pick("アプリ全体（実メモリ）", "App (resident)"), format_size(bytes)));
    }
    lines.push(format!("{}: {}", lang.pick("レイヤーのメモリ", "Layer memory"), format_size(usage.layers)));
    lines.push(format!("{}: {}", lang.pick("取り消しの履歴", "Undo history"), format_size(usage.history)));
    if let Some(bytes) = usage.gpu {
        lines.push(format!("GPU: {}", format_size(bytes)));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_short_and_never_zero() {
        assert_eq!(format_size(0), "1 MB");
        assert_eq!(format_size(300 * 1024), "1 MB");
        assert_eq!(format_size(812 * 1024 * 1024), "812 MB");
        assert_eq!(format_size(1023 * 1024 * 1024 + 600 * 1024), "1024 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_size(1536 * 1024 * 1024), "1.5 GB");
        assert_eq!(format_size(12 * 1024 * 1024 * 1024), "12.0 GB");
    }

    #[test]
    fn the_build_label_has_the_version_and_the_short_commit_when_embedded() {
        let label = build_label();
        assert!(label.starts_with(env!("CARGO_PKG_VERSION")), "{label}");
        match option_env!("YOLU_GIT_REV") {
            Some(rev) if !rev.is_empty() => assert!(label.ends_with(&format!(" · {rev}")), "{label}"),
            _ => assert_eq!(label, env!("CARGO_PKG_VERSION")),
        }
    }

    #[test]
    fn a_beta_version_carries_a_mark_in_the_status_band_and_the_about_text() {
        let usage = |build: &str| Usage { build: Some(build.into()), ..Usage::default() };
        for lang in Lang::ALL {
            let mark = lang.pick("試験版", "Beta");
            // 試験版: 版とビルドの左に印が付く（右端が版とビルド）
            let items = usage("0.4.0-rc.1 · a1b2c3d").items(lang);
            let keys: Vec<_> = items.iter().map(|i| (i.key, i.text.as_str())).collect();
            assert_eq!(keys, [("build", "0.4.0-rc.1 · a1b2c3d"), ("beta", mark)], "{lang:?}");
            assert!(!items[1].tip.is_empty());
            // 版だけ（ビルドの ID が埋まっていない）でも同じ
            assert_eq!(usage("0.4.0-beta.2").items(lang).len(), 2);
            // 正式版・試験版の形でないプレリリース・測っていない窓には、印を足さない
            for plain in ["0.4.0 · a1b2c3d", "0.4.0", "0.4.0-preview.1 · a1b2c3d", "0.4.0-rc1"] {
                assert_eq!(usage(plain).items(lang).len(), 1, "{plain}");
            }
            assert!(Usage::default().items(lang).is_empty());
        }
        assert_eq!(about_text(Lang::Ja, "0.4.0-rc.1"), "YoluPainter 0.4.0-rc.1（試験版）");
        assert_eq!(about_text(Lang::En, "0.4.0-rc.1"), "YoluPainter 0.4.0-rc.1 (beta)");
        for lang in Lang::ALL {
            assert_eq!(about_text(lang, "0.4.0"), "YoluPainter 0.4.0");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_resident_size_is_read_from_the_status_file() {
        assert_eq!(resident_from_status("Name:\tx\nVmRSS:\t   2048 kB\nThreads:\t3\n"), Some(2048 * 1024));
        assert_eq!(resident_from_status("VmRSS: many kB\n"), None);
        assert_eq!(resident_from_status("Name: x\n"), None);
        // 自分のプロセスは測れて、0 ではない
        assert!(process_bytes().is_some_and(|b| b > 1024 * 1024));
    }

    #[test]
    fn it_is_measured_at_the_interval_and_not_every_frame() {
        let mut usage = Usage::default();
        assert!(usage.due(10.0), "まだ測っていなければ測る");
        usage.sampled_at = Some(10.0);
        assert!(!usage.due(10.5));
        assert!(!usage.due(11.4));
        assert!(usage.due(11.6));
        assert!(usage.due(5.0), "時刻が戻ったら（新しい窓）測り直す");
    }
}
