//! GPU のメモリの配り方。利用者が選んだ「自動・低・標準・高」か MiB の合計を、GPU 側の 3 つの予算へ配る。
//!
//! - **3D の絵**（`view3d::paint`: 今のテクスチャセットとほかのセットのテクスチャ。今のセットを引いた残りに、遠いセットから落として収める。選択範囲の
//!   縁・赤い重ね・選択ペンの被覆（`view3d::selection_overlay`。文書と同じ大きさの R8 のミップつき）も、今のセットの絵の次に入れ、入らなければ出さない）
//! - **キャンバスの GPU の合成**（`canvas::gpu`: 表示のテクスチャと作業域の常駐の予算）
//! - **棚のサムネイル**（`shelf`: 素材を展開して見てよい量）
//!
//! **合計に入らない勘定**: 3D ビューの面の描き先（多サンプルの色・深度・HDR・ブルームの段。見積もりは `view3d::render::target_bytes`）は、
//! この 3 つとは別に、3D の絵の取り分（[`Budgets::paint`]）と同じ量を上限にする。上限を超える大きさの組み合わせはサンプル数を下げて収める
//! （1× は上限を超えても使う）。合計から配るのではなく外へ足すので、描き先が上限まで載ると、設定の合計に加えてその量
//! （標準で 512 MiB。4K の HDR＋ブルームの 4× で約 496 MiB）まで使う。絵の取り分から引かないのは、ウィンドウの大きさやサンプル数を替えるたびに
//! 絵が縮み直すため。影のマップなどの小さな持ち物も、この 3 つの予算には数えない。キャンバスの GPU の合成が 1 回の更新で上げる一時の
//! 入れ物（GPU の 2 つと CPU の並び）は合計の中: 1 束ぶんは常駐の固定の量に数え、それを超える分はキャンバスの取り分の空きの 1/3 まで
//! （64 MiB を天井）に収めて、超える前に分けて流す（`yolu_gpu::ResidentOptions::transfer_limit_bytes`）。
//!
//! 配り方の表は [`SHARES`] の 1 か所。標準の合計（[`STANDARD_MIB`]）は、これまで 3 つの定数だった値の合計
//! （512 + 512 + 128 MiB）で、アダプターから量が分からないときの値でもある。量が分かるとき（今は Windows の DXGI だけ。
//! wgpu はアダプターのメモリの量を返さない）は、標準を量の 1/8（下限は標準の定数、上限 4 GiB）にする。
//! 低・高は標準の半分・2 倍。自動は標準で、量が分かって 3 GiB に満たないときだけ低にする。

use crate::lang::Lang;

/// 1 MiB。
pub const MIB: u64 = 1 << 20;

/// 標準の合計（MiB）。量が分からないときの値で、3D の絵 512・キャンバス 512・棚 128 の合計。
pub const STANDARD_MIB: u64 = 1152;
/// 標準の合計の上限（MiB）。量が分かるときも、これより多くは配らない。
pub const STANDARD_MAX_MIB: u64 = 4096;
/// アダプターのメモリのうち、標準で GPU の予算に回す割合の分母。
const STANDARD_DIVISOR: u64 = 8;
/// 自動は、アダプターの量がこれ（MiB）に満たなければ低にする。
const AUTO_LOW_BELOW_MIB: u64 = 3072;

/// 指定できる合計の範囲（MiB）。
pub const MIN_TOTAL_MIB: u32 = 256;
pub const MAX_TOTAL_MIB: u32 = 32768;
/// 詳しくの指定の刻み（MiB）。
pub const TOTAL_STEP_MIB: u32 = 64;

/// 3 つの予算の割合（3D の絵 : キャンバスの合成 : 棚のサムネイル）。標準の 1152 MiB で 512 : 512 : 128 になる。
pub const SHARES: Shares = Shares {
    paint: 4,
    canvas: 4,
    shelf: 1,
};

/// 配り方の割合。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shares {
    pub paint: u64,
    pub canvas: u64,
    pub shelf: u64,
}

impl Shares {
    fn sum(self) -> u64 {
        self.paint + self.canvas + self.shelf
    }
}

