//! 置き場のあるディスクの空き（書く前の守り）。
//!
//! 書くと空きが「空けておく量」（`reserve`: 1 GiB と、ボリュームの 5% の大きいほう。ただし 10 GiB まで）を割るなら、その書き置きは書かない
//! （`yolu_io::GenerationStore::with_space_guard`。理由を出すだけで描くのは止めない）。空きが分からないボリューム・OS では
//! 確かめず、書く（分からないことで保存を止めない）。試験は `RecoveryState::set_space_probe` で空きを偽る。

use std::path::Path;
use std::sync::Arc;

use yolu_io::{LowSpace, SpaceGuard};

const GIB: u64 = 1 << 30;
/// 空けておく量の上限。容量の大きいボリュームで 5% が何十 GiB にもなって、空きに余裕があるのに書かなくなるのを防ぐ。
const RESERVE_MAX: u64 = 10 * GIB;

/// ボリュームの容量と、使える空き（バイト）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskSpace {
    pub total: u64,
    pub available: u64,
}

/// パスのあるボリュームの空きを答える口。分からなければ None。
pub type SpaceProbe = Arc<dyn Fn(&Path) -> Option<DiskSpace> + Send + Sync>;

/// 書いたあとも空けておく量: 1 GiB と、ボリュームの 5% の大きいほう（ただし 10 GiB まで）。
pub fn reserve(total: u64) -> u64 {
    GIB.max((total / 20).min(RESERVE_MAX))
}

/// 本物の空きの確かめ（OS に聞く）。
pub fn system_probe() -> SpaceProbe {
    Arc::new(probe_system)
}

/// 書く前の守り。新しく書くバイト数を足しても、空きが `reserve` を割らなければ通す。
pub(crate) fn guard(probe: SpaceProbe, root: std::path::PathBuf) -> SpaceGuard {
    Arc::new(move |needed| {
        let Some(disk) = probe(&root) else {
            return Ok(());
        };
        let reserve = reserve(disk.total);
        if disk.available.saturating_sub(needed) < reserve {
            Err(LowSpace {
                available: disk.available,
                needed,
                reserve,
            })
        } else {
            Ok(())
        }
    })
}

/// まだ無いパスは、あるところまで遡ってボリュームを見る。
fn existing_ancestor(path: &Path) -> Option<&Path> {
    path.ancestors()
        .find(|p| !p.as_os_str().is_empty() && p.exists())
}

// statvfs の欄の型は、64 ビットの Linux では u64、32 ビットでは u32。どちらでも同じ書き方で u64 にする
#[cfg(target_os = "linux")]
#[allow(clippy::unnecessary_cast)]
fn probe_system(path: &Path) -> Option<DiskSpace> {
    use std::os::unix::ffi::OsStrExt;
    let target = std::ffi::CString::new(existing_ancestor(path)?.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: `target` は NUL 終端の C 文字列、`stat` は書き込める statvfs の領域（statvfs(3) の呼び方どおり）。成功したときだけ読む。
    let status = unsafe { libc::statvfs(target.as_ptr(), stat.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    // SAFETY: statvfs が 0 を返したので、全部の欄が書かれている。
    let stat = unsafe { stat.assume_init() };
    let block = stat.f_frsize as u64;
    Some(DiskSpace {
        total: (stat.f_blocks as u64).saturating_mul(block),
        available: (stat.f_bavail as u64).saturating_mul(block),
    })
}

#[cfg(windows)]
fn probe_system(path: &Path) -> Option<DiskSpace> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = existing_ancestor(path)?
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let (mut available, mut total) = (0u64, 0u64);
    // SAFETY: `wide` は NUL 終端の UTF-16、2 つの出力は書き込める u64（GetDiskFreeSpaceExW の呼び方どおり。呼び出したユーザーが使える空きを返す）。
    unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide.as_ptr()),
            Some(&mut available as *mut u64),
            Some(&mut total as *mut u64),
            None,
        )
        .ok()?;
    }
    Some(DiskSpace { total, available })
}

#[cfg(not(any(target_os = "linux", windows)))]
fn probe_system(path: &Path) -> Option<DiskSpace> {
    let _ = existing_ancestor(path);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reserve_is_one_gib_or_five_percent_whichever_is_larger() {
        assert_eq!(reserve(10 * GIB), GIB);
        assert_eq!(reserve(20 * GIB), GIB);
        assert_eq!(reserve(100 * GIB), 5 * GIB);
        assert_eq!(reserve(0), GIB);
        assert_eq!(
            reserve(1000 * GIB),
            10 * GIB,
            "容量が大きくても 10 GiB まで"
        );
        assert_eq!(reserve(u64::MAX), 10 * GIB);
    }

    #[test]
    fn the_guard_refuses_only_when_the_write_would_drop_the_free_space_below_the_reserve() {
        let at = |available: u64| -> SpaceGuard {
            guard(
                Arc::new(move |_| {
                    Some(DiskSpace {
                        total: 100 * GIB,
                        available,
                    })
                }),
                "x".into(),
            )
        };
        // 予約は 5 GiB。ちょうどその空きが残るなら通す
        assert!(at(5 * GIB + 10)(10).is_ok());
        let refused = at(5 * GIB + 9)(10).unwrap_err();
        assert_eq!(
            refused,
            LowSpace {
                available: 5 * GIB + 9,
                needed: 10,
                reserve: 5 * GIB
            }
        );
        // 書くものが無くても、すでに予約を割っているあいだは書かない
        assert!(at(5 * GIB - 1)(0).is_err());
        // 空きが分からなければ止めない
        assert!(guard(Arc::new(|_| None), "x".into())(u64::MAX).is_ok());
    }

    #[test]
    fn the_system_reports_a_volume_for_an_existing_folder_and_for_one_that_is_not_there_yet() {
        let here = std::env::temp_dir();
        let Some(disk) = probe_system(&here) else {
            // 空きを答えない OS では、何も確かめないだけ
            return;
        };
        assert!(disk.total > 0 && disk.available <= disk.total);
        let later = probe_system(&here.join("yolu-space-not-there").join("deeper"))
            .expect("遡って同じボリュームを見る");
        assert_eq!(later.total, disk.total);
    }
}
