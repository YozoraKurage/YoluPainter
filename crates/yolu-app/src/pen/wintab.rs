//! WinTab（Wacom などのタブレットのドライバーが `Wintab32.dll` で出す入力）のパケットを `PenSample` に直す（OS に依らない純粋な部分）。
//! `Wintab32.dll` の呼び出しとウィンドウのメッセージは `win_tab`（Windows だけ）。ここは Linux でも単体試験できる。
//!
//! 値の直し方（出どころは WinTab 1.1 の仕様の `wintab.h`・`pktdef.h` と、Wacom の Wintab のリファレンス・開発者サポート）:
//! - 文脈は `WTI_DEFSYSCTX`（システムのカーソルを動かす既定の文脈）から作る。パケットは状態・時刻・カーソル・ボタン・X・Y・筆圧・向きで、ボタンは押している物のビット
//!   （`lcPktMode` を 0 にした絶対の形）。`lcIn*`・`lcSys*`・`lcSysMode` は変えない（変えると、利用者がドライバーの設定で決めたタブレットの範囲と画面の割り当てを
//!   上書きする）。`lcOut*` だけを機器の広さ（`lcIn*`）にして、整数の画素に丸められる前の値を受ける。
//! - 位置: パケットの X・Y は `lcOut*` の範囲に対する割合 u・v（Y は機器の下が 0）に直し、画面の矩形へ f64 で写す（Y は反転）。画面の矩形の候補は、ドライバーが文脈に
//!   書いた `lcSys*`・各モニター・仮想デスクトップ。ドライバーが割り当てを隠す設定（モニターへの割り当て・マウスモード）があるので、1 回に読んだ点の最後の位置を OS のカーソルと
//!   比べる（`ScreenMap`）。今の候補はカーソルと 12 画素以内のあいだだけ使い、新しく掴む・替えるのは、離れた 2 か所で同じ候補が最も合ったときだけ（押しの途中では替えない）。
//!   合わない回の点はカーソルの位置に置く。相対モード（`lcSysMode`）は候補を試さず、いつもカーソルの位置。
//! - 押しの持ち主: ペンの 1 本の押しは、始まりの点が先に来た側（Windows Ink か WinTab）の物になり、離すまで替えない（`PressOwner`）。もう一方の側の点はその間入れない。
//! - 筆圧: `DVC_NPRESSURE` の最小・最大で 0〜1。筆圧を持たない機器・カーソルは、触れている間 1。
//! - 傾き: `PK_ORIENTATION` の方位（時計回り、機器の上が 0）と高さ（紙との角、垂直が最大。裏返しは負）を、Windows Ink と同じ意味の傾き（x は右へ倒すと正、y は利用者の側へ
//!   倒すと正、度）に直す。角度の数は `DVC_ORIENTATION` の `axResolution`（円周あたりの数、16.16 の固定小数）から求める。
//! - 回転: 向きのねじれ。ねじれの軸を持たない機器は None。
//! - 消しゴムの端: パケットの状態の `TPS_INVERT`、またはカーソルの種類の `CRC_INVERT`（裏返した向きを表す）。
//! - 触れている: 先のボタン（ビット 0）か筆圧 > 0。サイドボタン: 先以外のボタンのビット。
//! - 時刻: パケットの時刻（0 のときは受け取った時刻）。

use super::PenSample;
use crate::engine::Tilt;
use crate::lang::Lang;

// ───────── 定数（wintab.h） ─────────

pub const PK_CONTEXT: u32 = 0x0001;
pub const PK_STATUS: u32 = 0x0002;
pub const PK_TIME: u32 = 0x0004;
pub const PK_CURSOR: u32 = 0x0020;
pub const PK_BUTTONS: u32 = 0x0040;
pub const PK_X: u32 = 0x0080;
pub const PK_Y: u32 = 0x0100;
pub const PK_NORMAL_PRESSURE: u32 = 0x0400;
pub const PK_ORIENTATION: u32 = 0x1000;
pub const PK_ROTATION: u32 = 0x2000;

/// 文脈が送るパケットの内容（`RawPacket` の並びと同じ順）。
pub const PACKET_DATA: u32 = PK_STATUS
    | PK_TIME
    | PK_CURSOR
    | PK_BUTTONS
    | PK_X
    | PK_Y
    | PK_NORMAL_PRESSURE
    | PK_ORIENTATION;
/// パケットを作る変化（動き・筆圧・向き・ボタン）。
pub const MOVE_MASK: u32 = PK_BUTTONS | PK_X | PK_Y | PK_NORMAL_PRESSURE | PK_ORIENTATION;

pub const CXO_SYSTEM: u32 = 0x0001;
pub const CXO_MESSAGES: u32 = 0x0004;

/// パケットの状態（`pkStatus`）: カーソルが裏返っている。
pub const TPS_INVERT: u32 = 0x0010;
/// カーソルの種類の機能（`CSR_CAPABILITIES`）: 裏返した向きを表す。
pub const CRC_INVERT: u32 = 0x0004;

/// 単位: 円周。
pub const TU_CIRCLE: u32 = 3;

/// 文脈の重なりの状態（`WT_CTXOVERLAP` の `lParam`）。
pub const CXS_DISABLED: u32 = 0x0001;
pub const CXS_OBSCURED: u32 = 0x0002;
pub const CXS_ONTOP: u32 = 0x0004;

pub const WT_DEFBASE: u32 = 0x7FF0;
pub const WT_MAXOFFSET: u32 = 0xF;
/// メッセージの番号（`lcMsgBase` からの差）。
pub const WT_PACKET: u32 = 0;
pub const WT_CTXOPEN: u32 = 1;
pub const WT_CTXCLOSE: u32 = 2;
pub const WT_CTXUPDATE: u32 = 3;
pub const WT_CTXOVERLAP: u32 = 4;
pub const WT_PROXIMITY: u32 = 5;
pub const WT_INFOCHANGE: u32 = 6;
pub const WT_CSRCHANGE: u32 = 7;

/// `WTInfo` の分類と項目。
pub const WTI_INTERFACE: u32 = 1;
pub const IFC_WINTABID: u32 = 1;
pub const WTI_DEFSYSCTX: u32 = 4;
pub const WTI_DEVICES: u32 = 100;
pub const WTI_CURSORS: u32 = 200;
pub const IFC_NDEVICES: u32 = 4;
pub const DVC_NPRESSURE: u32 = 15;
pub const DVC_ORIENTATION: u32 = 17;
pub const CSR_NAME: u32 = 1;
pub const CSR_PKTDATA: u32 = 3;
pub const CSR_TYPE: u32 = 20;
pub const CSR_CAPABILITIES: u32 = 19;

/// ボタンのビット: 先（ビット 0）。それ以外はサイドボタンとして扱う。
pub const BUTTON_TIP: u32 = 1;

/// 点の `pointer_id`（WinTab の点はどれもこの番号。Windows Ink の点と同時には来ない）。
pub const POINTER_ID: u32 = 0x5754;

/// 今の写し先を使い続けてよい差（物理の画素、縦横の差の和）。パケットとカーソルの読みの時刻の違いによる動きの分だけ許す（これを越える差のまま描くと、ずれて描いて、替えたときに跳ぶ）。
pub const HOLD_TOLERANCE: f64 = 12.0;
/// 新しく写し先を掴むとき、同じ候補が最も合った 2 回のカーソルの位置が離れているべき差（縦横の差の和）。端や隅で複数の候補が同じ位置に写るのを、これで見分ける。
pub const GRAB_APART: f64 = 100.0;

// ───────── ドライバーとやり取りする形（`#[repr(C)]`） ─────────

/// `AXIS`。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Axis {
    pub min: i32,
    pub max: i32,
    pub units: u32,
    /// 1 単位あたりの数（16.16 の固定小数。円周なら円周あたりの数）。
    pub resolution: u32,
}

/// `ORIENTATION`（方位・高さ・ねじれ）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Orientation {
    pub azimuth: i32,
    pub altitude: i32,
    pub twist: i32,
}

/// パケット 1 つ（`PACKET_DATA` の項目。ドライバーが出すバイト列は `PacketLayout` で読む）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RawPacket {
    pub status: u32,
    pub time: u32,
    pub cursor: u32,
    pub buttons: u32,
    pub x: i32,
    pub y: i32,
    pub pressure: u32,
    pub orientation: Orientation,
}

/// ドライバーが出すパケットのバイト列の並び。並びは文脈の `lcPktData` の印の順（`pktdef.h` の `PACKET`）で、項目は全部 4 バイトの整数
/// （向きは 3 つ並び）。文脈を開いたあとにドライバーが返した `lcPktData` で作り、要求した並びと違っても（印を外された）読めるようにする。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketLayout {
    data: u32,
    size: usize,
}

impl PacketLayout {
    /// 取りうる並びのうち最大のパケットのバイト数（`PACKET_DATA` の全部の項目）。受け皿の大きさと、一度に頼む数の元（ドライバーが並びを変えても溢れない）。
    pub const MAX_SIZE: usize = 40;

    /// 並び（印）から。`PACKET_DATA` の外の印（文脈のハンドルは 8 バイトで詰め方に依る、ほかは使わない）を含むなら None。
    pub fn new(data: u32) -> Option<PacketLayout> {
        if data & !PACKET_DATA != 0 {
            return None;
        }
        let words = [
            PK_STATUS,
            PK_TIME,
            PK_CURSOR,
            PK_BUTTONS,
            PK_X,
            PK_Y,
            PK_NORMAL_PRESSURE,
        ]
        .iter()
        .filter(|&&bit| data & bit != 0)
        .count()
            + if data & PK_ORIENTATION != 0 { 3 } else { 0 };
        Some(PacketLayout {
            data,
            size: words * 4,
        })
    }

    /// パケット 1 つのバイト数。
    pub fn size(&self) -> usize {
        self.size
    }

    /// 位置が入っているか（入っていなければ使えない並び）。
    pub fn has_position(&self) -> bool {
        self.data & PK_X != 0 && self.data & PK_Y != 0
    }

    /// バイト列の先頭から 1 つ読む（無い項目は 0）。足りなければ None。
    pub fn read(&self, bytes: &[u8]) -> Option<RawPacket> {
        if bytes.len() < self.size {
            return None;
        }
        let mut at = 0;
        let mut word = |present: bool| -> u32 {
            if !present {
                return 0;
            }
            let value =
                u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
            at += 4;
            value
        };
        let status = word(self.data & PK_STATUS != 0);
        let time = word(self.data & PK_TIME != 0);
        let cursor = word(self.data & PK_CURSOR != 0);
        let buttons = word(self.data & PK_BUTTONS != 0);
        let x = word(self.data & PK_X != 0) as i32;
        let y = word(self.data & PK_Y != 0) as i32;
        let pressure = word(self.data & PK_NORMAL_PRESSURE != 0);
        let has_orientation = self.data & PK_ORIENTATION != 0;
        let orientation = Orientation {
            azimuth: word(has_orientation) as i32,
            altitude: word(has_orientation) as i32,
            twist: word(has_orientation) as i32,
        };
        Some(RawPacket {
            status,
            time,
            cursor,
            buttons,
            x,
            y,
            pressure,
            orientation,
        })
    }
}