/// 利用者が選ぶ GPU のメモリ。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuMemory {
    /// 標準。アダプターの量が分かって少ないときだけ低。
    #[default]
    Auto,
    Low,
    Standard,
    High,
    /// 詳しくで指定した合計（MiB）。
    Mib(u32),
}

impl GpuMemory {
    /// ウィンドウの選択肢に並べる段（指定した量は並べない）。
    pub const LEVELS: [GpuMemory; 4] = [
        GpuMemory::Auto,
        GpuMemory::Low,
        GpuMemory::Standard,
        GpuMemory::High,
    ];

    /// 設定のファイルの値。
    pub fn key(self) -> String {
        match self {
            GpuMemory::Auto => "auto".into(),
            GpuMemory::Low => "low".into(),
            GpuMemory::Standard => "standard".into(),
            GpuMemory::High => "high".into(),
            GpuMemory::Mib(n) => n.clamp(MIN_TOTAL_MIB, MAX_TOTAL_MIB).to_string(),
        }
    }

    /// 設定のファイルの値から。範囲の外の MiB・知らない語は None。
    pub fn parse(value: &str) -> Option<GpuMemory> {
        match value {
            "auto" => Some(GpuMemory::Auto),
            "low" => Some(GpuMemory::Low),
            "standard" => Some(GpuMemory::Standard),
            "high" => Some(GpuMemory::High),
            _ => value
                .parse::<u32>()
                .ok()
                .filter(|n| (MIN_TOTAL_MIB..=MAX_TOTAL_MIB).contains(n))
                .map(GpuMemory::Mib),
        }
    }

    /// ウィンドウに出す名前（数は出さない。指定した量は「指定」とだけ）。
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            GpuMemory::Auto => lang.pick("自動", "Automatic"),
            GpuMemory::Low => lang.pick("低", "Low"),
            GpuMemory::Standard => lang.pick("標準", "Standard"),
            GpuMemory::High => lang.pick("高", "High"),
            GpuMemory::Mib(_) => lang.pick("指定", "Custom"),
        }
    }
}

/// アダプターから分かること。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adapter {
    /// GPU が使えるメモリの量（MiB）。分からなければ None（控えめの定数で配る）。
    pub memory_mib: Option<u64>,
}

impl Adapter {
    /// wgpu のアダプターの情報から。専用のメモリを持つ GPU は専用の量、メインメモリを分け合う GPU（内蔵）は共有の量。
    /// ソフトウェアの描画・分からないものは None。
    pub fn detect(info: &eframe::egui_wgpu::wgpu::AdapterInfo) -> Adapter {
        use eframe::egui_wgpu::wgpu::DeviceType;
        let memory_mib = match info.device_type {
            DeviceType::DiscreteGpu => platform::memory_mib(info.vendor, info.device, true),
            DeviceType::IntegratedGpu => platform::memory_mib(info.vendor, info.device, false),
            _ => None,
        };
        Adapter { memory_mib }
    }
}

/// 配った結果（バイト）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budgets {
    /// 3D の絵の全体（今のセットとほかのセットの合計。`View3dRenderer::set_paint_budget`）。
    pub paint: u64,
    /// キャンバスの GPU の合成の常駐（`GpuCanvas::set_budget`）。
    pub canvas: u64,
    /// 棚のサムネイルのために展開してよい量（`Shelf::set_preview_budget`）。
    pub shelf_preview: u64,
}

impl Default for Budgets {
    /// 標準（アダプターの量が分からないときと同じ）。
    fn default() -> Self {
        distribute(STANDARD_MIB * MIB)
    }
}

/// 標準の合計（MiB）。
fn standard_mib(adapter: &Adapter) -> u64 {
    match adapter.memory_mib {
        Some(memory) => (memory / STANDARD_DIVISOR).clamp(STANDARD_MIB, STANDARD_MAX_MIB),
        None => STANDARD_MIB,
    }
}

