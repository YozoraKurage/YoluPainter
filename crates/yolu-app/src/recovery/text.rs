//! 復旧の画面の言葉（日本語と英語）。名前・状態・短い理由だけで、説明はツールチップに置く。

use std::time::Duration;

use yolu_io::StoreError;

use super::{RecoveryError, Usage};
use crate::lang::Lang;

/// 復旧から開いたプロジェクトの名前（保存していない）。
pub fn recovered_name(lang: Lang) -> &'static str {
    lang.t("recovery-untitled-recovered")
}

impl Lang {
    /// 復旧の失敗の短い理由。日本語は診断をそのまま、英語は種類ごとの短い文にする。
    pub fn recovery_error(self, error: &RecoveryError) -> String {
        match error {
            RecoveryError::Store(e) => self.store_error(e),
            RecoveryError::Project(e) => self.io_error(e),
            RecoveryError::Io(e) => self.file_error(e),
            RecoveryError::Text(text) => text.clone(),
            RecoveryError::Panicked => self.t("recovery-internal-error-while-writing").into(),
            RecoveryError::NotPool => self.t("recovery-not-a-generation-in-the").into(),
        }
    }

    pub fn store_error(self, error: &StoreError) -> String {
        match error {
            StoreError::Io(e) => self.file_error(e),
            StoreError::LowSpace(_) => self.t("recovery-low-disk-space").into(),
            _ if self == Self::Ja => error.to_string(),
            StoreError::NoGeneration => "No generation".into(),
            StoreError::AlreadyExists => "The folder already has a generation".into(),
            StoreError::Changed(_) => "The recovery folder was changed outside".into(),
            StoreError::Busy => "Another writer is using the recovery folder".into(),
            StoreError::Corrupt(_) => "Damaged generation".into(),
            StoreError::Budget(why) => crate::lang::budget_text(why)
                .unwrap_or("Size limit exceeded")
                .into(),
            StoreError::InvalidArgument(_) => "Invalid setting".into(),
        }
    }

    /// 書き置きに失敗したときの状態の帯の文。空きが少なくて書かなかったときは、失敗ではなく見送り。
    pub fn recovery_failed(self, error: &RecoveryError) -> String {
        if matches!(error, RecoveryError::Store(StoreError::LowSpace(_))) {
            return self.with_reason(
                self.t("recovery-recovery-checkpoint-skipped"),
                self.recovery_error(error),
            );
        }
        self.with_reason(
            self.t("recovery-recovery-checkpoint-failed"),
            self.recovery_error(error),
        )
    }

    pub fn recovery_working_again(self) -> &'static str {
        self.t("recovery-recovery-checkpoints-are-working-again")
    }

    pub fn recovery_cannot_open(self, error: &RecoveryError) -> String {
        self.with_reason(
            self.t("recovery-cannot-open-the-recovery"),
            self.recovery_error(error),
        )
    }

    /// 設定のファイルの読み込みで気づいたこと（初めの 1 つだけを短く）。
    pub fn recovery_settings_problem(self, problem: &super::settings::Problem) -> String {
        use super::settings::Problem;
        match problem {
            Problem::OutOfRange { key, value, range } => self.f(
                "recovery-recovery-setting-is-out-of",
                &[
                    ("key", &key),
                    ("value", &value),
                    ("range", &range.0),
                    ("range_2", &range.1),
                ],
            ),
            Problem::RelativeDirectory(dir) => {
                self.f("recovery-recovery-folder-is-not-absolute", &[("dir", &dir)])
            }
            Problem::Unreadable(reason) => {
                let reason = self.file_error(&reason.to_error());
                self.f(
                    "recovery-generations-are-not-trimmed-because",
                    &[("reason", &reason)],
                )
            }
            Problem::PreviousRun(reason) => {
                let reason = self.file_error(&reason.to_error());
                self.with_reason(
                    self.t("recovery-cannot-settle-the-previous-recovery"),
                    reason,
                )
            }
        }
    }

    pub fn recovery_unavailable(self, error: &RecoveryError) -> String {
        self.with_reason(
            self.t("recovery-recovery-is-unavailable"),
            self.recovery_error(error),
        )
    }

    /// ウィンドウの下の帯に出す、復旧が使っている量（短く。内訳はツールチップ）。
    pub fn recovery_usage_text(self, usage: &Usage) -> String {
        let used = bytes_text(usage.total());
        self.f("recovery-in-use", &[("used", &used)])
    }

    /// 使っている量の内訳（ツールチップ）。`cap` は上限、`free` はディスクの空き（分からなければ None）。
    pub fn recovery_usage_tip(self, usage: &Usage, cap: u64, free: Option<u64>) -> String {
        let mut lines = vec![
            self.t("recovery-disk-space-used-by-recovery-tip")
                .to_owned(),
            format!(
                "{}  {}",
                self.t("recovery-this-session-tip"),
                bytes_text(usage.own)
            ),
            format!(
                "{}  {}",
                self.t("recovery-crashed-sessions-tip"),
                bytes_text(usage.crashed)
            ),
            format!(
                "{}  {}",
                self.t("recovery-closed-sessions-tip"),
                bytes_text(usage.closed)
            ),
        ];
        if usage.others > 0 {
            lines.push(format!(
                "{}  {}",
                self.t("recovery-other-windows-tip"),
                bytes_text(usage.others)
            ));
        }
        lines.push(format!(
            "{}  {}",
            self.t("recovery-limit-tip"),
            bytes_text(cap)
        ));
        if let Some(free) = free {
            lines.push(format!(
                "{}  {}",
                self.t("recovery-free-on-disk-tip"),
                bytes_text(free)
            ));
        }
        lines.join("\n")
    }

    /// 世代の経過時間（一覧の右の列）。
    pub fn age_text(self, age: Duration) -> String {
        let s = age.as_secs();
        match s {
            0..=9 => self.t("recovery-just-now").into(),
            10..=59 => self.f("recovery-s-ago", &[("s", &s)]),
            60..=3599 => self.f("recovery-min-ago", &[("s", &(s / 60))]),
            3600..=86399 => self.f("recovery-h-ago", &[("s", &(s / 3600))]),
            _ => self.f("recovery-d-ago", &[("s", &(s / 86400))]),
        }
    }

    pub fn sets_text(self, count: usize) -> String {
        self.pick(
            format!("{count} セット"),
            if count == 1 {
                "1 set".to_owned()
            } else {
                format!("{count} sets")
            },
        )
    }
}

