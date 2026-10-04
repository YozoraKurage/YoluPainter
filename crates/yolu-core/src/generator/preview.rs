use super::{
    anchor, evaluate, Blend, BoundGenerator, Error, Options, ProceduralSpace, Settings, Source,
    Target,
};
use crate::{Rect, Rgba8};

/// 全面が不透明の白の入力。画素を持たない（入力の白い画像を確保しないので、大きな引数でも使うメモリは出力の分だけで、
/// 予算の検査が確保より前に効く）。
struct White {
    width: u32,
    height: u32,
}
impl Source for White {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn pixel(&self, _x: u32, _y: u32) -> Rgba8 {
        Rgba8::new(255, 255, 255, 255)
    }
}

/// ノイズ・グランジの見本の画像（straight RGBA8 の灰色、左下原点）。UV 空間（周期つき）で、マップ・文書を使わずに評価する
/// （画面のサムネイル用。同じ設定ならスレッド数に依らず同じバイト）。レベル・にじみ・大きさ・シードが効き、回転は UV では効かない。
/// 出力は `Options::default()` の予算（`budget_bytes`）までで、超える大きさは何も確保せずに `Error::Budget` で断る。
pub fn preview(settings: &Settings, width: u32, height: u32) -> Result<Vec<u8>, Error> {
    if !settings.kind.is_procedural() {
        return Err(Error::Invalid("見本はノイズ・グランジだけです"));
    }
    let mut s = settings.clone();
    s.procedural.space = ProceduralSpace::Uv;
    s.blend = Blend::Replace;
    s.pins.clear();
    let bound = BoundGenerator::bind(
        &s,
        &[],
        None,
        (width, height),
        Err(anchor::Issue::NotChosen),
    )?;
    let source = White { width, height };
    let out = evaluate(
        &source,
        &bound,
        Rect::new(0, 0, width, height),
        Target::Color,
        1.,
        &Options::default(),
    )?;
    Ok(out.pixels)
}
