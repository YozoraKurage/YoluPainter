//! 設定の値の範囲。値の検査（各 `validate` とコンストラクタ）と、効果の種類の表（[`crate::effects::catalog`]）が同じ値を使うので、
//! 範囲を変えるときはここだけを変える（表が検査と食い違わない）。
//!
//! 範囲の外は断る（切り詰めない）。ここに無い条件（レベル補正の入力の幅が 1/255 以上、方向が零ベクトルでない、
//! 種類ごとに使える欄など）は各 `validate` が持つ。

use std::ops::RangeInclusive;

/// ぼかしの半径（画素）。
pub const BLUR_RADIUS: RangeInclusive<u32> = 1..=256;
/// シャープの半径（画素）。
pub const SHARPEN_RADIUS: RangeInclusive<u32> = 1..=64;
/// シャープの量。
pub const SHARPEN_AMOUNT: RangeInclusive<f64> = 0.0..=5.0;
/// シャープのしきい値（これ以下の差には効かせない）。
pub const SHARPEN_THRESHOLD: RangeInclusive<u32> = 0..=255;
/// ノイズの量。
pub const NOISE_AMOUNT: RangeInclusive<f64> = 0.0..=1.0;
/// レベル補正の入力・出力の黒と白（0〜1）。入力の幅は 1/255 以上（検査が見る）。
pub const LEVELS_UNIT: RangeInclusive<f64> = 0.0..=1.0;
/// レベル補正のガンマ。
pub const GAMMA: RangeInclusive<f64> = 0.1..=9.99;
/// 色相（度）。
pub const HUE: RangeInclusive<f64> = -180.0..=180.0;
/// 彩度・明度（−1〜1）。
pub const SATURATION: RangeInclusive<f64> = -1.0..=1.0;
/// 2 値化のしきい値。
pub const THRESHOLD_LEVEL: RangeInclusive<u32> = 1..=255;
/// ポスタリゼーションの階調。
pub const POSTERIZE_LEVELS: RangeInclusive<u32> = 2..=255;

/// Generator の 0〜1 の欄（低・高・減衰・ノイズの量・割合）。
pub const UNIT: RangeInclusive<f64> = 0.0..=1.0;
/// Generator に重ねるノイズの大きさ。
pub const NOISE_SCALE: RangeInclusive<f64> = 0.001..=1.0;
/// 手続き型の Generator の回転（度。軸ごと）。
pub const PROCEDURAL_ROTATION: RangeInclusive<f64> = -360.0..=360.0;
/// 手続き型の Generator のラクナリティ。
pub const LACUNARITY: RangeInclusive<f64> = 1.0..=4.0;
/// Generator の方向の成分の大きさの上限（これを超えるものは断る）。
pub const DIRECTION_COMPONENT: RangeInclusive<f64> = -1.0e6..=1.0e6;
