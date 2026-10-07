//! 書き出しの「lilToon の詰め方」: lilToon のテンプレートで書き出すとき、見た目の設定が lilToon のセットは、テクスチャのスロットに
//! 割り当てたチャンネルを、スロットごとの画像（`<名前>[_<セット名>]_<スロットの末尾>.png`）にして足す。値は 3D ビューの lilToon の
//! 再現が読む値と同じ:
//!
//! - ユーザーチャンネル: 何も描いていない所はチャンネルの既定に重ねた値（スカラーは値を RGB に・A は 1）。
//! - 標準のチャンネル: テンプレートの画像と同じ（Color は合成そのまま、Emission は値 × アルファの不透明、Normal は Normal の出力、
//!   Roughness・Metallic・Height は R × A）。
//! - 成分ごとの詰め合わせ: R・G・B・A をそれぞれの元から（0・1・チャンネルの成分）。リニアの画像。
//!
//! テンプレートの画像と同じものになるスロット（メインカラー = Color、ノーマルマップ = Normal、発光 = Emission）と、プロジェクトの画像を
//! 割り当てたスロット（マットキャップの絵）は書かない。どのレイヤーも使っていないチャンネルだけを読むスロットも書かない（テンプレートの
//! 画像と同じ決まり）。

use yolu_core::export::{uses, ExportError, ExportImage, ExportImageKind, ExportScalar};
use yolu_core::look::{LookKind, PlaneSource, TextureSource};
use yolu_core::{Channel, ChannelKind, ColorSpace, Document};

use super::liltoon::SLOTS;

/// テンプレートの画像と同じ入力か（書かない）。
fn covered_by_template(slot: &str, source: &TextureSource) -> bool {
    matches!(
        (slot, source),
        ("_MainTex", TextureSource::Channel(Channel::Color))
            | ("_BumpMap", TextureSource::Channel(Channel::Normal))
            | ("_EmissionMap", TextureSource::Channel(Channel::Emission))
    )
}

/// lilToon の詰め方で足す画像（スロットの名前と、書き出しの取り決め）。見た目の設定が lilToon でなければ空。
pub fn extra_images(doc: &Document) -> Vec<(&'static str, ExportImage)> {
    // 描く見た目（Unity から受けた値があればその上に利用者の設定）で決める: 3D ビューで見たとおりに詰める
    let look = doc.drawn_look();
    if look.kind != LookKind::LilToon {
        return Vec::new();
    }
    let mut out = Vec::new();
    for slot in SLOTS {
        let Some(source) = look.textures.get(slot.name) else {
            continue;
        };
        if covered_by_template(slot.name, source) || matches!(source, TextureSource::Image(_)) {
            continue;
        }
        let channels: Vec<Channel> = source
            .channels()
            .into_iter()
            .filter(|c| doc.channel_info(*c).is_some())
            .collect();
        if !channels.iter().any(|c| uses(doc, *c)) {
            continue;
        }
        let image = match source {
            TextureSource::Channel(c) => match doc.channel_info(*c) {
                Some(info) if info.kind == ChannelKind::Normal => {
                    ExportImage::of(slot.suffix, ExportImageKind::Normal)
                }
                Some(info)
                    if info.kind == ChannelKind::Color && info.color_space == ColorSpace::Srgb =>
                {
                    ExportImage::of(slot.suffix, ExportImageKind::BaseColor)
                }
                _ => linear(slot.suffix),
            },
            _ => linear(slot.suffix),
        };
        out.push((slot.name, image));
    }
    out
}

fn linear(suffix: &str) -> ExportImage {
    // 中身は `slot_image` が作る。取り決め（リニア・ノーマルマップでない）だけを持つ入れ物
    ExportImage::pack(
        suffix,
        ExportScalar::Zero,
        ExportScalar::Zero,
        ExportScalar::Zero,
        ExportScalar::One,
    )
}

