//! 効果の種類と欄の名前・説明（日本語と英語）。範囲・型・既定は core の表（`yolu_core::effects::catalog`）にあり、ここは文だけを持つ。
//! 表の全部の種類・欄に文があることは試験が確かめる（種類を足したら、ここに足すまで試験が落ちる）。

use crate::text::Text;

/// 種類の名前と説明。
pub fn kind_text(id: &str) -> Option<(Text, Text)> {
    let (title_ja, title_en, ja, en) = match id {
        "blur" => ("ぼかし（ガウス）", "Gaussian Blur", "半径の分だけ画素をなだらかに混ぜる。", "Smooths pixels over the given radius."),
        "sharpen" => ("シャープ", "Sharpen", "輪郭を強める。", "Strengthens edges."),
        "noise" => ("ノイズ", "Noise", "ざらつきを足す。", "Adds grain."),
        "levels" => ("レベル補正", "Levels", "入力と出力の黒・白とガンマで階調を直す。", "Remaps tones with input and output black/white points and gamma."),
        "invert" => ("階調の反転", "Invert", "明るさを反転する。", "Inverts the values."),
        "normalize" => ("正規化（レイヤー全体）", "Normalize (Layer)", "レイヤー全体の最小と最大が 0 と 1 になるよう広げる。", "Stretches the whole layer so its minimum and maximum become 0 and 1."),
        "hue_saturation" => ("色相・彩度・明度", "Hue / Saturation", "色相・彩度・明度を直す（色のチャンネルだけ）。", "Shifts hue, saturation and lightness (color channels only)."),
        "gradient_map" => ("グラデーションマップ", "Gradient Map", "輝度をグラデーションの色に置き換える。ランプは値の欄では作れず変えられない。", "Replaces luminance with a gradient. The ramp cannot be created or changed by values."),
        "tone_curve" => ("トーンカーブ", "Tone Curve", "曲線で階調を直す。曲線は値の欄では作れず変えられない。", "Remaps tones with curves. The curves cannot be created or changed by values."),
        "color_balance" => ("カラーバランス", "Color Balance", "シャドウ・中間・ハイライトごとに色を寄せる（色のチャンネルだけ）。", "Shifts colors separately in shadows, midtones and highlights (color channels only)."),
        "brightness_contrast" => ("明るさ・コントラスト", "Brightness / Contrast", "明るさとコントラストを直す。", "Adjusts brightness and contrast."),
        "threshold" => ("2 値化", "Threshold", "しきい値より明るければ白、そうでなければ黒にする。", "Turns values at or above the level white and the rest black."),
        "posterize" => ("ポスタリゼーション", "Posterize", "階調の数を減らす。", "Reduces the number of tone levels."),
        "edge_wear" => ("エッジの摩耗", "Edge Wear", "焼いた曲率のマップから、角の摩耗の値を作る（Generator）。", "A generator making edge wear from the baked curvature map."),
        "dirt" => ("汚れ・隙間", "Dirt", "焼いた AO・曲率のマップから、汚れの値を作る（Generator）。", "A generator making dirt from the baked ambient occlusion and curvature maps."),
        "position_gradient" => ("位置の勾配", "Position Gradient", "焼いた位置のマップから、軸に沿った勾配を作る（Generator）。", "A generator making a gradient along an axis from the baked position map."),
        "thickness" => ("厚み", "Thickness", "焼いた厚みのマップから値を作る（Generator）。", "A generator making values from the baked thickness map."),
        "direction" => ("向き", "Direction", "焼いた法線のマップから、向きに沿った値を作る（Generator）。", "A generator making values from the baked normal map along a direction."),
        "procedural_noise" => ("ノイズ（手続き型）", "Procedural Noise", "位置・UV から値・Perlin・Worley のノイズを作る（Generator）。位置のマップが無ければ UV で評価する。", "A generator making value, Perlin or Worley noise from position or UV. Without a position map it is evaluated in UV space."),
        "grunge" => ("グランジ", "Grunge", "汚れ・錆・傷などのプリセットの模様を作る（Generator）。位置のマップが無ければ UV で評価する。", "A generator making preset patterns such as stains, rust or scratches. Without a position map it is evaluated in UV space."),
        "shape_gradient" => ("形のグラデーション", "Shape Gradient", "形（ボリューム）とランプの勾配（Generator）。形とランプは値の欄では作れず変えられない。", "A generator with a volume and a ramp. The volume and ramp cannot be created or changed by values."),
        "id_color" => ("ID の色", "ID Color", "ID マップの選んだ色の所を取り出す（Generator）。色の一覧は値の欄では作れず変えられない。", "A generator selecting colors of the ID map. The color list cannot be created or changed by values."),
        "anchor" => ("アンカー", "Anchor", "下の層の結果を読む（Generator）。参照は値の欄では作れず変えられない。", "A generator reading the result of a layer below. The reference cannot be created or changed by values."),
        "image" => ("画像", "Image", "アセットの画像を塗りつぶしの層と同じ投影で読む（Generator）。色のチャンネルは画素の色、マスク・スカラーは選んだ成分。画像の参照と投影は値の欄では作れず変えられない。", "A generator reading a project image with the same projections as a fill layer: the pixel's colour on colour channels, the chosen component on masks and scalar channels. The image reference and projection cannot be created or changed by values."),
        _ => return None,
    };
    Some((Text::new(title_ja, title_en), Text::new(ja, en)))
}