/// 選んだ GPU のメモリの合計（バイト）。
pub fn total_bytes(choice: GpuMemory, adapter: &Adapter) -> u64 {
    let standard = standard_mib(adapter);
    let mib = match choice {
        GpuMemory::Mib(n) => n as u64,
        GpuMemory::Low => standard / 2,
        GpuMemory::Standard => standard,
        GpuMemory::High => standard * 2,
        GpuMemory::Auto => match adapter.memory_mib {
            Some(memory) if memory < AUTO_LOW_BELOW_MIB => standard / 2,
            _ => standard,
        },
    };
    mib * MIB
}

/// 合計を 3 つの予算へ配る（割合は [`SHARES`]）。
pub fn distribute(total: u64) -> Budgets {
    let sum = SHARES.sum();
    Budgets {
        paint: total / sum * SHARES.paint + total % sum * SHARES.paint / sum,
        canvas: total / sum * SHARES.canvas + total % sum * SHARES.canvas / sum,
        shelf_preview: total / sum * SHARES.shelf + total % sum * SHARES.shelf / sum,
    }
}

/// 選びとアダプターから、3 つの予算。
pub fn budgets(choice: GpuMemory, adapter: &Adapter) -> Budgets {
    distribute(total_bytes(choice, adapter))
}

/// アダプターのメモリの量を OS に聞く（wgpu は返さない）。分からなければ None。
mod platform {
    /// `vendor`・`device`（PCI の番号。wgpu のアダプターの情報と同じ）が一致する GPU の、専用のメモリ（`dedicated`）か
    /// メインメモリとの共有の量（MiB）。
    #[cfg(windows)]
    pub fn memory_mib(vendor: u32, device: u32, dedicated: bool) -> Option<u64> {
        use windows::Win32::Graphics::Dxgi::{
            CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
        };
        // SAFETY: DXGI の工場を作って、アダプターを数え上げ、記述を読むだけ（Win32 の呼び方どおり）。
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
            let mut found: Option<u64> = None;
            for index in 0.. {
                let Ok(adapter) = factory.EnumAdapters1(index) else {
                    break;
                };
                let Ok(desc) = adapter.GetDesc1() else {
                    continue;
                };
                if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0
                    || desc.VendorId != vendor
                    || desc.DeviceId != device
                {
                    continue;
                }
                let bytes = if dedicated {
                    desc.DedicatedVideoMemory
                } else {
                    desc.SharedSystemMemory
                } as u64;
                found = found.max(Some(bytes / (1 << 20)));
            }
            found.filter(|mib| *mib > 0)
        }
    }

    /// 今は Windows だけ。ほかの OS は分からない（控えめの定数で配る）。
    #[cfg(not(windows))]
    pub fn memory_mib(_vendor: u32, _device: u32, _dedicated: bool) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(memory_mib: u64) -> Adapter {
        Adapter {
            memory_mib: Some(memory_mib),
        }
    }

    #[test]
    fn the_standard_split_is_the_three_old_constants() {
        let b = Budgets::default();
        assert_eq!(b.paint, crate::view3d::paint::PAINT_BUDGET_BYTES);
        assert_eq!(b.canvas, crate::canvas::gpu::RESIDENT_BUDGET);
        assert_eq!(b.shelf_preview, crate::shelf::PREVIEW_BUDGET);
        // 量が分からない・自動・標準は、どれも同じ
        let unknown = Adapter::default();
        assert_eq!(budgets(GpuMemory::Auto, &unknown), b);
        assert_eq!(budgets(GpuMemory::Standard, &unknown), b);
    }

    #[test]
    fn the_split_follows_the_shares_and_loses_nothing() {
        for total in [
            0,
            1,
            7,
            9,
            1000,
            1152 * MIB,
            1152 * MIB + 5,
            u64::from(u32::MAX) * MIB,
        ] {
            let b = distribute(total);
            let sum = b.paint + b.canvas + b.shelf_preview;
            // 端数は切り捨てるだけ（合計を超えず、失うのは割合の数より少ない）
            assert!(sum <= total && total - sum < SHARES.sum(), "{total}: {b:?}");
            assert_eq!(b.paint, b.canvas, "{total}");
            assert!(b.shelf_preview <= b.paint, "{total}");
        }
        let b = distribute(900 * MIB);
        assert_eq!(
            (b.paint, b.canvas, b.shelf_preview),
            (400 * MIB, 400 * MIB, 100 * MIB)
        );
    }

    #[test]
    fn low_standard_and_high_double_each_time_and_auto_is_standard() {
        let a = Adapter::default();
        let (low, standard, high) = (
            total_bytes(GpuMemory::Low, &a),
            total_bytes(GpuMemory::Standard, &a),
            total_bytes(GpuMemory::High, &a),
        );
        assert_eq!((low * 2, standard * 2), (standard, high));
        assert_eq!(total_bytes(GpuMemory::Auto, &a), standard);
        for known in [gpu(4096), gpu(8192), gpu(24576)] {
            assert_eq!(
                total_bytes(GpuMemory::Auto, &known),
                total_bytes(GpuMemory::Standard, &known)
            );
        }
    }

    #[test]
    fn a_known_amount_scales_the_standard_between_its_floor_and_ceiling() {
        let standard = |m: u64| total_bytes(GpuMemory::Standard, &gpu(m)) / MIB;
        assert_eq!(standard(2048), STANDARD_MIB, "少なくても下限（以前の定数）");
        assert_eq!(standard(8192), STANDARD_MIB.max(8192 / 8));
        assert_eq!(standard(16384), 2048);
        assert_eq!(standard(24576), 3072);
        assert_eq!(standard(1 << 20), STANDARD_MAX_MIB, "多くても上限");
        // 量が多いほど減らない
        let mut last = 0;
        for m in (0..=64).map(|i| i * 1024) {
            let now = standard(m);
            assert!(now >= last, "{m}");
            last = now;
        }
    }

    #[test]
    fn auto_goes_low_only_when_the_known_amount_is_small() {
        let low_of = |a: &Adapter| total_bytes(GpuMemory::Low, a);
        assert_eq!(total_bytes(GpuMemory::Auto, &gpu(2048)), low_of(&gpu(2048)));
        assert_eq!(total_bytes(GpuMemory::Auto, &gpu(3071)), low_of(&gpu(3071)));
        assert_ne!(total_bytes(GpuMemory::Auto, &gpu(3072)), low_of(&gpu(3072)));
        // 量が分からないときは、低にしない
        assert_ne!(
            total_bytes(GpuMemory::Auto, &Adapter::default()),
            low_of(&Adapter::default())
        );
    }

    #[test]
    fn a_custom_total_is_taken_as_it_is_whatever_the_adapter_is() {
        for a in [Adapter::default(), gpu(2048), gpu(32768)] {
            assert_eq!(total_bytes(GpuMemory::Mib(700), &a), 700 * MIB);
        }
    }

    #[test]
    fn the_file_values_round_trip_and_bad_values_are_refused() {
        for choice in [
            GpuMemory::Auto,
            GpuMemory::Low,
            GpuMemory::Standard,
            GpuMemory::High,
            GpuMemory::Mib(256),
            GpuMemory::Mib(1500),
            GpuMemory::Mib(32768),
        ] {
            assert_eq!(GpuMemory::parse(&choice.key()), Some(choice), "{choice:?}");
        }
        for bad in [
            "",
            "AUTO",
            "mid",
            "-1",
            "255",
            "32769",
            "1e3",
            "12.5",
            "99999999999999999999",
            " 512",
        ] {
            assert_eq!(GpuMemory::parse(bad), None, "{bad:?}");
        }
        // 範囲の外の MiB は、書くときに範囲へ収める
        assert_eq!(GpuMemory::Mib(1).key(), MIN_TOTAL_MIB.to_string());
        assert_eq!(GpuMemory::Mib(u32::MAX).key(), MAX_TOTAL_MIB.to_string());
    }

    #[test]
    fn the_names_carry_no_numbers() {
        for lang in Lang::ALL {
            for choice in GpuMemory::LEVELS.into_iter().chain([GpuMemory::Mib(1500)]) {
                let name = choice.name(lang);
                assert!(
                    !name.is_empty() && !name.chars().any(|c| c.is_ascii_digit()),
                    "{lang:?} {choice:?}: {name}"
                );
            }
        }
    }
}
