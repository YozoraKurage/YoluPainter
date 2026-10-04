//! 復旧の画面の言葉（日本語と英語）。名前・状態・短い理由だけで、説明はツールチップに置く。

use std::time::Duration;

use yolu_io::StoreError;

use super::RecoveryError;
use crate::lang::Lang;

/// 復旧から開いたプロジェクトの名前（保存していない）。
pub fn recovered_name(lang: Lang) -> &'static str {
    lang.pick("名称未設定（復旧）", "Untitled (Recovered)")
}

impl Lang {
    /// 復旧の失敗の短い理由。日本語は診断をそのまま、英語は種類ごとの短い文にする。
    pub fn recovery_error(self, error: &RecoveryError) -> String {
        match error {
            RecoveryError::Store(e) => self.store_error(e),
            RecoveryError::Project(e) => self.io_error(e),
            RecoveryError::Io(e) => self.file_error(e),
            RecoveryError::Text(text) => text.clone(),
            RecoveryError::Panicked => self.pick("書き込みの内部の失敗", "Internal error while writing").into(),
            RecoveryError::NotPool => self.pick(
                "復旧の置き場の中の世代ではありません",
                "Not a generation in the recovery folder",
            )
            .into(),
        }
    }

    pub fn store_error(self, error: &StoreError) -> String {
        match error {
            StoreError::Io(e) => self.file_error(e),
            _ if self == Self::Ja => error.to_string(),
            StoreError::NoGeneration => "No generation".into(),
            StoreError::AlreadyExists => "The folder already has a generation".into(),
            StoreError::Changed(_) => "The recovery folder was changed outside".into(),
            StoreError::Busy => "Another writer is using the recovery folder".into(),
            StoreError::Corrupt(_) => "Damaged generation".into(),
            StoreError::Budget(_) => "Size limit exceeded".into(),
            StoreError::InvalidArgument(_) => "Invalid setting".into(),
        }
    }

    /// 書き置きに失敗したときの状態の帯の文。
    pub fn recovery_failed(self, error: &RecoveryError) -> String {
        format!(
            "{}: {}",
            self.pick("復旧用の書き置きに失敗", "Recovery checkpoint failed"),
            self.recovery_error(error)
        )
    }

    pub fn recovery_working_again(self) -> &'static str {
        self.pick(
            "復旧用の書き置きが戻りました",
            "Recovery checkpoints are working again",
        )
    }

    pub fn recovery_cannot_open(self, error: &RecoveryError) -> String {
        format!(
            "{}: {}",
            self.pick("復旧を開けません", "Cannot open the recovery"),
            self.recovery_error(error)
        )
    }

    /// 設定のファイルの読み込みで気づいたこと（初めの 1 つだけを短く）。
    pub fn recovery_settings_problem(self, problem: &super::settings::Problem) -> String {
        use super::settings::Problem;
        match problem {
            Problem::OutOfRange { key, value, range } => self.pick(
                format!("復旧の設定 {key}={value} は範囲外（{}〜{}）。既定を使います", range.0, range.1),
                format!("Recovery setting {key}={value} is out of range ({}–{}); using the default", range.0, range.1),
            ),
            Problem::RelativeDirectory(dir) => self.pick(
                format!("復旧の置き場 {dir} は絶対パスではありません。既定を使います"),
                format!("Recovery folder {dir} is not absolute; using the default"),
            ),
            Problem::Unreadable(reason) => {
                let reason = self.file_error(&reason.to_error());
                self.pick(
                    format!("復旧の設定を読めません: {reason}。世代は整理しません"),
                    format!("Cannot read the recovery settings: {reason}. Generations are not trimmed"),
                )
            }
            Problem::PreviousRun(reason) => {
                let reason = self.file_error(&reason.to_error());
                self.pick(
                    format!("前回の復旧の印を片付けられません: {reason}"),
                    format!("Cannot settle the previous recovery marker: {reason}"),
                )
            }
        }
    }

    pub fn recovery_unavailable(self, error: &RecoveryError) -> String {
        format!(
            "{}: {}",
            self.pick("復旧を使えません", "Recovery is unavailable"),
            self.recovery_error(error)
        )
    }

    /// 世代の経過時間（一覧の右の列）。
    pub fn age_text(self, age: Duration) -> String {
        let s = age.as_secs();
        match s {
            0..=9 => self.pick("たった今", "just now").into(),
            10..=59 => self.pick(format!("{s} 秒前"), format!("{s} s ago")),
            60..=3599 => self.pick(format!("{} 分前", s / 60), format!("{} min ago", s / 60)),
            3600..=86399 => self.pick(format!("{} 時間前", s / 3600), format!("{} h ago", s / 3600)),
            _ => self.pick(format!("{} 日前", s / 86400), format!("{} d ago", s / 86400)),
        }
    }

    pub fn sets_text(self, count: usize) -> String {
        self.pick(
            format!("{count} セット"),
            if count == 1 { "1 set".to_owned() } else { format!("{count} sets") },
        )
    }
}

/// 設定の名前のツールチップ（`key` は "interval"・"keep"）。いま選んでいる値は帯に出ているので、繰り返さない。
pub fn settings_tip(lang: Lang, key: &str) -> &'static str {
    match key {
        "interval" => lang.pick(
            "変更があってから書き置くまでの時間",
            "Time from a change to its checkpoint",
        ),
        _ => lang.pick(
            "残しておく世代の数（超えた分は古いものから整理）",
            "Number of generations to keep (older ones are trimmed)",
        ),
    }
}

/// UTC の時刻の文字列（ツールチップ）。世代の名前の時刻（UTC の Unix ミリ秒）から。
pub fn utc_text(ms: u64) -> String {
    let stamp = yolu_io::utc_stamp(ms);
    format!(
        "{}-{}-{} {}:{}:{} UTC",
        &stamp[0..4],
        &stamp[4..6],
        &stamp[6..8],
        &stamp[9..11],
        &stamp[11..13],
        &stamp[13..15]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_tooltips_name_the_setting_without_repeating_its_value() {
        for lang in Lang::ALL {
            let (interval, keep) = (settings_tip(lang, "interval"), settings_tip(lang, "keep"));
            assert_ne!(interval, keep, "ラベルごとに別の文");
            for text in [interval, keep] {
                assert!(!text.chars().any(|c| c.is_ascii_digit()), "いま選んでいる値は帯に出ている。繰り返さない: {text}");
            }
            assert_eq!(interval.is_ascii() && keep.is_ascii(), lang == Lang::En);
        }
    }
}