/// チャンネルの、lilToon の再現が読む値（straight RGBA8、行は下から上、文書の大きさ）。
fn channel_values(
    doc: &Document,
    channel: Channel,
    max_working_bytes: u64,
) -> Result<Vec<u8>, ExportError> {
    let n = doc.width() as u64 * doc.height() as u64;
    if 4 * n > max_working_bytes {
        return Err(ExportError::WorkingBudgetExceeded {
            needed: 4 * n,
            allowed: max_working_bytes,
        });
    }
    let info = doc
        .channel_info(channel)
        .ok_or(ExportError::InvalidArgument("チャンネルが無い"))?
        .clone();
    if channel == Channel::Normal {
        return Ok(doc.normal_output(max_working_bytes)?);
    }
    let mut px = doc.composite_channel(channel, doc.bounds())?;
    if channel == Channel::Color {
        return Ok(px);
    }
    if channel.is_standard() {
        // Emission は値 × アルファの不透明、スカラーは R × A を RGB に
        for p in px.as_chunks_mut::<4>().0 {
            let a = p[3] as u32;
            let mul = |v: u8| ((v as u32 * a + 127) / 255) as u8;
            if channel == Channel::Emission {
                p[0] = mul(p[0]);
                p[1] = mul(p[1]);
                p[2] = mul(p[2]);
            } else {
                let v = mul(p[0]);
                p[0] = v;
                p[1] = v;
                p[2] = v;
            }
            p[3] = 255;
        }
        return Ok(px);
    }
    // ユーザーチャンネル: 乗算済みにして既定に重ねる（3D ビューの `fetch` と同じ式）
    let d = info.default.to_array();
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        for k in 0..4 {
            let premult = if k == 3 {
                a
            } else {
                (p[k] as u32 * a + 127) / 255
            };
            let v = premult + (d[k] as u32 * (255 - a) + 127) / 255;
            p[k] = v.min(255) as u8;
        }
        if info.kind == ChannelKind::Scalar {
            p[1] = p[0];
            p[2] = p[0];
            p[3] = 255;
        }
    }
    Ok(px)
}