/// `LOGCONTEXTA`（名前が 1 バイト文字の版。全部 4 バイトの項目なので、詰め方によらない並び）。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LogContext {
    pub name: [u8; 40],
    pub options: u32,
    pub status: u32,
    pub locks: u32,
    pub msg_base: u32,
    pub device: u32,
    pub pkt_rate: u32,
    pub pkt_data: u32,
    pub pkt_mode: u32,
    pub move_mask: u32,
    pub btn_dn_mask: u32,
    pub btn_up_mask: u32,
    pub in_org: [i32; 3],
    pub in_ext: [i32; 3],
    pub out_org: [i32; 3],
    pub out_ext: [i32; 3],
    pub sens: [u32; 3],
    pub sys_mode: i32,
    pub sys_org: [i32; 2],
    pub sys_ext: [i32; 2],
    pub sys_sens: [u32; 2],
}

impl LogContext {
    pub fn zeroed() -> LogContext {
        LogContext {
            name: [0; 40],
            options: 0,
            status: 0,
            locks: 0,
            msg_base: 0,
            device: 0,
            pkt_rate: 0,
            pkt_data: 0,
            pkt_mode: 0,
            move_mask: 0,
            btn_dn_mask: 0,
            btn_up_mask: 0,
            in_org: [0; 3],
            in_ext: [0; 3],
            out_org: [0; 3],
            out_ext: [0; 3],
            sens: [0; 3],
            sys_mode: 0,
            sys_org: [0; 2],
            sys_ext: [0; 2],
            sys_sens: [0; 2],
        }
    }

    /// 既定のシステムの文脈（`WTI_DEFSYSCTX`）から、こちらの文脈を作る。パケットの内容と、動きの印・ボタンの印を決め、出力の範囲を機器の広さにする。
    /// `lcIn*`・`lcSys*`・`lcSysMode` は変えない（利用者がドライバーで決めた範囲と画面の割り当てを上書きしないため）。
    pub fn ours(default: &LogContext) -> LogContext {
        let mut lc = *default;
        lc.name = [0; 40];
        let name = b"YoluPainter";
        lc.name[..name.len()].copy_from_slice(name);
        lc.options = default.options | CXO_SYSTEM | CXO_MESSAGES;
        lc.pkt_data = PACKET_DATA;
        lc.pkt_mode = 0;
        lc.move_mask = MOVE_MASK;
        lc.btn_dn_mask = u32::MAX;
        lc.btn_up_mask = u32::MAX;
        lc.out_org = default.in_org;
        lc.out_ext = default.in_ext;
        lc
    }
}

// ───────── メッセージ ─────────

/// ウィンドウのメッセージが WinTab のメッセージなら、`lcMsgBase` からの差（`WT_PACKET` など）。
pub fn message_offset(msg: u32, base: u32) -> Option<u32> {
    (msg >= base && msg - base <= WT_MAXOFFSET).then(|| msg - base)
}

// ───────── 位置 ─────────

/// 画面の矩形（物理の画素。左上が原点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(left: i32, top: i32, width: i32, height: i32) -> Rect {
        Rect {
            left: left as f64,
            top: top as f64,
            width: width as f64,
            height: height as f64,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}

/// パケットの X・Y を、文脈の出力の範囲（`lcOut*`）に対する割合にする。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputRange {
    pub org: [i32; 2],
    pub ext: [i32; 2],
}

impl OutputRange {
    pub fn of(lc: &LogContext) -> OutputRange {
        OutputRange {
            org: [lc.out_org[0], lc.out_org[1]],
            ext: [lc.out_ext[0], lc.out_ext[1]],
        }
    }

    /// 機器の左が u=0・右が 1、下が v=0・上が 1（範囲の外は端に寄せる）。広さが 0 なら None。
    /// 出力の範囲は、広さの符号によらず `org` から `org + |ext|`。広さが負なら、その軸は反転している（機器の下・左が範囲の終わり側）ので、割合を `1 - 割合` にする。
    pub fn fraction(&self, x: i32, y: i32) -> Option<[f64; 2]> {
        if self.ext[0] == 0 || self.ext[1] == 0 {
            return None;
        }
        let axis = |value: i32, org: i32, ext: i32| {
            let raw = (value as f64 - org as f64) / (ext as f64).abs();
            if ext < 0 {
                1.0 - raw
            } else {
                raw
            }
        };
        let u = axis(x, self.org[0], self.ext[0]);
        let v = axis(y, self.org[1], self.ext[1]);
        (u.is_finite() && v.is_finite()).then(|| [u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)])
    }
}

/// 割合を画面の矩形へ写す（Y は機器の下が 0 なので反転）。整数に丸めない。
pub fn to_screen(fraction: [f64; 2], rect: Rect) -> [f64; 2] {
    [
        rect.left + fraction[0] * rect.width,
        rect.top + (1.0 - fraction[1]) * rect.height,
    ]
}

/// 画面の位置の 1 成分を、整数の画素（切り下げ）と 0 以上 1 未満の端数に分ける（`ScreenToClient` が整数しか受けないので、端数は後で加える）。
/// 負の位置（主のモニターより左・上）でも端数は 0 以上で、`整数 + 端数` が元の値になる。数でない・整数に入らない値は None。
pub fn split_position(v: f64) -> Option<(i32, f32)> {
    let floor = v.floor();
    (v.is_finite() && floor >= i32::MIN as f64 && floor <= i32::MAX as f64)
        .then_some((floor as i32, (v - floor) as f32))
}

/// 写し先の矩形の出どころ（ログに出す）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapKind {
    /// ドライバーが文脈に書いた `lcSys*`。
    Driver,
    /// i 番目のモニター。
    Monitor(usize),
    /// 仮想デスクトップ全体。
    Desktop,
}

/// 画面への写し先を選ぶ。候補（ドライバーの `lcSys*`・各モニター・仮想デスクトップ）のうち、1 回に読んだ最後の点の位置が OS のカーソルと重なる物を使う。
///
/// - 今の候補は、カーソルとの差が `HOLD_TOLERANCE` 以内のあいだだけ使う。越えた回は、候補を残したまま、その回の点をカーソルの位置に置く
///   （一時の遅れで写しを失わない。マウスモードのように合わないままなら、毎回カーソルの位置になる）。
/// - 新しく掴む・替えるのは、候補のうち最も差が小さい物が `HOLD_TOLERANCE` 以内で、同じ候補が、`GRAB_APART` 以上離れたカーソルの位置で、間に合わない回を挟まずに
///   もう 1 度そうなったときだけ。1 か所の偶然（端・隅・マウスモードの重なり）では掴まない。押しの途中（`pressing`）は掴まず、替えない。
/// - 候補が無い（相対モード）ときは、いつも None。
#[derive(Clone, Debug)]
pub struct ScreenMap {
    candidates: Vec<(MapKind, Rect)>,
    current: Option<usize>,
    /// 1 回最も合った候補と、そのときのカーソルの位置（離れたもう 1 回で掴む）。
    pending: Option<(usize, [f64; 2])>,
}

impl ScreenMap {
    /// 候補は、ドライバー・モニター・仮想デスクトップの順（同じ程度に合うときは前を取る）。矩形になっていない候補は入れない。
    pub fn new(driver: Option<Rect>, monitors: &[Rect], desktop: Option<Rect>) -> ScreenMap {
        let mut candidates = Vec::new();
        if let Some(r) = driver.filter(Rect::is_valid) {
            candidates.push((MapKind::Driver, r));
        }
        for (i, r) in monitors.iter().enumerate() {
            if r.is_valid() {
                candidates.push((MapKind::Monitor(i), *r));
            }
        }
        if let Some(r) = desktop.filter(Rect::is_valid) {
            candidates.push((MapKind::Desktop, r));
        }
        ScreenMap {
            candidates,
            current: None,
            pending: None,
        }
    }

    /// 候補を持たない写し先（相対モード。点の位置はいつもカーソル）。
    pub fn none() -> ScreenMap {
        ScreenMap::new(None, &[], None)
    }

    /// 今の写し先。
    pub fn current(&self) -> Option<(MapKind, Rect)> {
        self.current.map(|i| self.candidates[i])
    }

    /// 候補は変えずに、選びと待っていた物を忘れる（画面の構成が読み直せなかったとき）。
    pub fn forget(&mut self) {
        self.current = None;
        self.pending = None;
    }

    /// 候補の先頭（カーソルが読めないときの、仮の写し先）。
    fn first(&self) -> Option<(MapKind, Rect)> {
        self.candidates.first().copied()
    }

    /// 割合 `fraction` の点が、OS のカーソル `cursor` と重なる写し先を決めて、この回に使う写し先を返す（合わなければ None）。
    pub fn settle(
        &mut self,
        fraction: [f64; 2],
        cursor: [f64; 2],
        pressing: bool,
    ) -> Option<(MapKind, Rect)> {
        let error = |rect: Rect| {
            let p = to_screen(fraction, rect);
            (p[0] - cursor[0]).abs() + (p[1] - cursor[1]).abs()
        };
        if let Some(i) = self.current {
            if error(self.candidates[i].1) <= HOLD_TOLERANCE {
                self.pending = None;
                return self.current();
            }
        }
        // 今の候補が合わない回。押しの途中は掴まない・替えない
        if pressing {
            self.pending = None;
            return None;
        }
        let mut best: Option<(usize, f64)> = None;
        for (i, (_, rect)) in self.candidates.iter().enumerate() {
            let e = error(*rect);
            if e <= HOLD_TOLERANCE && best.is_none_or(|(_, b)| e < b) {
                best = Some((i, e));
            }
        }
        let Some((best, _)) = best else {
            self.pending = None;
            return None;
        };
        match self.pending {
            Some((candidate, at))
                if candidate == best
                    && (at[0] - cursor[0]).abs() + (at[1] - cursor[1]).abs() >= GRAB_APART =>
            {
                self.current = Some(best);
                self.pending = None;
                self.current()
            }
            // 同じ候補が近くでもう 1 度合っただけなら、最初の位置を覚えたまま待つ
            Some((candidate, _)) if candidate == best => None,
            _ => {
                self.pending = Some((best, cursor));
                None
            }
        }
    }
}