/// 欄の説明。種類によって意味が違う欄（`amount`）は種類で分ける。
pub fn param_text(kind: &str, name: &str) -> Option<Text> {
    let (ja, en) = match (kind, name) {
        ("sharpen", "amount") => ("強さ。", "Strength."),
        ("noise", "amount") => ("ノイズの量。", "Amount of noise."),
        (_, "radius") => ("画素の半径。", "Radius in pixels."),
        (_, "threshold") => ("これ以下の差には効かせない（0〜255）。", "Differences below this are left alone (0-255)."),
        (_, "seed") => ("乱数の種。同じ値なら同じ模様。", "Random seed. The same seed gives the same pattern."),
        (_, "monochrome") => ("真ならモノクロのノイズ。", "True for monochrome noise."),
        (_, "input_black") => ("入力の黒（0〜1）。", "Input black point (0-1)."),
        (_, "input_white") => ("入力の白（0〜1。黒より 1/255 以上大きい）。", "Input white point (0-1, at least 1/255 above the black point)."),
        (_, "gamma") => ("ガンマ。1 で変えない。", "Gamma. 1 leaves tones as they are."),
        (_, "output_black") => ("出力の黒（0〜1）。", "Output black point (0-1)."),
        (_, "output_white") => ("出力の白（0〜1）。", "Output white point (0-1)."),
        (_, "hue") => ("色相（度）。", "Hue shift in degrees."),
        (_, "saturation") => ("彩度（−1〜1）。", "Saturation (-1 to 1)."),
        (_, "lightness") => ("明度（−1〜1）。", "Lightness (-1 to 1)."),
        (_, "preserve_luminosity") => ("真なら輝度を保つ。", "True keeps the luminosity."),
        (_, "brightness") => ("明るさ。", "Brightness."),
        (_, "contrast") => ("コントラスト。", "Contrast."),
        (_, "level") => ("しきい値（1〜255）。", "Threshold level (1-255)."),
        (_, "levels") => ("階調の数。", "Number of tone levels."),
        (_, "low") => ("値が 0 になる下の端（0〜1）。", "Lower end of the range that maps to 0 (0-1)."),
        (_, "high") => ("値が 1 になる上の端（0〜1。low より 0.001 以上大きい）。", "Upper end of the range that maps to 1 (0-1, at least 0.001 above low)."),
        (_, "softness") => ("境目のなだらかさ（0〜1）。", "Softness of the transition (0-1)."),
        (_, "invert") => ("真なら結果を反転する。", "True inverts the result."),
        (_, "blend") => ("下の値との混ぜ方。", "How the result is combined with the value below."),
        (_, "noise_amount") => ("重ねるノイズの量（0〜1）。", "Amount of noise mixed in (0-1)."),
        (_, "noise_scale") => ("重ねるノイズの大きさ。", "Size of the mixed-in noise."),
        (_, "noise_seed") => ("重ねるノイズの種。", "Seed of the mixed-in noise."),
        (_, "noise_space") => ("重ねるノイズを評価する空間（model か uv）。", "Space the mixed-in noise is evaluated in (model or uv)."),
        (_, "balance") => ("AO と曲線の割合（0 は AO だけ、1 は曲線だけ）。", "Balance between ambient occlusion and curvature (0 is AO only, 1 is curvature only)."),
        (_, "axis") => ("勾配の軸。", "Axis of the gradient."),
        (_, "direction_x") => ("向きの X 成分。", "X component of the direction."),
        (_, "direction_y") => ("向きの Y 成分。", "Y component of the direction."),
        (_, "direction_z") => ("向きの Z 成分。", "Z component of the direction."),
        (_, "use_bent_normal") => ("真なら焼いたベントノーマルを読む。", "True reads the baked bent normal."),
        (_, "space") => ("評価する空間。", "Space the pattern is evaluated in."),
        (_, "scale") => ("模様の大きさ。", "Size of the pattern."),
        (_, "rotation_x") => ("X 軸の回転（度）。", "Rotation around X in degrees."),
        (_, "rotation_y") => ("Y 軸の回転（度）。", "Rotation around Y in degrees."),
        (_, "rotation_z") => ("Z 軸の回転（度）。", "Rotation around Z in degrees."),
        (_, "bleed") => ("にじみ（0〜1）。", "Bleed that distorts the pattern edges (0-1)."),
        (_, "blend_width") => ("トライプラナーの境目の幅（0〜1）。", "Width of the triplanar blend (0-1)."),
        (_, "basis") => ("ノイズの基底。", "Noise basis."),
        (_, "cell_output") => ("Worley が返す量（基底が worley のときだけ）。", "What Worley noise returns (only with the worley basis)."),
        (_, "fractal") => ("オクターブの重ね方。", "How octaves are combined."),
        (_, "octaves") => ("重ねる数。", "Number of octaves."),
        (_, "lacunarity") => ("次のオクターブの周波数の倍率。", "Frequency multiplier of the next octave."),
        (_, "gain") => ("次のオクターブの振幅の倍率（0〜1）。", "Amplitude multiplier of the next octave (0-1)."),
        (_, "preset") => (
            "グランジのプリセット。変えると模様の大きさとレベルがプリセットの既定になる（同時に渡した値が先）。",
            "Grunge preset. Changing it resets the pattern size and levels to the preset's defaults (values given in the same call win).",
        ),
        (_, n) if n.contains("cyan_red") => ("−100 でシアン、+100 でレッドへ寄せる。", "-100 shifts toward cyan, +100 toward red."),
        (_, n) if n.contains("magenta_green") => ("−100 でマゼンタ、+100 でグリーンへ寄せる。", "-100 shifts toward magenta, +100 toward green."),
        (_, n) if n.contains("yellow_blue") => ("−100 でイエロー、+100 でブルーへ寄せる。", "-100 shifts toward yellow, +100 toward blue."),
        _ => return None,
    };
    Some(Text::new(ja, en))
}