/// スロットの画像を作る（`extra_images` に出たスロット）。作業のバイト数（`max_working_bytes`）は、確保の前に同時に持つ量で
/// 見積もって断る: チャンネル 1 つなら合成の 1 枚（4n）、成分ごとの詰め合わせなら出力と、読むチャンネルの合成を 1 つずつ（8n。
/// チャンネルごとに合成して出力へ書いたら捨てる）。
pub fn slot_image(
    doc: &Document,
    slot: &str,
    max_working_bytes: u64,
) -> Result<Vec<u8>, ExportError> {
    let source = doc
        .drawn_look()
        .textures
        .get(slot)
        .copied()
        .ok_or(ExportError::InvalidArgument("割り当ての無いスロット"))?;
    match source {
        TextureSource::Channel(c) => channel_values(doc, c, max_working_bytes),
        TextureSource::Packed(planes) => {
            let n = doc.width() as u64 * doc.height() as u64;
            let reads = planes.iter().any(
                |p| matches!(p, PlaneSource::Channel { channel, .. } if doc.channel_info(*channel).is_some()),
            );
            let needed = if reads { 8 * n } else { 4 * n };
            if needed > max_working_bytes {
                return Err(ExportError::WorkingBudgetExceeded {
                    needed,
                    allowed: max_working_bytes,
                });
            }
            let mut out = vec![0u8; 4 * n as usize];
            for (k, plane) in planes.iter().enumerate() {
                let fixed = match plane {
                    PlaneSource::Zero => Some(0u8),
                    PlaneSource::One => Some(255),
                    // 無いチャンネルは 3D ビューと同じく 1
                    PlaneSource::Channel { channel, .. }
                        if doc.channel_info(*channel).is_none() =>
                    {
                        Some(255)
                    }
                    PlaneSource::Channel { .. } => None,
                };
                if let Some(v) = fixed {
                    for p in out.as_chunks_mut::<4>().0 {
                        p[k] = v;
                    }
                }
            }
            // 同じチャンネルは 1 回だけ合成し、そのチャンネルを読む成分を全部書いてから捨てる（合成を同時に 2 つ持たない）
            let mut done: Vec<Channel> = Vec::new();
            for plane in planes.iter() {
                let PlaneSource::Channel { channel, .. } = plane else {
                    continue;
                };
                if done.contains(channel) || doc.channel_info(*channel).is_none() {
                    continue;
                }
                done.push(*channel);
                let values = channel_values(doc, *channel, max_working_bytes - 4 * n)?;
                for (k, p) in planes.iter().enumerate() {
                    if let PlaneSource::Channel {
                        channel: c,
                        component,
                    } = p
                    {
                        if c == channel {
                            let at = (*component).min(3) as usize;
                            for (o, s) in out
                                .as_chunks_mut::<4>()
                                .0
                                .iter_mut()
                                .zip(values.as_chunks::<4>().0)
                            {
                                o[k] = s[at];
                            }
                        }
                    }
                }
            }
            Ok(out)
        }
        TextureSource::Image(_) => {
            Err(ExportError::InvalidArgument("プロジェクトの画像のスロット"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::look::MaterialLook;
    use yolu_core::{ChannelInfo, Rgba8};

    #[test]
    fn slots_pack_user_channels_over_their_defaults_and_skip_the_template_images() {
        let mut doc = Document::new(4, 2).unwrap();
        let mask = doc
            .add_channel(ChannelInfo {
                name: "影".into(),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(200, 200, 200, 255),
            })
            .unwrap();
        let layer = doc.add_layer("a").unwrap();
        doc.set_channel_enabled(layer, mask, true).unwrap();
        doc.set_channel_pixel(layer, mask, 0, 0, Rgba8::new(10, 10, 10, 255))
            .unwrap();
        doc.set_channel_pixel(layer, mask, 1, 0, Rgba8::new(10, 10, 10, 128))
            .unwrap();
        let mut look = MaterialLook {
            kind: LookKind::LilToon,
            ..MaterialLook::default()
        };
        crate::look::default_textures(&mut look);
        look.textures
            .insert("_ShadowStrengthMask".into(), TextureSource::Channel(mask));
        look.textures.insert(
            "_ShadowBorderMask".into(),
            TextureSource::Packed([
                PlaneSource::Channel {
                    channel: mask,
                    component: 0,
                },
                PlaneSource::Zero,
                PlaneSource::One,
                PlaneSource::One,
            ]),
        );
        doc.set_look(look, false).unwrap();
        let extra = extra_images(&doc);
        let names: Vec<&str> = extra.iter().map(|(s, _)| *s).collect();
        assert_eq!(
            names,
            vec!["_ShadowStrengthMask", "_ShadowBorderMask"],
            "Main・Normal・Emission はテンプレートの画像"
        );
        assert!(!extra[0].1.srgb());
        let px = slot_image(&doc, "_ShadowStrengthMask", u64::MAX).unwrap();
        // 塗った所は値、半分の所は既定と半々、何も描いていない所は既定
        assert_eq!(&px[0..4], &[10, 10, 10, 255]);
        let half = (10 * 128 + 127) / 255 + (200 * 127 + 127) / 255;
        assert_eq!(px[4] as u32, half);
        assert_eq!(&px[8..12], &[200, 200, 200, 255]);
        let packed = slot_image(&doc, "_ShadowBorderMask", u64::MAX).unwrap();
        assert_eq!(&packed[0..4], &[10, 0, 255, 255]);
        assert_eq!(&packed[8..12], &[200, 0, 255, 255]);
        // 予算を超える作業は確保の前に断る（チャンネル 1 つは合成の 1 枚 = 4n、詰め合わせは出力と合成 1 つ = 8n。n = 8 画素）
        let over = |r: Result<Vec<u8>, ExportError>| {
            matches!(r, Err(ExportError::WorkingBudgetExceeded { .. }))
        };
        assert!(over(slot_image(&doc, "_ShadowStrengthMask", 31)));
        assert!(slot_image(&doc, "_ShadowStrengthMask", 32).is_ok());
        assert!(over(slot_image(&doc, "_ShadowBorderMask", 63)));
        assert_eq!(slot_image(&doc, "_ShadowBorderMask", 64).unwrap(), packed);
    }

    #[test]
    fn a_packed_slot_composites_one_channel_at_a_time_within_the_budget() {
        // 2 つのチャンネルを読む詰め合わせ（顔の影の SDF の形）も、同時に持つのは出力と合成 1 つ（8n）。読まない成分だけなら 4n
        let mut doc = Document::new(4, 4).unwrap();
        let mut masks = Vec::new();
        for (name, v) in [("左", 30u8), ("右", 220u8)] {
            let c = doc
                .add_channel(ChannelInfo {
                    name: name.into(),
                    kind: ChannelKind::Scalar,
                    color_space: ColorSpace::Linear,
                    default: Rgba8::new(255, 255, 255, 255),
                })
                .unwrap();
            let layer = doc.add_layer(name).unwrap();
            doc.set_channel_enabled(layer, c, true).unwrap();
            for y in 0..4 {
                for x in 0..4 {
                    doc.set_channel_pixel(layer, c, x, y, Rgba8::new(v, v, v, 255))
                        .unwrap();
                }
            }
            masks.push(c);
        }
        let mut look = MaterialLook {
            kind: LookKind::LilToon,
            ..MaterialLook::default()
        };
        look.textures.insert(
            "_ShadowStrengthMask".into(),
            TextureSource::Packed([
                PlaneSource::Channel {
                    channel: masks[0],
                    component: 0,
                },
                PlaneSource::Channel {
                    channel: masks[1],
                    component: 0,
                },
                PlaneSource::Channel {
                    channel: masks[0],
                    component: 0,
                },
                PlaneSource::One,
            ]),
        );
        look.textures.insert(
            "_ShadowBorderMask".into(),
            TextureSource::Packed([
                PlaneSource::One,
                PlaneSource::Zero,
                PlaneSource::One,
                PlaneSource::Zero,
            ]),
        );
        doc.set_look(look, false).unwrap();
        let n = 16u64;
        match slot_image(&doc, "_ShadowStrengthMask", 8 * n - 1) {
            Err(ExportError::WorkingBudgetExceeded { needed, allowed }) => {
                assert_eq!((needed, allowed), (8 * n, 8 * n - 1))
            }
            other => panic!("{other:?}"),
        }
        let px = slot_image(&doc, "_ShadowStrengthMask", 8 * n).unwrap();
        assert_eq!(&px[0..4], &[30, 220, 30, 255]);
        assert_eq!(&px[60..64], &[30, 220, 30, 255]);
        // 成分が 0・1 だけの詰め合わせは出力の 1 枚
        assert!(matches!(
            slot_image(&doc, "_ShadowBorderMask", 4 * n - 1),
            Err(ExportError::WorkingBudgetExceeded { .. })
        ));
        assert_eq!(
            &slot_image(&doc, "_ShadowBorderMask", 4 * n).unwrap()[0..4],
            &[255, 0, 255, 0]
        );
    }

    #[test]
    fn the_slots_added_for_the_new_features_are_packed_too() {
        // 描く口が作るチャンネル（リムシェードのマスク・メインカラー 2nd の色のレイヤー）も、書き出しの lilToon の詰め方に入る
        let mut doc = Document::new(4, 2).unwrap();
        crate::look::apply_new_set_look(&mut doc);
        let layer = doc.add_layer("a").unwrap();
        let rim =
            crate::look::paint_slot(&mut doc, "_RimShadeMask", crate::lang::Lang::Ja).unwrap();
        let decal =
            crate::look::paint_slot(&mut doc, "_Main2ndTex", crate::lang::Lang::Ja).unwrap();
        for c in [rim, decal] {
            doc.set_channel_enabled(layer, c, true).unwrap();
        }
        doc.set_channel_pixel(layer, decal, 0, 0, Rgba8::new(255, 0, 0, 255))
            .unwrap();
        let extra = extra_images(&doc);
        let names: Vec<(&str, &str)> = extra.iter().map(|(s, i)| (*s, i.suffix())).collect();
        assert_eq!(
            names,
            vec![
                ("_Main2ndTex", "Main2nd"),
                ("_RimShadeMask", "RimShadeMask")
            ]
        );
        assert!(extra[0].1.srgb(), "色のレイヤーは sRGB");
        assert!(!extra[1].1.srgb());
        // 色のレイヤーは何も描いていない所が透明
        let px = slot_image(&doc, "_Main2ndTex", u64::MAX).unwrap();
        assert_eq!(&px[0..4], &[255, 0, 0, 255]);
        assert_eq!(&px[4..8], &[0, 0, 0, 0]);
    }
}
