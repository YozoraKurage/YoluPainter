//! 一度に落とした画像を、名前の後ろの印（`crate::pbr`）で PBR のチャンネルに当て、同じ前の名前の組をアセットの「マテリアル」にする。
//!
//! - マテリアルは、塗りつぶしレイヤー 1 つを持つ .ylsmart の形のファイル（保存の形式は今のまま）。各チャンネルの画像は、同じプロジェクトの
//!   アセットの画像を ID で指す（画像を .ylsmart に埋め込む形は、今は置けない素材になるので使わない）。画像はアセットに普通の画像としても並ぶ。
//!   置くと、塗りつぶしレイヤーの各チャンネルにその画像が入る（`shelf::place_material`）。
//! - 色（Color・Emission）の浮動小数の画像は sRGB へ直して、データ（ノーマル・ラフネスなど）は線形のまま 8 ビットにする。データの画像は
//!   棚の色空間を linear にする。
//! - 標準のチャンネルが無い印（AO・Opacity）は、マテリアルに入れず、画像として足す。文書に無いチャンネルは使わない（足さない）。
//! - 画像の中身は名前が合っているかを見ない。画素の値の変換は、Glossiness の反転・DirectX のノーマルの緑の反転・ORM の分割だけ。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yolu_core::fill_image::Projection;
use yolu_core::{Channel, Document, ImageId, Rgba8};

use crate::fillfx::inputs::image_id;
use crate::imagefile::{self, Transfer};
use crate::lang::{Count, Lang};
use crate::notice::Source;
use crate::pbr::{self, Kind, Member, Role};
use crate::state::AppState;

/// 組の中で、あるチャンネルに使う画像（同じチャンネルが 2 つあれば大きい方）。
struct Pick {
    /// 落とした画像の並びでの、元のファイルの番号（ORM から分けた物は同じ番号）。
    file: usize,
    resource: String,
    width: u32,
    height: u32,
    /// 平均の色（マテリアルの値の既定。画像が使えない所・サムネイルに出る）。
    mean: Rgba8,
}

/// 1 つの組を取り込んだ結果。
#[derive(Default)]
struct Outcome {
    /// マテリアルに使う画像（チャンネルごと）。
    picks: BTreeMap<Channel, Pick>,
    /// アセットへ足した画像のうち、マテリアルに入らなかった枚数（印はあったが使えなかった物・ORM から分けた AO・小さくて替えられた物）。
    unused: usize,
}

/// 取り込みの全体の数え上げ（知らせの文のために）。
#[derive(Default)]
struct Tally {
    /// 足したマテリアルの名前と、使ったファイルの数（ORM は 1 枚）。
    materials: Vec<(String, usize)>,
    /// マテリアルに入らず画像のまま足した枚数（ORM から分けた AO も数える）。
    images: usize,
}

fn stem_of(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image")
        .to_owned()
}

/// 画素（straight RGBA8）の平均の色（不透明）。
fn mean_color(rgba: &[u8]) -> Rgba8 {
    let mut sums = [0u64; 3];
    let mut count = 0u64;
    for px in rgba.as_chunks::<4>().0.iter() {
        for (sum, v) in sums.iter_mut().zip(px) {
            *sum += u64::from(*v);
        }
        count += 1;
    }
    let mean = |i: usize| (sums[i] / count.max(1)) as u8;
    Rgba8::new(mean(0), mean(1), mean(2), 255)
}

/// 組の名前をマテリアルの名前にする（空なら「マテリアル」、長すぎれば切る）。
fn material_name(lang: Lang, base: &str) -> String {
    if base.is_empty() {
        lang.t("shelf-material").to_owned()
    } else {
        base.chars().take(120).collect()
    }
}

impl AppState {
    /// 2 枚以上の画像を一度に落とした（か選んだ）ときの取り込み: 名前の印で組にできた物はマテリアル、残りは今までどおり画像としてアセットに足す。
    pub(super) fn import_image_set(&mut self, paths: &[PathBuf]) {
        if self.shelf_refuse_while_saving() {
            return;
        }
        let stems: Vec<String> = paths.iter().map(|p| stem_of(p)).collect();
        let refs: Vec<&str> = stems.iter().map(String::as_str).collect();
        let plan = pbr::plan(&refs);
        if plan.groups.is_empty() {
            for path in paths {
                self.import_image_file(path);
            }
            return;
        }
        let mut tally = Tally::default();
        for &i in &plan.plain {
            tally.images += usize::from(self.import_plain(&paths[i]));
        }
        for group in &plan.groups {
            let name = material_name(self.lang, &group.name);
            let outcome = self.import_group(&name, paths, &group.members);
            tally.images += outcome.unused;
            self.add_group_material(&name, outcome, &mut tally);
        }
        self.tell_import_set(&tally);
    }

    /// 印の無い画像を、今までどおり色の画像として足す（知らせは出さず、足せたかを返す。足せなければ理由を出す）。
    fn import_plain(&mut self, path: &Path) -> bool {
        let Some((rgba, w, h)) = self.read_set_image(path, Transfer::Srgb) else {
            return false;
        };
        self.add_set_image(&stem_of(path), rgba, w, h, "srgb")
            .is_some()
    }