/// 1 回に読んだ点の画面の位置。`cursor` は読んだ時点の OS のカーソル（読めなければ None）。`touching` は前の回から触れたままか。
/// 最後の点とカーソルで写し先を選び（`ScreenMap::settle`）、使える写し先があれば全部の点をそこへ写す（f64）。なければ、全部の点をカーソルの位置に置く。
/// カーソルが読めないときは、今の写し先（無ければ候補の先頭）で写す。
pub fn batch_positions(
    packets: &[RawPacket],
    range: &OutputRange,
    map: &mut ScreenMap,
    cursor: Option<[f64; 2]>,
    touching: bool,
) -> Vec<Option<[f64; 2]>> {
    let fractions: Vec<Option<[f64; 2]>> =
        packets.iter().map(|p| range.fraction(p.x, p.y)).collect();
    let last = fractions.last().copied().flatten();
    let pressing = touching
        || packets
            .iter()
            .any(|p| tip_down(p.buttons) || p.pressure > 0);
    let rect = match (cursor, last) {
        (Some(c), Some(f)) => map.settle(f, c, pressing).map(|(_, r)| r),
        (Some(_), None) => None,
        (None, _) => map.current().or_else(|| map.first()).map(|(_, r)| r),
    };
    fractions
        .iter()
        .map(|f| match (rect, f) {
            (Some(r), Some(f)) => Some(to_screen(*f, r)),
            _ => cursor,
        })
        .collect()
}

// ───────── 筆圧・向き・ボタン・時刻 ─────────

/// 筆圧を 0〜1 にする。範囲が使えない（最大が最小以下）なら None。
pub fn normalize_pressure(raw: u32, axis: &Axis) -> Option<f32> {
    let span = axis.max as f64 - axis.min as f64;
    if span <= 0.0 {
        return None;
    }
    Some(((raw as f64 - axis.min as f64) / span).clamp(0.0, 1.0) as f32)
}

/// 機器の向きの軸（円周あたりの数）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrientationAxes {
    /// 方位・高さの円周あたりの数。どちらかが 0 なら傾きを送れない機器。
    pub tilt_circle: Option<[f64; 2]>,
    /// ねじれの円周あたりの数。0 なら回転を送れない機器。
    pub twist_circle: Option<f64>,
}

impl OrientationAxes {
    /// `DVC_ORIENTATION`（方位・高さ・ねじれの 3 つ）から。方位・高さは、単位が円周（`TU_CIRCLE`）で解像度が正の軸だけ使う（違えば傾きを送れない機器）。
    pub fn from_axes(axes: &[Axis; 3]) -> OrientationAxes {
        let circle = |a: &Axis| {
            let c = a.resolution as f64 / 65536.0;
            (c.is_finite() && c > 0.0).then_some(c)
        };
        let angle = |a: &Axis| circle(a).filter(|_| a.units == TU_CIRCLE);
        OrientationAxes {
            tilt_circle: angle(&axes[0]).zip(angle(&axes[1])).map(|(a, b)| [a, b]),
            twist_circle: circle(&axes[2]),
        }
    }
}

/// 方位（時計回り、機器の上が 0）と高さ（紙との角。垂直が最大。裏返しは負）から、Windows Ink と同じ意味の傾き（度）にする。
/// 画面の右を x、利用者の側を y とすると、ペンの先から後ろの端へ向かう向きは (cos 高さ × sin 方位, -cos 高さ × cos 方位, sin 高さ) で、
/// 傾き x・y はそれぞれ x・y の成分と垂直の成分の角（x は右へ倒すと正、y は利用者の側へ倒すと正）。
/// 全部 0 の向きは、向きを送っていない（平らに寝たペンと区別できない）とみて、垂直。
pub fn tilt_degrees(o: &Orientation, circles: [f64; 2]) -> Tilt {
    if o.azimuth == 0 && o.altitude == 0 && o.twist == 0 {
        return Tilt::default();
    }
    let turn = std::f64::consts::TAU;
    let azimuth = o.azimuth as f64 / circles[0] * turn;
    let altitude = (o.altitude as f64).abs() / circles[1] * turn;
    let altitude = altitude.clamp(0.0, std::f64::consts::FRAC_PI_2);
    let along = altitude.sin();
    let across = altitude.cos();
    // 数値の誤差で 0 にならなかった成分は 0 にする（真横に寝かせたとき、もう一方の軸が ±90° に振れないように）
    let angle = |across_part: f64| {
        let across_part = if across_part.abs() < 1e-12 {
            0.0
        } else {
            across_part
        };
        let v = across_part.atan2(along);
        if v.is_finite() {
            v.to_degrees().clamp(-90.0, 90.0) as f32
        } else {
            0.0
        }
    };
    Tilt {
        x: angle(across * azimuth.sin()),
        y: angle(-across * azimuth.cos()),
    }
}

/// ねじれ（時計回り）を 0〜360 の度にする。
pub fn rotation_degrees(twist: i32, circle: f64) -> Option<f32> {
    let degrees = (twist as f64 / circle * 360.0).rem_euclid(360.0);
    degrees.is_finite().then_some(degrees as f32)
}

/// ボタンのビットから、先が押されているか。
pub fn tip_down(buttons: u32) -> bool {
    buttons & BUTTON_TIP != 0
}

/// ボタンのビットから、サイドボタン（先以外のボタン）が押されているか。
pub fn side_down(buttons: u32) -> bool {
    buttons & !BUTTON_TIP != 0
}

/// パケットの時刻（ミリ秒）。ドライバーが時刻を入れない（0）ときは、受け取った時刻。
pub fn packet_time(raw: u32, received: u32) -> u32 {
    if raw != 0 {
        raw
    } else {
        received
    }
}

/// カーソルの種類（ペン先・消しゴムの端など）ごとの情報。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorInfo {
    /// 裏返した向き（消しゴムの端）を表す種類。
    pub invert: bool,
    /// 筆圧を送る種類。
    pub pressure: bool,
    /// 向きを送る種類。
    pub orientation: bool,
}

impl Default for CursorInfo {
    /// 問い合わせが答えない種類は、機器の軸に任せる（筆圧・向きは送る物として扱う）。
    fn default() -> Self {
        CursorInfo {
            invert: false,
            pressure: true,
            orientation: true,
        }
    }
}

impl CursorInfo {
    /// `CSR_CAPABILITIES` と `CSR_PKTDATA` の答えから（答えが無ければ None）。
    pub fn from_info(capabilities: Option<u32>, packet_data: Option<u32>) -> CursorInfo {
        CursorInfo {
            invert: capabilities.is_some_and(|c| c & CRC_INVERT != 0),
            pressure: packet_data.is_none_or(|d| d & PK_NORMAL_PRESSURE != 0),
            orientation: packet_data.is_none_or(|d| d & PK_ORIENTATION != 0),
        }
    }
}

/// 消しゴムの端か（パケットの状態が裏返し、または裏返しを表すカーソルの種類）。
pub fn is_eraser(status: u32, cursor: &CursorInfo) -> bool {
    status & TPS_INVERT != 0 || cursor.invert
}

/// 文脈を開いたときに分かった、機器の筆圧・向きの軸。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Device {
    /// `DVC_NPRESSURE`。取れなかった・範囲が使えないなら None（筆圧を持たない機器）。
    pub pressure: Option<Axis>,
    /// `DVC_ORIENTATION`。取れなかったなら向きを送れない機器。
    pub orientation: OrientationAxes,
}

impl Device {
    pub const NONE: Device = Device {
        pressure: None,
        orientation: OrientationAxes {
            tilt_circle: None,
            twist_circle: None,
        },
    };
}

/// パケット 1 つを `PenSample` にする。`pos` はクライアント領域の位置（呼ぶ側が画面の位置から求める）。`received` はパケットを受け取った時刻（ミリ秒）。
pub fn sample_from_packet(
    packet: &RawPacket,
    pos: [f32; 2],
    device: &Device,
    cursor: &CursorInfo,
    received: u32,
) -> PenSample {
    let axis = device.pressure.filter(|_| cursor.pressure);
    let normalized = axis.and_then(|a| normalize_pressure(packet.pressure, &a));
    let contact = tip_down(packet.buttons) || normalized.is_some_and(|p| p > 0.0);
    let pressure = match normalized {
        Some(p) => p,
        None if contact => 1.0,
        None => 0.0,
    };
    let tilt = match device.orientation.tilt_circle {
        Some(circles) if cursor.orientation => tilt_degrees(&packet.orientation, circles),
        _ => Tilt::default(),
    };
    let rotation = device
        .orientation
        .twist_circle
        .filter(|_| cursor.orientation)
        .and_then(|c| rotation_degrees(packet.orientation.twist, c));
    PenSample {
        pos,
        pressure,
        tilt,
        rotation,
        contact,
        eraser: is_eraser(packet.status, cursor),
        barrel: side_down(packet.buttons),
        pointer_id: POINTER_ID,
        time_ms: packet_time(packet.time, received),
    }
}

// ───────── 触れている間の記録 ─────────

/// 触れている点の離し（同じ位置で触れていない・筆圧 0・サイドボタン無し）。
pub fn lift(sample: PenSample) -> PenSample {
    PenSample {
        contact: false,
        pressure: 0.0,
        barrel: false,
        ..sample
    }
}

/// 触れている最後の点の記録（ペンが離れた・ウィンドウを離れた・選びを替えたとき、ペンの押しが残らないように、離しの点を作る）。
#[derive(Clone, Copy, Debug, Default)]
pub struct Touch {
    last: Option<PenSample>,
}

impl Touch {
    /// 送った点を覚える（触れていれば最後の接触の点、触れていなければ忘れる）。
    pub fn note(&mut self, sample: &PenSample) {
        self.last = sample.contact.then_some(*sample);
    }

    /// 触れていたなら、離しの点（記録は忘れる）。
    pub fn release(&mut self) -> Option<PenSample> {
        self.last.take().map(lift)
    }

    pub fn is_touching(&self) -> bool {
        self.last.is_some()
    }

    /// 触れている最後の点（触れていなければ None）。
    pub fn touching(&self) -> Option<PenSample> {
        self.last
    }
}

// ───────── 押しの持ち主 ─────────

/// 点を出す側（ペンの押しの持ち主）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Windows Ink（`WM_POINTER`）。
    Ink,
    WinTab,
}

impl Owner {
    pub fn name(self) -> &'static str {
        match self {
            Owner::Ink => "ink",
            Owner::WinTab => "wintab",
        }
    }
}

/// ペンの 1 本の押し（触れてから離すまで）の持ち主。WinTab が開いている間、同じペンの点が Windows Ink からも WinTab からも来るので、
/// 押しごとに持ち主を決め、離すまで替えない。もう一方の側の点はその間入れない（入れると、押しが 2 本になる・始まりが欠ける）。
///
/// - 押しの始まりの点（触れた点）が先に来た側が持ち主。Windows Ink の押し（`WM_POINTERDOWN`）が来たときは、先に WinTab のパケットを取ってから決める
///   （WinTab の触れた点が既にあれば WinTab の物。無ければ Windows Ink の物。文脈が無効のあいだ・後ろにあるウィンドウの最初の押しも、パケットが無いので Windows Ink の物）。
/// - 持ち主のいない（浮いている）間は、WinTab の点だけを入れる（Windows Ink の浮いた点は入れない）。
/// - 離し（触れていない点）で持ち主を手放す。離しが来ないまま止まる場合（文脈が閉じた・覆われた・ウィンドウが後ろへ行った・ペンが文脈から出た）は、呼ぶ側が `reset` する。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PressOwner {
    owner: Option<Owner>,
}