/// バイト数を短く（GB は 10 未満で小数 1 桁。1 GB = 1024 MB）。
pub fn bytes_text(bytes: u64) -> String {
    const KIB: u64 = 1 << 10;
    const MIB: u64 = 1 << 20;
    const GIB: u64 = 1 << 30;
    if bytes >= GIB {
        let gb = bytes as f64 / GIB as f64;
        if gb < 10.0 {
            format!("{gb:.1} GB")
        } else {
            format!("{gb:.0} GB")
        }
    } else if bytes >= MIB {
        format!("{} MB", (bytes + MIB / 2) / MIB)
    } else {
        format!("{} KB", bytes / KIB)
    }
}

/// 設定の名前のツールチップ（`key` は "interval"・"keep"・"disk"）。いま選んでいる値は帯に出ているので、繰り返さない。
pub fn settings_tip(lang: Lang, key: &str) -> &'static str {
    match key {
        "interval" => lang.t("recovery-time-from-a-change-to-tip"),
        "disk" => lang.t("recovery-how-much-disk-space-the-tip"),
        _ => lang.t("recovery-number-of-generations-to-keep-tip"),
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
                assert!(
                    !text.chars().any(|c| c.is_ascii_digit()),
                    "いま選んでいる値は帯に出ている。繰り返さない: {text}"
                );
            }
            assert_eq!(interval.is_ascii() && keep.is_ascii(), lang == Lang::En);
        }
    }

    #[test]
    fn byte_amounts_are_short_and_use_the_units_people_see_in_the_file_manager() {
        assert_eq!(bytes_text(0), "0 KB");
        assert_eq!(bytes_text(300 * 1024), "300 KB");
        assert_eq!(bytes_text(5 * (1 << 20)), "5 MB");
        assert_eq!(
            bytes_text((1 << 30) - 1),
            "1024 MB",
            "GB の手前は MB のまま（丸めて 1.0 GB にしない）"
        );
        assert_eq!(bytes_text(1 << 30), "1.0 GB");
        assert_eq!(bytes_text(3 * (1 << 29)), "1.5 GB");
        assert_eq!(bytes_text(12 * (1 << 30)), "12 GB");
    }

    #[test]
    fn the_usage_reads_short_in_both_languages_and_the_tooltip_breaks_it_down() {
        let usage = Usage {
            own: 3 << 20,
            crashed: 2 << 30,
            closed: 1 << 20,
            others: 0,
        };
        assert_eq!(Lang::Ja.recovery_usage_text(&usage), "使用中 2.0 GB");
        assert_eq!(Lang::En.recovery_usage_text(&usage), "2.0 GB in use");
        let tip = Lang::En.recovery_usage_tip(&usage, 2 << 30, Some(50 << 30));
        for line in [
            "This session  3 MB",
            "Crashed sessions  2.0 GB",
            "Closed sessions  1 MB",
            "Limit  2.0 GB",
            "Free on disk  50 GB",
        ] {
            assert!(tip.contains(line), "{line}: {tip}");
        }
        assert!(
            !tip.contains("Other windows"),
            "別のウィンドウが無ければ出さない"
        );
        assert!(
            !Lang::En
                .recovery_usage_tip(&usage, 1, None)
                .contains("Free on disk"),
            "空きが分からなければ出さない"
        );
        let other = Usage {
            others: 5 << 20,
            ..usage
        };
        assert!(Lang::Ja
            .recovery_usage_tip(&other, 1, None)
            .contains("ほかのウィンドウ  5 MB"));
        for lang in Lang::ALL {
            let tip = lang.recovery_usage_tip(&other, 2 << 30, Some(1 << 30));
            assert_eq!(has_japanese(&tip), lang == Lang::Ja, "{tip}");
        }
    }

    fn has_japanese(text: &str) -> bool {
        text.chars().any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'))
    }

    #[test]
    fn a_skipped_checkpoint_is_a_short_reason_without_numbers_not_a_failure() {
        let low = RecoveryError::Store(StoreError::LowSpace(yolu_io::LowSpace {
            available: 5,
            needed: 9,
            reserve: 7,
        }));
        for lang in Lang::ALL {
            let text = lang.recovery_failed(&low);
            assert!(
                !text.chars().any(|c| c.is_ascii_digit()),
                "状態の帯に数を出さない: {text}"
            );
            assert_eq!(has_japanese(&text), lang == Lang::Ja, "{text}");
            assert!(
                !text.contains(lang.pick("失敗", "failed")),
                "見送りであって失敗ではない: {text}"
            );
        }
    }
}