    /// 画像を読む（拡張子が読めない形式・読めないファイルは理由を出して None）。
    fn read_set_image(&mut self, path: &Path, transfer: Transfer) -> Option<(Vec<u8>, u32, u32)> {
        let lang = self.lang;
        let name = stem_of(path);
        let Some(format) = imagefile::format_of(path) else {
            self.fail(
                Source::Assets,
                lang.with_reason(
                    super::cannot_add_image(lang, &name),
                    lang.t("dropin-unsupported-format"),
                ),
            );
            return None;
        };
        match imagefile::read_file_as(path, format, transfer)
            .map_err(|e| lang.image_read_error(format, &e))
        {
            Ok(read) => Some(read),
            Err(why) => {
                self.fail(
                    Source::Assets,
                    lang.with_reason(super::cannot_add_image(lang, &name), why),
                );
                None
            }
        }
    }

    /// 読んだ画素（上の行が先）を、文書と同じ向き（下の行が先）にしてアセットへ足す。足せなければ理由を出して None（棚の ID を返す）。
    fn add_set_image(
        &mut self,
        name: &str,
        mut rgba: Vec<u8>,
        w: u32,
        h: u32,
        space: &str,
    ) -> Option<String> {
        let lang = self.lang;
        super::flip_rows(&mut rgba, w as usize * 4);
        let before = self.shelf.resources().len();
        match self.shelf.add_image_in(lang, name, &rgba, w, h, space) {
            Ok(id) => {
                if self.shelf.resources().len() > before {
                    self.session.mark_modified();
                }
                Some(id)
            }
            Err(why) => {
                self.fail(
                    Source::Assets,
                    lang.with_reason(super::cannot_add_image(lang, name), why),
                );
                None
            }
        }
    }

    /// 組の画像を 1 枚ずつ読み、アセットへ足して、チャンネルごとに使う画像を決める。
    fn import_group(&mut self, base: &str, paths: &[PathBuf], members: &[Member]) -> Outcome {
        let mut outcome = Outcome::default();
        for member in members {
            let path = &paths[member.index];
            let transfer = if member.kind.is_color() {
                Transfer::Srgb
            } else {
                Transfer::Linear
            };
            let Some((mut rgba, w, h)) = self.read_set_image(path, transfer) else {
                continue;
            };
            let name = stem_of(path);
            match member.kind {
                Kind::Single {
                    role,
                    invert,
                    flip_green,
                } => {
                    if invert {
                        pbr::invert(&mut rgba);
                    }
                    if flip_green {
                        pbr::flip_green(&mut rgba);
                    }
                    self.add_piece(&mut outcome, member.index, role, &name, rgba, (w, h));
                }
                // 成分を 1 つずつ作り、足してから次を作る（元の画像と 3 枚を同時に持たない）
                Kind::Packed => {
                    for (part, role) in pbr::PACKED.into_iter().enumerate() {
                        let name = if base.is_empty() {
                            role.label().to_owned()
                        } else {
                            format!("{base}_{}", role.label())
                        };
                        let px = pbr::packed_part(&rgba, part);
                        self.add_piece(&mut outcome, member.index, role, &name, px, (w, h));
                    }
                }
            }
        }
        outcome
    }

    /// 画像 1 枚をアセットへ足し、チャンネルの候補にする。候補にならなければ（足せなかった物は除く）画像のままの枚数に数える。
    fn add_piece(
        &mut self,
        outcome: &mut Outcome,
        file: usize,
        role: Role,
        name: &str,
        px: Vec<u8>,
        (w, h): (u32, u32),
    ) {
        let mean = mean_color(&px);
        let space = if role.is_color() { "srgb" } else { "linear" };
        let Some(resource) = self.add_set_image(name, px, w, h, space) else {
            return;
        };
        let pick = Pick {
            file,
            resource,
            width: w,
            height: h,
            mean,
        };
        if !self.pick_for(outcome, role, pick) {
            outcome.unused += 1;
        }
    }

    /// 画像をチャンネルの候補にする。標準のチャンネルが無い印・文書に無いチャンネルは使わない（false）。同じチャンネルが 2 つあれば大きい方を残す。
    fn pick_for(&mut self, outcome: &mut Outcome, role: Role, pick: Pick) -> bool {
        let Some(channel) = role
            .channel()
            .filter(|c| self.session.doc.channel_info(*c).is_some())
        else {
            return false;
        };
        let pixels = |p: &Pick| u64::from(p.width) * u64::from(p.height);
        match outcome.picks.get(&channel) {
            Some(old) if pixels(old) >= pixels(&pick) => false,
            Some(_) => {
                // 大きい方に替わる。前の画像はアセットに残る（画像のまま）
                outcome.picks.insert(channel, pick);
                outcome.unused += 1;
                true
            }
            None => {
                outcome.picks.insert(channel, pick);
                true
            }
        }
    }