impl PressOwner {
    pub fn current(&self) -> Option<Owner> {
        self.owner
    }

    /// 押しが無いことにする。
    pub fn reset(&mut self) {
        self.owner = None;
    }

    /// WinTab の点 1 つを入れてよいか（触れた点で、持ち主がいなければ WinTab が持ち主になる。離しで手放す）。
    pub fn admit_tab(&mut self, contact: bool) -> bool {
        match self.owner {
            Some(Owner::Ink) => false,
            Some(Owner::WinTab) => {
                if !contact {
                    self.owner = None;
                }
                true
            }
            None => {
                if contact {
                    self.owner = Some(Owner::WinTab);
                }
                true
            }
        }
    }

    /// Windows Ink のペンの押し（`WM_POINTERDOWN`）。WinTab のパケットを先に取ってから呼ぶ。持ち主を返す。
    pub fn ink_down(&mut self) -> Owner {
        match self.owner {
            Some(Owner::WinTab) => Owner::WinTab,
            _ => {
                self.owner = Some(Owner::Ink);
                Owner::Ink
            }
        }
    }

    /// Windows Ink の点 1 つを入れてよいか（WinTab が開いている間。持ち主が Windows Ink の押しの点だけ入れる。持ち主がいないとき、触れた点は押しの始まりとして入れる）。
    pub fn admit_ink(&mut self, contact: bool) -> bool {
        match self.owner {
            Some(Owner::WinTab) => false,
            Some(Owner::Ink) => {
                if !contact {
                    self.owner = None;
                }
                true
            }
            None => {
                if contact {
                    self.owner = Some(Owner::Ink);
                }
                contact
            }
        }
    }

    /// Windows Ink の押しが終わった（`WM_POINTERUP`）。
    pub fn ink_up(&mut self) {
        if self.owner == Some(Owner::Ink) {
            self.owner = None;
        }
    }
}

/// Windows Ink のペンのメッセージの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InkMessage {
    Down,
    Update,
    Up,
}

/// WinTab の点（古い順）のうち、持ち主が許す物だけを返す。
pub fn route_tab(owner: &mut PressOwner, samples: Vec<PenSample>) -> Vec<PenSample> {
    samples
        .into_iter()
        .filter(|s| owner.admit_tab(s.contact))
        .collect()
}

/// Windows Ink のペンの点（古い順）のうち、持ち主が許す物だけを返す。押しの始まり（`Down`）では先に持ち主を決める。
pub fn route_ink(
    owner: &mut PressOwner,
    message: InkMessage,
    samples: Vec<PenSample>,
) -> Vec<PenSample> {
    if message == InkMessage::Down {
        owner.ink_down();
    }
    let kept = samples
        .into_iter()
        .filter(|s| owner.admit_ink(s.contact))
        .collect();
    if message == InkMessage::Up {
        owner.ink_up();
    }
    kept
}

/// `WT_CTXOVERLAP` の状態が、文脈が最前面（パケットが来る）か。
pub fn overlap_on_top(status: isize) -> bool {
    (status as usize as u32) & CXS_ONTOP != 0
}

// ───────── 使えない理由 ─────────

/// WinTab が使えなかった理由（設定で WinTab を選んだのに Windows Ink へ戻した理由）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// `Wintab32.dll` が無い（ドライバーが入っていない）。
    NoLibrary,
    /// ドライバーがあるが、WinTab の問い合わせに応えない。
    NoDriver,
    /// タブレットが繋がっていない。
    NoTablet,
    /// 文脈を開けなかった。
    NoContext,
}

impl Unavailable {
    /// 知らせの文（理由と、Windows Ink を使うこと。1 文）。
    pub fn text(self, lang: Lang) -> &'static str {
        match self {
            Unavailable::NoLibrary => lang.pick(
                "WinTab のドライバーが見つからないため、Windows Ink を使います。",
                "Using Windows Ink because no WinTab driver was found.",
            ),
            Unavailable::NoDriver => lang.pick(
                "WinTab のドライバーが応えないため、Windows Ink を使います。",
                "Using Windows Ink because the WinTab driver did not respond.",
            ),
            Unavailable::NoTablet => lang.pick(
                "WinTab のタブレットが見つからないため、Windows Ink を使います。",
                "Using Windows Ink because no WinTab tablet was found.",
            ),
            Unavailable::NoContext => lang.pick(
                "WinTab を開けなかったため、Windows Ink を使います。",
                "Using Windows Ink because WinTab could not be opened.",
            ),
        }
    }
}

// ───────── 調べるための記録（`YOLU_PEN_LOG`） ─────────

/// 受けたパケットの生の値の 1 行。
pub fn log_packet(p: &RawPacket) -> String {
    format!(
        "pkt time={} cursor={} status=0x{:x} buttons=0x{:x} x={} y={} pressure={} azimuth={} altitude={} twist={}",
        p.time,
        p.cursor,
        p.status,
        p.buttons,
        p.x,
        p.y,
        p.pressure,
        p.orientation.azimuth,
        p.orientation.altitude,
        p.orientation.twist,
    )
}

/// 直した `PenSample` の 1 行。`screen` は写した画面の位置、`source` は写し先の出どころ（カーソルのとき "cursor"）。
pub fn log_sample(s: &PenSample, screen: Option<[f64; 2]>, source: &str) -> String {
    let screen = screen.map_or_else(
        || "none".to_string(),
        |p| format!("({:.3},{:.3})", p[0], p[1]),
    );
    let rotation = s
        .rotation
        .map_or_else(|| "none".to_string(), |r| format!("{r:.1}"));
    format!(
        "smp screen={screen} map={source} pos=({:.3},{:.3}) pressure={:.4} tilt=({:.1},{:.1}) rotation={rotation} contact={} eraser={} barrel={} time={}",
        s.pos[0],
        s.pos[1],
        s.pressure,
        s.tilt.x,
        s.tilt.y,
        s.contact,
        s.eraser,
        s.barrel,
        s.time_ms,
    )
}

/// 押しの始まりのメッセージを受けたときの 1 行（行の頭は `down`。`press` と書くと、画面の文字の検査が操作の指示と読む。持ち主と、その前に取った WinTab の点の数・触れた点があったか）。
pub fn log_press(owner: Option<Owner>, message: &str, pumped: usize, contact: bool) -> String {
    format!(
        "down owner={} msg={message} pumped={pumped} contact={contact}",
        owner.map_or("none", Owner::name)
    )
}

