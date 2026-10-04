//! 今の版の正本（1 層の文書）から、古い版の並びを作る試験用の道具（C# の `ArchiveTestUtil.AsVersion` と同じ）。
//! 古い版の書き手が無い（Unity 版も今の版しか書かない）ので、並びの違いを今の書き手の出力から作る。
//! 版 9 以前の各版が足したもの（各層の最後の 1 バイト）、版 7 の Normal の出力設定、版 6 の親グループ、版 5 のクリッピングを抜く。
//! 版 10 から今の版の 1 つ前までは、版 11 の Generator の段・版 12 のロック・版 13 の形のグラデーション・版 14 のチャンネルごとの合成・
//! 版 15 の ID の色の Generator・版 16 の塗りつぶしの画像と投影とデカール・版 18 のパスのマテリアル・版 19 の手動の ID 色・版 20 の Anchor・
//! 版 21 の勾配のどれも持たない文書は、版の数だけが違う並びになるので、版の数だけを書き換える。
#![allow(dead_code)]

/// 版 7 で文書の頭（タイルの大きさの直後）に足した Normal の出力設定のバイト数（アルゴリズム版 4・派生 1・強さ 8・端 4・向き 4）。
pub const NORMAL_SETTINGS_BYTES: usize = 4 + 1 + 8 + 4 + 4;
/// Normal の出力設定の位置（識別子 8・版 4・文書 ID 16・幅・高さ・タイル 12 の後）。
pub const NORMAL_SETTINGS_OFFSET: usize = 8 + 4 + 16 + 12;
/// 今の書き手（ユーザーチャンネルの無い文書）の版。
pub const CURRENT_VERSION: i32 = 21;

fn set_version(bytes: &mut [u8], version: i32) {
    bytes[8..12].copy_from_slice(&version.to_le_bytes());
}

/// 1 層の文書の、各層の属性の 1 バイト（版 12 から。それ以前はクリッピングの 1 バイト）の位置。Normal の出力設定があるとき（版 7 以降）。
pub fn attribute_byte(layer_name: &str) -> usize {
    // 識別子・版・文書 ID・幅・高さ・タイル、Normal の設定、層の数、層 ID、名前（長さ + 本体）、表示、不透明度、合成モードの後
    NORMAL_SETTINGS_OFFSET + NORMAL_SETTINGS_BYTES + 4 + 16 + 4 + layer_name.len() + 1 + 8 + 4
}

/// `current`（版 21 の 1 層の文書。層の名前は `layer_name`）を `target` 版の並びにする。
/// 3 以上 20 以下: 10 以上は版の数だけ。9 は各層の最後の 2D パスの有無（版 10）を抜く。8 はフィルターの有無（版 9）も、7 はパスの有無（版 8）も、
/// 6 は Normal の出力設定（版 7）も、5 は親グループの ID（版 6）も、4 以下はクリッピングの 1 バイト（版 5）も抜く。
/// 版 3 以下で無くなる種類（調整・グループ）を持つ文書、版 2 以下の並び（種類と塗りつぶしの値が無い）には使えない。
pub fn as_version(current: &[u8], layer_name: &str, target: i32) -> Vec<u8> {
    assert!(
        (3..CURRENT_VERSION).contains(&target),
        "版 {target} は作れない"
    );
    assert_eq!(
        i32::from_le_bytes(current[8..12].try_into().unwrap()),
        CURRENT_VERSION,
        "今の版から変える"
    );
    let mut bytes = current.to_vec();
    if target >= 10 {
        set_version(&mut bytes, target);
        return bytes;
    }
    for (version, what) in [(10, "2D パス"), (9, "フィルター"), (8, "パス")] {
        // 1 層の文書では、各版が足した有無の 1 バイトが、ファイルの最後から順に並ぶ
        assert_eq!(bytes.pop(), Some(0), "{what}の無い層");
        if target == version - 1 {
            set_version(&mut bytes, target);
            return bytes;
        }
    }
    bytes.drain(NORMAL_SETTINGS_OFFSET..NORMAL_SETTINGS_OFFSET + NORMAL_SETTINGS_BYTES);
    if target == 6 {
        set_version(&mut bytes, 6);
        return bytes;
    }
    // 識別子・版・文書 ID・幅・高さ・タイル・層の数・層 ID・名前・表示・不透明度・合成モードの次がクリッピングの 1 バイト、
    // そのあとに種類の 4 バイト、親グループの ID（16 バイト）
    let clipping = attribute_byte(layer_name) - NORMAL_SETTINGS_BYTES;
    let parent = clipping + 1 + 4;
    assert!(
        bytes[parent..parent + 16].iter().all(|b| *b == 0),
        "一番上の段の層は親が空"
    );
    bytes.drain(parent..parent + 16);
    if target <= 4 {
        assert_eq!(bytes[clipping], 0, "クリッピングの無い層");
        bytes.remove(clipping);
    }
    set_version(&mut bytes, target);
    bytes
}