    /// 組のマテリアルをアセットへ足す。使えるチャンネルが 2 つに満たなければマテリアルにせず、その画像は画像のままにする。
    fn add_group_material(&mut self, name: &str, outcome: Outcome, tally: &mut Tally) {
        let lang = self.lang;
        let count = outcome.picks.len();
        if count < 2 {
            tally.images += count;
            return;
        }
        let mut files: Vec<usize> = outcome.picks.values().map(|p| p.file).collect();
        files.sort_unstable();
        files.dedup();
        let added = material_bytes(name, &outcome.picks)
            .and_then(|bytes| self.shelf.add_material(lang, name, &bytes));
        match added {
            Ok(id) => {
                self.shelf.selected = Some(id);
                self.session.mark_modified();
                tally.materials.push((name.to_owned(), files.len()));
            }
            Err(why) => {
                self.fail(
                    Source::Assets,
                    lang.with_reason(super::cannot_add_image(lang, name), why),
                );
                tally.images += count;
            }
        }
    }

    /// 取り込みの知らせ（足したマテリアルと、画像のまま足した枚数）を 1 つ出す。
    fn tell_import_set(&mut self, tally: &Tally) {
        let lang = self.lang;
        let mut text = match tally.materials.as_slice() {
            [] => String::new(),
            [(name, n)] => lang.f(
                "fillfx-pbr-material-added",
                &[("name", &lang.quote(name)), ("n", &Count(*n))],
            ),
            many => lang.f(
                "fillfx-pbr-materials-added",
                &[
                    ("count", &Count(many.len())),
                    ("n", &Count(many.iter().map(|m| m.1).sum())),
                ],
            ),
        };
        if tally.images > 0 {
            let rest = lang.f("fillfx-pbr-images-added", &[("n", &Count(tally.images))]);
            text = if text.is_empty() {
                rest
            } else {
                format!("{text} · {rest}")
            };
        }
        if !text.is_empty() {
            self.info(Source::Assets, text);
        }
    }
}

impl AppState {
    /// 素材のレイヤーが指す画像（塗りつぶしのチャンネルの画像）を、置く前に文書の効果の入力へ頼む。アセットに無い画像があれば置けない
    /// （理由を出して None）。頼んだ画像の ID を返す（置けなかったときは `release_material_images` で手放す）。
    pub(crate) fn claim_material_images(
        &mut self,
        name: &str,
        material: &yolu_core::smart::SmartMaterial,
    ) -> Option<Vec<ImageId>> {
        let mut wanted: Vec<ImageId> = material
            .layers()
            .iter()
            .flat_map(|l| l.fill_images().map(|(_, id)| id))
            .collect();
        wanted.sort_unstable_by_key(|id| id.0);
        wanted.dedup();
        let mut held = Vec::new();
        for id in wanted {
            match self.use_shelf_image(&crate::fx::inputs::resource_id(id)) {
                Ok(_) => held.push(id),
                Err(why) => {
                    self.release_material_images(&held);
                    self.cannot(
                        crate::notice::Kind::Refusal,
                        Source::Assets,
                        crate::shelf::Attempt::Place,
                        Some(name),
                        &why,
                    );
                    return None;
                }
            }
        }
        Some(held)
    }

    /// `claim_material_images` で頼んだ画像を手放す（置けなかったとき。ほかのレイヤーが指している画像は残る）。
    pub(crate) fn release_material_images(&mut self, held: &[ImageId]) {
        for id in held {
            self.release_shelf_image(*id);
        }
    }
}

/// チャンネルごとの画像を指す塗りつぶしレイヤー 1 つを、マテリアルのファイルのバイト列にする。値の既定は画像の平均の色（画像が使えない所に出る）。
fn material_bytes(name: &str, picks: &BTreeMap<Channel, Pick>) -> Result<Vec<u8>, String> {
    let fail = |e: &dyn std::fmt::Display| e.to_string();
    let (w, h) = picks
        .values()
        .map(|p| (p.width, p.height))
        .max_by_key(|(w, h)| u64::from(*w) * u64::from(*h))
        .expect("画像が 1 つ以上");
    let mut doc = Document::new(w, h).map_err(|e| fail(&e))?;
    let values: Vec<(Channel, Rgba8)> = picks.iter().map(|(c, p)| (*c, p.mean)).collect();
    let layer = doc
        .add_fill_layer(name, &values, None)
        .map_err(|e| fail(&e))?;
    let images: Vec<(Channel, ImageId)> = picks
        .iter()
        .map(|(c, p)| {
            image_id(&p.resource)
                .map(|id| (*c, id))
                .ok_or_else(|| p.resource.clone())
        })
        .collect::<Result<_, _>>()?;
    doc.set_fill_images_for_load(layer, &images, Projection::default())
        .map_err(|e| fail(&e))?;
    let material = doc.capture_material(layer, name).map_err(|e| fail(&e))?;
    crate::shelf::material_file_bytes(&material).map_err(|e| fail(&e))
}