/// 出どころの名前。
pub fn map_name(kind: Option<MapKind>) -> String {
    match kind {
        Some(MapKind::Driver) => "driver".to_string(),
        Some(MapKind::Monitor(i)) => format!("monitor{i}"),
        Some(MapKind::Desktop) => "desktop".to_string(),
        None => "cursor".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet() -> RawPacket {
        RawPacket {
            status: 0,
            time: 1000,
            cursor: 1,
            buttons: 0,
            x: 0,
            y: 0,
            pressure: 0,
            orientation: Orientation::default(),
        }
    }

    fn axis(min: i32, max: i32) -> Axis {
        Axis {
            min,
            max,
            ..Axis::default()
        }
    }

    /// 円周あたりの数 3600（0.1 度刻み）の軸。
    fn tenths() -> Axis {
        Axis {
            min: 0,
            max: 3600,
            units: TU_CIRCLE,
            resolution: 3600 << 16,
        }
    }

    #[test]
    fn the_shapes_shared_with_the_driver_have_the_sizes_of_the_c_structs() {
        assert_eq!(std::mem::size_of::<Axis>(), 16);
        assert_eq!(std::mem::size_of::<[Axis; 3]>(), 48);
        // LOGCONTEXTA: 名前 40 + 4 バイトの項目 33 個
        assert_eq!(std::mem::size_of::<LogContext>(), 172);
        // パケットの内容の並び（status, time, cursor, buttons, x, y, pressure, orientation）が PACKET_DATA の印の順
        assert_eq!(
            PACKET_DATA,
            0x0002 | 0x0004 | 0x0020 | 0x0040 | 0x0080 | 0x0100 | 0x0400 | 0x1000
        );
        assert_eq!(PACKET_DATA & !MOVE_MASK, PK_STATUS | PK_TIME | PK_CURSOR);
    }

    #[test]
    fn the_packet_bytes_are_read_in_the_order_of_the_data_bits() {
        let words =
            |values: &[i32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
        // 要求した並び全部: status, time, cursor, buttons, x, y, pressure, azimuth, altitude, twist
        let layout = PacketLayout::new(PACKET_DATA).unwrap();
        assert_eq!(layout.size(), 40);
        assert_eq!(
            PacketLayout::MAX_SIZE,
            layout.size(),
            "最大の並び = 要求した並び全部"
        );
        let bytes = words(&[0x10, 77, 2, 5, -30, 1234, 900, 11, -22, 33]);
        let p = layout.read(&bytes).unwrap();
        assert_eq!(
            p,
            RawPacket {
                status: 0x10,
                time: 77,
                cursor: 2,
                buttons: 5,
                x: -30,
                y: 1234,
                pressure: 900,
                orientation: Orientation {
                    azimuth: 11,
                    altitude: -22,
                    twist: 33
                },
            }
        );
        // 続きのパケットは、size 分あとから
        let two = [bytes.clone(), words(&[0, 78, 2, 0, 1, 2, 0, 0, 0, 0])].concat();
        assert_eq!(layout.read(&two[layout.size()..]).unwrap().time, 78);
        // 足りないバイト列は読まない
        assert_eq!(layout.read(&bytes[..39]), None);
        // ドライバーが向きの印を外した並び: 向きの 3 つが無く、その分短い。無い項目は 0
        let layout = PacketLayout::new(PACKET_DATA & !PK_ORIENTATION).unwrap();
        assert_eq!(layout.size(), 28);
        let p = layout.read(&words(&[0, 5, 1, 1, 10, 20, 300])).unwrap();
        assert_eq!((p.x, p.y, p.pressure), (10, 20, 300));
        assert_eq!(p.orientation, Orientation::default());
        // 位置の印が無い並びは使えない。要求の外の印（文脈のハンドル・Z など）を含む並びは読まない
        assert!(!PacketLayout::new(PK_BUTTONS | PK_STATUS)
            .unwrap()
            .has_position());
        assert!(layout.has_position());
        assert_eq!(PacketLayout::new(PACKET_DATA | PK_CONTEXT), None);
        assert_eq!(PacketLayout::new(PACKET_DATA | PK_ROTATION), None);
    }

    #[test]
    fn our_context_changes_only_the_packet_and_output_settings() {
        let mut default = LogContext::zeroed();
        default.options = CXO_SYSTEM;
        default.msg_base = WT_DEFBASE;
        default.device = 0;
        default.pkt_data = PK_X | PK_Y | PK_BUTTONS;
        default.pkt_mode = PK_BUTTONS;
        default.in_org = [10, 20, 0];
        default.in_ext = [15200, 9500, 1023];
        default.out_org = [0, 0, 0];
        default.out_ext = [1920, -1080, 0];
        default.sys_mode = 0;
        default.sys_org = [-1920, 0];
        default.sys_ext = [3840, 1080];
        default.locks = 0;
        let ours = LogContext::ours(&default);
        assert_eq!(ours.pkt_data, PACKET_DATA);
        assert_eq!(ours.pkt_mode, 0, "ボタンは押している物のビット（絶対）");
        assert_eq!(ours.move_mask, MOVE_MASK);
        assert_eq!(ours.btn_dn_mask, u32::MAX);
        assert_eq!(ours.btn_up_mask, u32::MAX);
        assert!(ours.options & CXO_SYSTEM != 0 && ours.options & CXO_MESSAGES != 0);
        assert_eq!(&ours.name[..11], b"YoluPainter");
        assert_eq!(ours.name[11], 0);
        // 出力は機器の広さ（丸められる前の値）
        assert_eq!(ours.out_org, default.in_org);
        assert_eq!(ours.out_ext, default.in_ext);
        // 利用者が決めた範囲と割り当てには触れない
        assert_eq!(ours.in_org, default.in_org);
        assert_eq!(ours.in_ext, default.in_ext);
        assert_eq!(ours.sys_org, default.sys_org);
        assert_eq!(ours.sys_ext, default.sys_ext);
        assert_eq!(ours.sys_mode, default.sys_mode);
        assert_eq!(ours.locks, default.locks);
        assert_eq!(ours.msg_base, default.msg_base);
        assert_eq!(ours.device, default.device);
    }

    #[test]
    fn messages_are_found_from_the_message_base() {
        assert_eq!(message_offset(WT_DEFBASE, WT_DEFBASE), Some(WT_PACKET));
        assert_eq!(
            message_offset(WT_DEFBASE + 5, WT_DEFBASE),
            Some(WT_PROXIMITY)
        );
        assert_eq!(message_offset(WT_DEFBASE + 0xF, WT_DEFBASE), Some(0xF));
        assert_eq!(message_offset(WT_DEFBASE - 1, WT_DEFBASE), None);
        assert_eq!(message_offset(WT_DEFBASE + 0x10, WT_DEFBASE), None);
        // 別の基準
        assert_eq!(message_offset(0x7A02, 0x7A00), Some(2));
        assert_eq!(message_offset(0x0200, WT_DEFBASE), None, "WM_MOUSEMOVE");
    }

    #[test]
    fn the_tablet_position_is_a_fraction_of_the_output_range_with_the_bottom_at_zero() {
        let range = OutputRange {
            org: [100, 200],
            ext: [1000, 500],
        };
        assert_eq!(range.fraction(100, 200), Some([0.0, 0.0]));
        assert_eq!(range.fraction(1100, 700), Some([1.0, 1.0]));
        assert_eq!(range.fraction(600, 450), Some([0.5, 0.5]));
        // 範囲の外は端
        assert_eq!(range.fraction(-50, 9999), Some([0.0, 1.0]));
        // 広さが負の軸は反転している: 範囲は org から org + |ext|（0〜500）で、割合は反対向き（org 側が機器の上）
        let flipped = OutputRange {
            org: [0, 0],
            ext: [1000, -500],
        };
        assert_eq!(flipped.fraction(500, 250), Some([0.5, 0.5]));
        assert_eq!(flipped.fraction(0, 0), Some([0.0, 1.0]));
        assert_eq!(flipped.fraction(1000, 500), Some([1.0, 0.0]));
        // 範囲の外（負の側）は端
        assert_eq!(flipped.fraction(500, -250), Some([0.5, 1.0]));
        // 広さが負の X 軸も反転
        let flipped_x = OutputRange {
            org: [100, 0],
            ext: [-1000, 500],
        };
        assert_eq!(flipped_x.fraction(100, 0), Some([1.0, 0.0]));
        assert_eq!(flipped_x.fraction(1100, 500), Some([0.0, 1.0]));
        // 広さ 0 は割れない
        let broken = OutputRange {
            org: [0, 0],
            ext: [0, 100],
        };
        assert_eq!(broken.fraction(1, 1), None);
    }

    #[test]
    fn the_screen_position_keeps_the_fraction_and_flips_y() {
        let screen = Rect::new(0, 0, 1920, 1080);
        // 機器の左下 → 画面の左下、右上 → 右上
        assert_eq!(to_screen([0.0, 0.0], screen), [0.0, 1080.0]);
        assert_eq!(to_screen([1.0, 1.0], screen), [1920.0, 0.0]);
        // 整数に丸めない（機器の細かい位置が残る）
        let p = to_screen([0.123456, 0.654321], screen);
        assert!((p[0] - 237.03552).abs() < 1e-9, "{p:?}");
        assert!((p[1] - 373.33332).abs() < 1e-3, "{p:?}");
        // 画面の一部（左のモニター。原点が負）
        let left = Rect::new(-1920, 0, 1920, 1080);
        assert_eq!(to_screen([0.5, 0.5], left), [-960.0, 540.0]);
        // 画面の一部（右のモニターが縦にずれている）
        let right = Rect::new(1920, 200, 1280, 720);
        assert_eq!(to_screen([0.0, 1.0], right), [1920.0, 200.0]);
        assert_eq!(to_screen([1.0, 0.0], right), [3200.0, 920.0]);
    }

    #[test]
    fn a_screen_position_splits_into_a_pixel_and_a_non_negative_fraction_even_left_of_the_primary_monitor(
    ) {
        assert_eq!(split_position(100.25), Some((100, 0.25)));
        assert_eq!(split_position(0.0), Some((0, 0.0)));
        // 主のモニターより左・上（負）。端数は 0 以上で、加えると元の値
        assert_eq!(split_position(-1.25), Some((-2, 0.75)));
        assert_eq!(split_position(-1920.5), Some((-1921, 0.5)));
        let (pixel, fraction) = split_position(-3.1).unwrap();
        assert!((0.0..1.0).contains(&fraction));
        assert!((pixel as f64 + fraction as f64 - -3.1).abs() < 1e-6);
        // 数でない・整数に入らない
        assert_eq!(split_position(f64::NAN), None);
        assert_eq!(split_position(f64::INFINITY), None);
        assert_eq!(split_position(1e12), None);
    }

    const DESKTOP: Rect = Rect {
        left: 0.0,
        top: 0.0,
        width: 3840.0,
        height: 1080.0,
    };
    const LEFT: Rect = Rect {
        left: 0.0,
        top: 0.0,
        width: 1920.0,
        height: 1080.0,
    };
    const RIGHT: Rect = Rect {
        left: 1920.0,
        top: 0.0,
        width: 1920.0,
        height: 1080.0,
    };

    /// 横に 2 枚のモニター。ドライバーは仮想デスクトップ全体を言う。
    fn two_monitors() -> ScreenMap {
        ScreenMap::new(Some(DESKTOP), &[LEFT, RIGHT], Some(DESKTOP))
    }

    /// 機器の横の位置 x（`truth` に割り当てられた画面の x。y は中ほど）で、1 回読んだときの写し先。
    fn along(map: &mut ScreenMap, truth: Rect, x: f64, pressing: bool) -> Option<(MapKind, Rect)> {
        let fraction = [x / truth.width, 0.5];
        map.settle(fraction, to_screen(fraction, truth), pressing)
    }

    #[test]
    fn a_wider_candidate_is_not_kept_at_the_edge_of_the_screen_it_is_chosen_only_after_two_matches_apart(
    ) {
        // 利用者は左のモニターに割り当て、ドライバーは仮想デスクトップ全体（3840 幅）を言う。
        // 画面の左の端（5 画素）ではドライバーの写しも 5 画素のずれ（許し内）だが、1 つ目のモニターは 0 → 最も合う方。1 か所では掴まない
        let mut map = two_monitors();
        assert_eq!(along(&mut map, LEFT, 5.0, false), None);
        // 40 画素動いただけ（離れていない）では掴まない
        assert_eq!(along(&mut map, LEFT, 45.0, false), None);
        // 離れた 2 か所目で掴む。ドライバーではなく 1 つ目のモニター
        let grabbed = along(&mut map, LEFT, 130.0, false).expect("掴んだ");
        assert_eq!((grabbed.0, grabbed.1), (MapKind::Monitor(0), LEFT));
        // 以後は保つ。画面の端から端まで、写した位置はカーソルとの差が許し内（ずれたまま描かない・途中で跳ばない）
        for x in [300.0, 900.0, 1500.0, 1900.0] {
            let fraction = [x / 1920.0, 0.5];
            let (kind, rect) = along(&mut map, LEFT, x, false).expect("保つ");
            assert_eq!(kind, MapKind::Monitor(0));
            let mapped = to_screen(fraction, rect);
            let truth = to_screen(fraction, LEFT);
            assert!((mapped[0] - truth[0]).abs() + (mapped[1] - truth[1]).abs() <= HOLD_TOLERANCE);
        }
        // 端から 63 画素のところから始めても（ドライバーの写しが 63 画素ずれる位置）、ドライバーは掴まない
        let mut map = two_monitors();
        let mut seen = Vec::new();
        for x in [63.0, 120.0, 200.0, 400.0, 800.0, 1600.0] {
            seen.push(along(&mut map, LEFT, x, false).map(|(k, _)| k));
        }
        assert!(!seen.contains(&Some(MapKind::Driver)), "{seen:?}");
        assert_eq!(seen.last().copied().flatten(), Some(MapKind::Monitor(0)));
        // 端のすぐそば（ドライバーも合う位置）からでも、より合う 1 つ目のモニターを掴む（動くと、ドライバーは合わなくなる）
        let mut map = two_monitors();
        along(&mut map, LEFT, 2.0, false);
        let grabbed = along(&mut map, LEFT, 500.0, false).expect("掴んだ");
        assert_eq!(grabbed.0, MapKind::Monitor(0));
    }

    #[test]
    fn the_driver_range_is_chosen_when_it_is_the_one_that_fits() {
        let mut map = two_monitors();
        assert_eq!(along(&mut map, DESKTOP, 1000.0, false), None);
        let grabbed = along(&mut map, DESKTOP, 2500.0, false).expect("掴んだ");
        // ドライバーと仮想デスクトップは同じ矩形で、同じ程度に合うときは前（ドライバー）
        assert_eq!(grabbed.0, MapKind::Driver);
    }

    #[test]
    fn a_coincidence_in_mouse_mode_is_not_grabbed() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let mut map = ScreenMap::new(Some(screen), &[], None);
        // マウスモード: カーソルはペンと無関係に動く。1 か所だけ偶然重なっても、その回はカーソルの位置（掴まない）
        let f = [0.5, 0.5];
        let on = to_screen(f, screen);
        assert_eq!(map.settle(f, [on[0] + 5.0, on[1] + 1.0], false), None);
        // 次の回は離れた所で合わない: 待っていた物も忘れる
        assert_eq!(map.settle([0.1, 0.9], [1500.0, 200.0], false), None);
        // また離れた所で偶然重なっても、間に合わない回があったので、1 か所目として数え直す
        let g = [0.9, 0.1];
        let on = to_screen(g, screen);
        assert_eq!(map.settle(g, [on[0] - 3.0, on[1] + 3.0], false), None);
        assert_eq!(map.current(), None);
        // 偶然が近い所で続いても（離れていない）、掴まない
        assert_eq!(map.settle(g, [on[0] - 2.0, on[1] + 2.0], false), None);
        assert_eq!(map.current(), None);
        // 相対モード（候補を持たない）は、何度合っても掴めない
        let mut none = ScreenMap::none();
        for x in [100.0, 600.0, 1200.0] {
            assert_eq!(along(&mut none, screen, x, false), None);
        }
        assert_eq!(none.current(), None);
    }

    #[test]
    fn the_map_is_held_through_a_hiccup_and_not_changed_while_pressing() {
        let mut map = two_monitors();
        along(&mut map, LEFT, 5.0, false);
        along(&mut map, LEFT, 300.0, false).expect("掴んだ");
        // 一時の遅れ（13 画素）の回はカーソルの位置。写しは残り、次に合えばそのまま使う（掴み直さない）
        let f = [0.5, 0.5];
        let exact = to_screen(f, LEFT);
        let near = |dx: f64| [exact[0] + dx, exact[1]];
        assert_eq!(
            map.settle(f, near(11.0), false).unwrap().0,
            MapKind::Monitor(0)
        );
        assert_eq!(map.settle(f, near(13.0), false), None);
        assert_eq!(map.current().unwrap().0, MapKind::Monitor(0));
        assert_eq!(
            map.settle(f, near(0.0), false).unwrap().0,
            MapKind::Monitor(0)
        );
        // 割り当てが右のモニターへ変わった。押しの途中は、合わなくても替えない（その回の点はカーソルの位置）
        assert_eq!(along(&mut map, RIGHT, 100.0, true), None);
        assert_eq!(along(&mut map, RIGHT, 400.0, true), None);
        assert_eq!(map.current().unwrap().0, MapKind::Monitor(0));
        // 離したあと、1 か所では替えない。離れた 2 か所目で替える
        assert_eq!(along(&mut map, RIGHT, 100.0, false), None);
        assert_eq!(along(&mut map, RIGHT, 150.0, false), None);
        let switched = along(&mut map, RIGHT, 400.0, false).expect("替えた");
        assert_eq!((switched.0, switched.1), (MapKind::Monitor(1), RIGHT));
        // 押しの途中に始まった（写しが無い）ときも、掴まない
        let mut map = two_monitors();
        assert_eq!(along(&mut map, LEFT, 5.0, true), None);
        assert_eq!(along(&mut map, LEFT, 400.0, true), None);
        assert_eq!(map.current(), None);
        // 矩形になっていない候補は入れない
        let mut empty = ScreenMap::new(Some(Rect::new(0, 0, 0, 0)), &[], None);
        assert_eq!(empty.settle([0.5, 0.5], [1.0, 1.0], false), None);
    }

    /// 写し先を掴ませる（離れた 2 か所で合う）。
    fn grabbed(screen: Rect) -> ScreenMap {
        let mut map = ScreenMap::new(Some(screen), &[], None);
        along(&mut map, screen, 10.0, false);
        along(&mut map, screen, screen.width - 10.0, false).expect("掴んだ");
        map
    }

    #[test]
    fn a_batch_is_mapped_in_floating_point_or_put_on_the_cursor() {
        let screen = Rect::new(0, 0, 2000, 1000);
        let range = OutputRange {
            org: [0, 0],
            ext: [20000, 10000],
        };
        let p = |x, y| RawPacket { x, y, ..packet() };
        let batch = [p(1, 9999), p(10001, 5000), p(20000, 10000)];
        // 写し先を掴んだあと、最後の点（右上）のカーソルが合っている → 全部写す。1 つ目は整数に丸められていない
        let mut map = grabbed(screen);
        let cursor = to_screen([1.0, 1.0], screen);
        let out = batch_positions(&batch, &range, &mut map, Some(cursor), false);
        assert_eq!(out.len(), 3);
        let first = out[0].unwrap();
        assert!((first[0] - 0.1).abs() < 1e-9, "{first:?}");
        assert!((first[1] - 0.1).abs() < 1e-9, "{first:?}");
        let mid = out[1].unwrap();
        assert!((mid[0] - 1000.1).abs() < 1e-9 && (mid[1] - 500.0).abs() < 1e-9);
        // まだ写し先を掴んでいない回は、全部カーソルの位置
        let mut fresh = ScreenMap::new(Some(screen), &[], None);
        let out = batch_positions(&batch, &range, &mut fresh, Some(cursor), false);
        assert_eq!(out, vec![Some(cursor); 3]);
        // 合わないカーソル → 全部カーソルの位置
        let mut map = grabbed(screen);
        let out = batch_positions(&batch, &range, &mut map, Some([5.0, 5.0]), false);
        assert_eq!(out, vec![Some([5.0, 5.0]); 3]);
        // カーソルが読めない → 今の写し先で写す
        let mut map = grabbed(screen);
        let out = batch_positions(&batch, &range, &mut map, None, false);
        assert!(out.iter().all(Option::is_some));
        // 候補もカーソルも無い → 点の位置が決まらない
        let mut map = ScreenMap::none();
        let out = batch_positions(&batch, &range, &mut map, None, false);
        assert_eq!(out, vec![None; 3]);
        // 空の回
        assert!(batch_positions(&[], &range, &mut map, Some([0.0, 0.0]), false).is_empty());
        // 広さ 0 の範囲 → カーソルの位置
        let broken = OutputRange {
            org: [0, 0],
            ext: [0, 0],
        };
        let mut map = grabbed(screen);
        assert_eq!(
            batch_positions(&batch[..1], &broken, &mut map, Some([7.0, 8.0]), false),
            vec![Some([7.0, 8.0])]
        );
        // 触れている最中の回は替えない: 前の回から触れたまま、または回の中に触れた点があるとき
        let mut map = grabbed(screen);
        let cursor = to_screen([1.0, 1.0], screen);
        let right = Rect::new(2000, 0, 2000, 1000);
        let elsewhere = to_screen([1.0, 1.0], right);
        for (touching, packets) in [
            (true, vec![p(20000, 10000)]),
            (
                false,
                vec![RawPacket {
                    buttons: 1,
                    ..p(20000, 10000)
                }],
            ),
        ] {
            let out = batch_positions(&packets, &range, &mut map, Some(elsewhere), touching);
            assert_eq!(out, vec![Some(elsewhere)], "合わない回はカーソルの位置");
        }
        assert!(map.current().is_some(), "写しは残る");
        let _ = cursor;
    }

    const INK: u32 = 7;

    fn sample(contact: bool, pointer_id: u32) -> PenSample {
        PenSample {
            pos: [1.0, 2.0],
            pressure: if contact { 0.5 } else { 0.0 },
            tilt: Tilt::default(),
            rotation: None,
            contact,
            eraser: false,
            barrel: false,
            pointer_id,
            time_ms: 0,
        }
    }

    fn tab(contact: bool) -> PenSample {
        sample(contact, POINTER_ID)
    }

    fn ink(contact: bool) -> PenSample {
        sample(contact, INK)
    }

    /// 出た点の中の、押しの始まりの数（触れていない → 触れた の変わり目）。
    fn presses(out: &[PenSample]) -> usize {
        let mut before = false;
        let mut count = 0;
        for s in out {
            count += usize::from(s.contact && !before);
            before = s.contact;
        }
        count
    }

    #[test]
    fn a_press_that_wintab_started_stays_wintabs_and_the_ink_points_of_the_same_press_are_dropped()
    {
        let mut owner = PressOwner::default();
        let mut out = Vec::new();
        // 浮いている間: WinTab の点は入る。Windows Ink の浮いた点は入らない
        out.extend(route_tab(&mut owner, vec![tab(false), tab(false)]));
        out.extend(route_ink(&mut owner, InkMessage::Update, vec![ink(false)]));
        assert_eq!(out.len(), 2);
        // WinTab の触れた点が先に来て（押しの前に取ったパケット）、そのあとに Windows Ink の押しが来る
        out.extend(route_tab(&mut owner, vec![tab(true)]));
        assert_eq!(owner.current(), Some(Owner::WinTab));
        out.extend(route_ink(&mut owner, InkMessage::Down, vec![ink(true)]));
        assert_eq!(owner.current(), Some(Owner::WinTab), "持ち主は替わらない");
        out.extend(route_ink(
            &mut owner,
            InkMessage::Update,
            vec![ink(true), ink(true)],
        ));
        out.extend(route_tab(&mut owner, vec![tab(true), tab(true)]));
        out.extend(route_ink(&mut owner, InkMessage::Up, vec![ink(false)]));
        assert_eq!(
            owner.current(),
            Some(Owner::WinTab),
            "Windows Ink の離しでは手放さない"
        );
        out.extend(route_tab(&mut owner, vec![tab(false)]));
        assert_eq!(owner.current(), None);
        assert_eq!(presses(&out), 1, "押しは 1 本");
        assert!(out.iter().all(|s| s.pointer_id == POINTER_ID));
        assert_eq!(out.len(), 2 + 1 + 2 + 1);
    }

    #[test]
    fn a_press_that_ink_started_stays_inks_even_when_the_wintab_packets_arrive_late() {
        let mut owner = PressOwner::default();
        let mut out = Vec::new();
        // 後ろにあるウィンドウをペンで押した: 押しの時点では WinTab のパケットは無い（文脈が無効）→ Windows Ink の押し
        out.extend(route_ink(&mut owner, InkMessage::Down, vec![ink(true)]));
        assert_eq!(owner.current(), Some(Owner::Ink));
        // そのあと文脈が有効になって WinTab のパケットが来ても、この押しの間は入れない（触れた点も浮いた点も）
        out.extend(route_tab(
            &mut owner,
            vec![tab(true), tab(true), tab(false), tab(true)],
        ));
        out.extend(route_ink(
            &mut owner,
            InkMessage::Update,
            vec![ink(true), ink(true)],
        ));
        out.extend(route_ink(&mut owner, InkMessage::Up, vec![ink(false)]));
        assert_eq!(owner.current(), None, "離しで手放す");
        assert_eq!(presses(&out), 1, "押しは 1 本");
        assert!(out.iter().all(|s| s.pointer_id == INK));
        assert_eq!(out.len(), 1 + 2 + 1);
        // 離したあとは、また WinTab の点が入る。次の押しは WinTab の物になれる
        let mut next = route_tab(&mut owner, vec![tab(false), tab(true)]);
        assert_eq!(owner.current(), Some(Owner::WinTab));
        next.extend(route_tab(&mut owner, vec![tab(false)]));
        assert_eq!(next.len(), 3);
        assert_eq!(presses(&next), 1);
    }

    #[test]
    fn an_ink_touch_without_a_down_message_starts_an_ink_press_and_hover_comes_from_wintab_only() {
        let mut owner = PressOwner::default();
        // 押しの始まりを見逃した Windows Ink の触れた点: 押しの始まりとして入れる
        let started = route_ink(&mut owner, InkMessage::Update, vec![ink(true)]);
        assert_eq!(started.len(), 1);
        assert_eq!(owner.current(), Some(Owner::Ink));
        // この押しの間の WinTab の点は入らない
        assert!(route_tab(&mut owner, vec![tab(true), tab(false)]).is_empty());
        // 浮いた点（触れていない）で手放す
        assert_eq!(
            route_ink(&mut owner, InkMessage::Update, vec![ink(false)]).len(),
            1
        );
        assert_eq!(owner.current(), None);
        // 浮いている間の Windows Ink の点は入らない（WinTab が出す）
        assert!(route_ink(&mut owner, InkMessage::Update, vec![ink(false), ink(false)]).is_empty());
        assert_eq!(route_tab(&mut owner, vec![tab(false)]).len(), 1);
        // 離しが来ないまま止まったときは、呼ぶ側が手放す。手放したあとの Windows Ink の押しは Windows Ink の物
        route_tab(&mut owner, vec![tab(true)]);
        assert_eq!(owner.current(), Some(Owner::WinTab));
        owner.reset();
        assert_eq!(owner.ink_down(), Owner::Ink);
        // Windows Ink の押しの終わりは、WinTab の物の押しを手放さない
        let mut owner = PressOwner::default();
        route_tab(&mut owner, vec![tab(true)]);
        owner.ink_up();
        assert_eq!(owner.current(), Some(Owner::WinTab));
        // 同じ押しの `Down` を 2 度受けても、持ち主は替わらない
        assert_eq!(owner.ink_down(), Owner::WinTab);
        assert_eq!(owner.ink_down(), Owner::WinTab);
    }

    #[test]
    fn two_presses_in_a_row_each_have_one_owner_and_never_start_twice() {
        // 1 本目は WinTab の物、2 本目は Windows Ink の物（WinTab が遅れた）、3 本目はまた WinTab の物
        let mut owner = PressOwner::default();
        let mut out = Vec::new();
        out.extend(route_tab(
            &mut owner,
            vec![tab(true), tab(true), tab(false)],
        ));
        out.extend(route_ink(&mut owner, InkMessage::Down, vec![ink(true)]));
        out.extend(route_tab(&mut owner, vec![tab(true)]));
        out.extend(route_ink(&mut owner, InkMessage::Up, vec![ink(false)]));
        out.extend(route_tab(
            &mut owner,
            vec![tab(false), tab(true), tab(false)],
        ));
        assert_eq!(presses(&out), 3);
        let ids: Vec<u32> = out.iter().map(|s| s.pointer_id).collect();
        assert_eq!(
            ids,
            // WinTab の押し（触れる・触れる・離す）、Windows Ink の押し（触れる・離す）、WinTab の押し（浮く・触れる・離す）
            [POINTER_ID, POINTER_ID, POINTER_ID, INK, INK, POINTER_ID, POINTER_ID, POINTER_ID]
        );
    }

    #[test]
    fn a_context_is_on_top_only_when_the_overlap_status_says_so() {
        assert!(overlap_on_top(CXS_ONTOP as isize));
        assert!(overlap_on_top((CXS_ONTOP | CXS_OBSCURED) as isize));
        assert!(!overlap_on_top(CXS_DISABLED as isize));
        assert!(!overlap_on_top(CXS_OBSCURED as isize));
        assert!(!overlap_on_top(0));
        // 上位のビットは見ない
        assert!(!overlap_on_top(0x1_0000));
    }

    #[test]
    fn pressure_is_normalized_by_the_device_range_even_when_the_minimum_is_not_zero() {
        let a = axis(0, 1023);
        assert_eq!(normalize_pressure(0, &a), Some(0.0));
        assert_eq!(normalize_pressure(1023, &a), Some(1.0));
        let half = normalize_pressure(512, &a).unwrap();
        assert!((half - 512.0 / 1023.0).abs() < 1e-6);
        // 最小が 0 でない機器
        let b = axis(100, 1100);
        assert_eq!(normalize_pressure(100, &b), Some(0.0));
        assert_eq!(normalize_pressure(600, &b), Some(0.5));
        assert_eq!(normalize_pressure(1100, &b), Some(1.0));
        // 範囲の外は端
        assert_eq!(normalize_pressure(50, &b), Some(0.0));
        assert_eq!(normalize_pressure(5000, &b), Some(1.0));
        // 範囲が使えない（筆圧を持たない）
        assert_eq!(normalize_pressure(5, &axis(0, 0)), None);
        assert_eq!(normalize_pressure(5, &axis(10, 5)), None);
    }

    #[test]
    fn azimuth_and_altitude_become_the_tilt_of_windows_ink() {
        let circles = [3600.0, 3600.0];
        let tilt = |azimuth: i32, altitude: i32| {
            tilt_degrees(
                &Orientation {
                    azimuth,
                    altitude,
                    twist: 1,
                },
                circles,
            )
        };
        let near = |a: Tilt, x: f32, y: f32| (a.x - x).abs() < 1e-3 && (a.y - y).abs() < 1e-3;
        // 垂直（高さ 90°）はどの方位でも傾き無し
        assert!(near(tilt(0, 900), 0.0, 0.0));
        assert!(near(tilt(1234, 900), 0.0, 0.0));
        // 45° に倒す: 方位 0（後ろの端が機器の上）は画面の奥へ（y が負）、90° は右（x が正）、
        // 180° は利用者の側（y が正）、270° は左（x が負）
        assert!(near(tilt(0, 450), 0.0, -45.0), "{:?}", tilt(0, 450));
        assert!(near(tilt(900, 450), 45.0, 0.0), "{:?}", tilt(900, 450));
        assert!(near(tilt(1800, 450), 0.0, 45.0), "{:?}", tilt(1800, 450));
        assert!(near(tilt(2700, 450), -45.0, 0.0), "{:?}", tilt(2700, 450));
        // 裏返し（高さが負）も同じ向きの傾き
        assert!(near(tilt(900, -450), 45.0, 0.0));
        // ほとんど寝かせても 90° を越えない
        let flat = tilt(900, 1);
        assert!(flat.x > 85.0 && flat.x <= 90.0, "{flat:?}");
        assert!(near(tilt(900, 0), 90.0, 0.0));
        // 方位と高さが 45° ずつ: 両方の成分
        let diagonal = tilt(450, 450);
        assert!(diagonal.x > 0.0 && diagonal.y < 0.0, "{diagonal:?}");
        // 向きを送っていない（全部 0）なら垂直
        assert_eq!(
            tilt_degrees(&Orientation::default(), circles),
            Tilt::default()
        );
        // 円周あたりの数が違う機器（0.01 度刻み）でも同じ角度
        let fine = tilt_degrees(
            &Orientation {
                azimuth: 9000,
                altitude: 4500,
                twist: 0,
            },
            [36000.0, 36000.0],
        );
        assert!(near(fine, 45.0, 0.0), "{fine:?}");
        // 範囲の外の高さも 90° 以内
        let wild = tilt(900, 99999);
        assert!(wild.x.abs() <= 90.0 && wild.y.abs() <= 90.0);
    }

    #[test]
    fn the_orientation_axes_come_from_the_resolutions() {
        let axes = [tenths(), tenths(), tenths()];
        let o = OrientationAxes::from_axes(&axes);
        assert_eq!(o.tilt_circle, Some([3600.0, 3600.0]));
        assert_eq!(o.twist_circle, Some(3600.0));
        // 解像度が 0 の軸は送れない（傾きは方位と高さの両方が要る）
        let none = Axis::default();
        let no_twist = OrientationAxes::from_axes(&[tenths(), tenths(), none]);
        assert!(no_twist.tilt_circle.is_some() && no_twist.twist_circle.is_none());
        let no_tilt = OrientationAxes::from_axes(&[none, tenths(), tenths()]);
        assert!(no_tilt.tilt_circle.is_none() && no_tilt.twist_circle.is_some());
        assert_eq!(
            OrientationAxes::from_axes(&[none; 3]),
            Device::NONE.orientation
        );
        // 単位が円周でない軸は傾きを送れない（ねじれは解像度だけで決める）
        let not_circle = Axis {
            units: 0,
            ..tenths()
        };
        let o = OrientationAxes::from_axes(&[not_circle, tenths(), tenths()]);
        assert!(o.tilt_circle.is_none() && o.twist_circle.is_some());
        let o = OrientationAxes::from_axes(&[tenths(), not_circle, tenths()]);
        assert!(o.tilt_circle.is_none());
        // 小数部のある解像度（16.16）
        let half = Axis {
            resolution: (360 << 16) + (1 << 15),
            ..tenths()
        };
        let o = OrientationAxes::from_axes(&[half, half, half]);
        assert_eq!(o.tilt_circle, Some([360.5, 360.5]));
    }

    #[test]
    fn the_twist_is_a_clockwise_rotation_in_zero_to_360() {
        assert_eq!(rotation_degrees(0, 3600.0), Some(0.0));
        assert_eq!(rotation_degrees(900, 3600.0), Some(90.0));
        assert_eq!(rotation_degrees(3599, 3600.0), Some(359.9));
        assert_eq!(rotation_degrees(3600, 3600.0), Some(0.0));
        // 負のねじれ
        assert_eq!(rotation_degrees(-900, 3600.0), Some(270.0));
        let r = rotation_degrees(7300, 3600.0).unwrap();
        assert!((0.0..360.0).contains(&r), "{r}");
        assert_eq!(rotation_degrees(5, f64::NAN), None);
    }

    #[test]
    fn the_cursor_kind_and_the_status_decide_the_eraser_end() {
        let pen = CursorInfo::default();
        let eraser = CursorInfo {
            invert: true,
            ..CursorInfo::default()
        };
        assert!(!is_eraser(0, &pen));
        assert!(is_eraser(TPS_INVERT, &pen), "パケットの状態が裏返し");
        assert!(is_eraser(0, &eraser), "裏返しを表すカーソルの種類");
        assert!(
            !is_eraser(0x1, &pen),
            "ほかの状態（近づいている）では消しゴムにならない"
        );
        // 問い合わせの答えから
        assert!(CursorInfo::from_info(Some(CRC_INVERT), None).invert);
        assert!(!CursorInfo::from_info(Some(0x3), None).invert);
        assert!(!CursorInfo::from_info(None, None).invert);
        // 送る内容: 答えが無ければ送る物として扱う。答えがあって筆圧・向きの印が無ければ送らない
        let all = CursorInfo::from_info(None, None);
        assert!(all.pressure && all.orientation);
        let puck = CursorInfo::from_info(Some(0), Some(PK_X | PK_Y | PK_BUTTONS));
        assert!(!puck.pressure && !puck.orientation);
        let pen = CursorInfo::from_info(Some(0), Some(PACKET_DATA));
        assert!(pen.pressure && pen.orientation);
    }

    #[test]
    fn the_buttons_are_bits_where_bit_zero_is_the_tip() {
        assert!(!tip_down(0) && !side_down(0));
        assert!(tip_down(1) && !side_down(1));
        assert!(!tip_down(2) && side_down(2), "下のサイドボタン");
        assert!(!tip_down(4) && side_down(4), "上のサイドボタン");
        assert!(tip_down(3) && side_down(3), "先とサイドボタン");
        assert!(side_down(0b1000), "ほかのボタンも先ではない");
    }

    #[test]
    fn the_packet_time_falls_back_to_the_received_time() {
        assert_eq!(packet_time(1234, 99), 1234);
        assert_eq!(packet_time(0, 99), 99);
    }

    fn device() -> Device {
        Device {
            pressure: Some(axis(0, 1023)),
            orientation: OrientationAxes::from_axes(&[tenths(), tenths(), tenths()]),
        }
    }

    #[test]
    fn a_packet_becomes_a_sample_with_pressure_tilt_rotation_eraser_and_barrel() {
        let p = RawPacket {
            status: TPS_INVERT,
            time: 4321,
            cursor: 2,
            buttons: 0b101,
            pressure: 512,
            orientation: Orientation {
                azimuth: 900,
                altitude: -450,
                twist: 1800,
            },
            ..packet()
        };
        let s = sample_from_packet(&p, [10.5, 20.25], &device(), &CursorInfo::default(), 7);
        assert_eq!(s.pos, [10.5, 20.25]);
        assert!((s.pressure - 512.0 / 1023.0).abs() < 1e-6);
        assert!(
            (s.tilt.x - 45.0).abs() < 1e-3 && s.tilt.y.abs() < 1e-3,
            "{:?}",
            s.tilt
        );
        assert_eq!(s.rotation, Some(180.0));
        assert!(s.contact && s.eraser && s.barrel);
        assert_eq!(s.pointer_id, POINTER_ID);
        assert_eq!(s.time_ms, 4321);
        // 浮いている（ボタンも筆圧も無い）
        let hover = sample_from_packet(&packet(), [0.0, 0.0], &device(), &CursorInfo::default(), 7);
        assert!(!hover.contact && !hover.barrel && !hover.eraser);
        assert_eq!(hover.pressure, 0.0);
        // 先のボタンが無くても筆圧があれば触れている
        let by_pressure = RawPacket {
            pressure: 10,
            ..packet()
        };
        assert!(
            sample_from_packet(&by_pressure, [0.0; 2], &device(), &CursorInfo::default(), 7)
                .contact
        );
        // 先のボタンがあれば筆圧 0 でも触れている
        let by_tip = RawPacket {
            buttons: 1,
            ..packet()
        };
        let s = sample_from_packet(&by_tip, [0.0; 2], &device(), &CursorInfo::default(), 7);
        assert!(s.contact && s.pressure == 0.0);
        // 時刻が 0 なら受け取った時刻
        assert_eq!(hover.time_ms, 1000, "パケットの時刻");
        let no_time = RawPacket {
            time: 0,
            ..packet()
        };
        assert_eq!(
            sample_from_packet(&no_time, [0.0; 2], &device(), &CursorInfo::default(), 77).time_ms,
            77
        );
    }

    #[test]
    fn a_device_or_cursor_without_pressure_or_orientation_sends_one_while_touching_and_no_tilt() {
        let tip = RawPacket {
            buttons: 1,
            pressure: 0,
            orientation: Orientation {
                azimuth: 900,
                altitude: 450,
                twist: 900,
            },
            ..packet()
        };
        // 機器に筆圧の範囲が無い
        let bare = Device::NONE;
        let s = sample_from_packet(&tip, [0.0; 2], &bare, &CursorInfo::default(), 0);
        assert!(s.contact);
        assert_eq!(s.pressure, 1.0);
        assert_eq!(s.tilt, Tilt::default());
        assert_eq!(s.rotation, None);
        // 浮いているときは 0
        let hover = sample_from_packet(&packet(), [0.0; 2], &bare, &CursorInfo::default(), 0);
        assert_eq!(hover.pressure, 0.0);
        // カーソルの種類が筆圧・向きを送らない（マウスのカーソルなど）
        let puck = CursorInfo::from_info(Some(0), Some(PK_X | PK_Y | PK_BUTTONS));
        let s = sample_from_packet(&tip, [0.0; 2], &device(), &puck, 0);
        assert!(s.contact);
        assert_eq!(s.pressure, 1.0);
        assert_eq!(s.tilt, Tilt::default());
        assert_eq!(s.rotation, None);
        // 傾きは送れるがねじれは送れない機器
        let no_twist = Device {
            orientation: OrientationAxes::from_axes(&[tenths(), tenths(), Axis::default()]),
            ..device()
        };
        let s = sample_from_packet(&tip, [0.0; 2], &no_twist, &CursorInfo::default(), 0);
        assert!(s.tilt.x > 0.0);
        assert_eq!(s.rotation, None, "ねじれの軸が無ければ回転は None");
    }

    /// パケットの組み立て（触れる・動く・離すの試験用）。
    fn send(touch: &mut Touch, buttons: u32, pressure: u32, x: f32) -> PenSample {
        let p = RawPacket {
            buttons,
            pressure,
            ..packet()
        };
        let s = sample_from_packet(&p, [x, 5.0], &device(), &CursorInfo::default(), 0);
        touch.note(&s);
        s
    }

    #[test]
    fn touching_moving_and_lifting_follow_the_packets_and_a_lift_can_be_made_on_demand() {
        let mut touch = Touch::default();
        // 浮く → 触れる → 動く → 離す
        let mut contacts = Vec::new();
        for (buttons, pressure, x) in [(0, 0, 1.0), (1, 300, 2.0), (1, 600, 3.0), (0, 0, 4.0)] {
            contacts.push(send(&mut touch, buttons, pressure, x).contact);
        }
        assert_eq!(contacts, [false, true, true, false]);
        assert!(!touch.is_touching());
        // 離したあとは、離しの点を作らない（押しは残っていない）
        assert_eq!(touch.release(), None);
        // 触れたまま止める → 最後の位置の離しの点が 1 度だけ
        let last = send(&mut touch, 0b11, 700, 9.0);
        assert!(touch.is_touching());
        let lifted = touch.release().expect("触れていた");
        assert_eq!(lifted.pos, last.pos);
        assert!(!lifted.contact && !lifted.barrel && lifted.pressure == 0.0);
        assert_eq!(lifted.pointer_id, last.pointer_id);
        assert_eq!(touch.release(), None, "1 度だけ");
        assert!(!touch.is_touching());
    }

    #[test]
    fn the_log_lines_carry_the_raw_values_and_the_converted_sample() {
        let p = RawPacket {
            status: 0x10,
            time: 77,
            cursor: 2,
            buttons: 5,
            x: 1234,
            y: 5678,
            pressure: 900,
            orientation: Orientation {
                azimuth: 1,
                altitude: -2,
                twist: 3,
            },
        };
        let line = log_packet(&p);
        for part in [
            "time=77",
            "cursor=2",
            "status=0x10",
            "buttons=0x5",
            "x=1234",
            "y=5678",
            "pressure=900",
            "azimuth=1",
            "altitude=-2",
            "twist=3",
        ] {
            assert!(line.contains(part), "{line}");
        }
        assert!(!line.contains('\n'));
        let s = sample_from_packet(&p, [1.5, 2.5], &device(), &CursorInfo::default(), 0);
        let line = log_sample(&s, Some([10.0, 20.0]), &map_name(Some(MapKind::Monitor(1))));
        for part in [
            "screen=(10.000,20.000)",
            "map=monitor1",
            "pos=(1.500,2.500)",
            "contact=true",
            "eraser=true",
            "barrel=true",
            "time=77",
        ] {
            assert!(line.contains(part), "{line}");
        }
        assert!(!line.contains('\n'));
        let none = log_sample(&s, None, &map_name(None));
        assert!(none.contains("screen=none") && none.contains("map=cursor"));
        assert_eq!(
            log_press(Some(Owner::Ink), "POINTERDOWN", 0, false),
            "down owner=ink msg=POINTERDOWN pumped=0 contact=false"
        );
        assert_eq!(
            log_press(None, "LBUTTONDOWN", 3, true),
            "down owner=none msg=LBUTTONDOWN pumped=3 contact=true"
        );
        assert_eq!(map_name(Some(MapKind::Driver)), "driver");
        assert_eq!(map_name(Some(MapKind::Desktop)), "desktop");
    }

    #[test]
    fn every_reason_has_one_japanese_and_one_english_sentence_naming_the_fallback() {
        for why in [
            Unavailable::NoLibrary,
            Unavailable::NoDriver,
            Unavailable::NoTablet,
            Unavailable::NoContext,
        ] {
            let ja = why.text(Lang::Ja);
            let en = why.text(Lang::En);
            assert!(ja.ends_with('。') && ja.matches('。').count() == 1, "{ja}");
            assert!(en.ends_with('.') && en.matches('.').count() == 1, "{en}");
            assert!(ja.contains("Windows Ink") && en.contains("Windows Ink"));
            assert!(ja.contains("WinTab") && en.contains("WinTab"));
            // 日本語の文に英語の文は混ざらない
            assert!(!ja.contains("Using") && !en.contains("使います"));
        }
        // 理由ごとに文が違う
        let all: std::collections::HashSet<&str> = [
            Unavailable::NoLibrary,
            Unavailable::NoDriver,
            Unavailable::NoTablet,
            Unavailable::NoContext,
        ]
        .iter()
        .map(|w| w.text(Lang::En))
        .collect();
        assert_eq!(all.len(), 4);
    }
}
