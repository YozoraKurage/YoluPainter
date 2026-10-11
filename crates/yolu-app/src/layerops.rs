//! レイヤーの操作の画面: 複数選択・結合・ロック・変形（移動・90° 回転・反転）。計算・検証・履歴は core（`document/merge.rs`・`locks.rs`・
//! `transform.rs`）に任せ、ここは「どのレイヤーに当てるか」「断られた理由を画面の言語で言う」「見た目が変わる結合を確かめる」だけを持つ。
//!
//! 複数選択（Unity 版の `LayerSelection` と同じ）: 描く先（`AppState::selected_layer`）は 1 つのままで、複数選択はそれを含む集合。
//! `selected_layer` をほかの所（新しいレイヤー・結合の結果・テクスチャセットの切り替え・取り消しでレイヤーが消えた）が変えると、集合は
//! 描く先の 1 つに戻る（集合は「描く先」が同じあいだだけ有効）。選択は文書にも .ylp にも入れない。

use std::collections::HashSet;

use egui::Vec2;
use yolu_core::{Affine2D, LayerLocks, LayerMergeReport, Resampling};

use crate::engine::{Channel, CoreError, LayerId, LayerKind};
use crate::jobs::JobSpec;
use crate::lang::Lang;
use crate::m2;
use crate::notice::Source;
use crate::state::AppState;

/// 複数選択の集合と、次の Shift クリックの起点。
#[derive(Clone, Debug, Default)]
pub struct LayerSelection {
    ids: HashSet<LayerId>,
    /// 集合を作ったときの描く先。`selected_layer` と違えば集合は無効。
    active: Option<LayerId>,
    anchor: Option<LayerId>,
}

/// 結合の種類（見た目が変わる結合を確かめたあと、同じ結合を許容差なしで行うために覚える）。
#[derive(Clone, Debug, PartialEq)]
pub enum MergeOp {
    Down(LayerId),
    Layers(Vec<LayerId>),
    Group(LayerId),
    Visible,
}

/// 見た目が丸めの許容差を超えて変わる結合の確かめ（ウィンドウを出して、結合するかを聞く）。
#[derive(Clone, Debug, PartialEq)]
pub struct MergeConfirm {
    pub op: MergeOp,
    /// 変わるチャンネルごとの画素数（番号の順）。
    pub channels: Vec<(Channel, u64)>,
}

/// レイヤーの統合の確かめ（キーの割り当てを止める）。
pub(crate) const JOB: JobSpec = JobSpec {
    modal: Some(|app| app.layer_ops.merge_confirm.is_some()),
    ..JobSpec::new("layers.merge", crate::jobs::never)
};

/// 画面の状態。
#[derive(Debug, Default)]
pub struct LayerOpsState {
    pub selection: LayerSelection,
    pub merge_confirm: Option<MergeConfirm>,
    pub confirm_offset: Vec2,
}

/// レイヤーの変形（動かす対象は選んでいるレイヤー。グループなら中身のラスターレイヤーごと。選択範囲があればその中身と選択範囲）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Xform {
    /// 90° 回転・左右反転・上下反転。動かすものの範囲の中心を軸にし、90° では軸を画素の格子に合わせて画素をそのまま写す。
    Rotate90 {
        clockwise: bool,
    },
    Flip {
        horizontal: bool,
    },
    /// 整数画素の移動（矢印キー・ドラッグの移動）。画素をそのまま写す。
    Move {
        dx: i32,
        dy: i32,
    },
    /// 数値の変形: 動かすものの中心を軸に、拡大縮小（負は反転）・回転（度、反時計回り）してから (dx, dy) ずらす。
    Numeric {
        dx: f64,
        dy: f64,
        degrees: f64,
        sx: f64,
        sy: f64,
    },
    /// ハンドルのドラッグで決めた変形（キャンバスの座標）。
    Affine(Affine2D),
}

/// ロックの 4 種（表示の順）。
pub const LOCK_FLAGS: [LayerLocks; 4] = [
    LayerLocks::TRANSPARENCY,
    LayerLocks::PIXELS,
    LayerLocks::POSITION,
    LayerLocks::ALL,
];

pub fn lock_name(lang: Lang, flag: LayerLocks) -> &'static str {
    if flag == LayerLocks::TRANSPARENCY {
        lang.pick("透明部分", "Transparent pixels")
    } else if flag == LayerLocks::PIXELS {
        lang.pick("画素", "Image pixels")
    } else if flag == LayerLocks::POSITION {
        lang.pick("位置", "Position")
    } else {
        lang.pick("すべて", "All")
    }
}

/// メニューの項目名（見出しが無いので、何をするかを言い切る「〜をロック」。プロパティの「ロック」の行などは、種類の名前だけの `lock_name`）。
pub fn menu_lock_name(lang: Lang, flag: LayerLocks) -> &'static str {
    if flag == LayerLocks::TRANSPARENCY {
        lang.pick("透明部分をロック", "Lock Transparent Pixels")
    } else if flag == LayerLocks::PIXELS {
        lang.pick("画素をロック", "Lock Image Pixels")
    } else if flag == LayerLocks::POSITION {
        lang.pick("位置をロック", "Lock Position")
    } else {
        lang.pick("すべてをロック", "Lock All")
    }
}

/// 効いているロックの個別の種類の名前（すべてを除く。すべては個別の 3 種を含んで効く）。
pub fn lock_names(lang: Lang, locks: LayerLocks) -> Vec<&'static str> {
    LOCK_FLAGS[..3]
        .iter()
        .filter(|f| locks.contains(**f))
        .map(|f| lock_name(lang, *f))
        .collect()
}

/// 結合が結果へ足したこと（C# の `MergeNotes`）。何も無ければ空。
pub fn merge_notes_text(lang: Lang, notes: u8) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if notes & 1 != 0 {
        parts.push(lang.pick(
            "フィルター・ジェネレーターを画素に焼いた",
            "Filters and generators baked into pixels",
        ));
    }
    if notes & 2 != 0 {
        parts.push(lang.pick("パスは画素になった", "Paths became pixels"));
    }
    if notes & 4 != 0 {
        parts.push(lang.pick("隠した子は除いた", "Hidden children dropped"));
    }
    if notes & 8 != 0 {
        parts.push(lang.pick(
            "無効のチャンネルの画素は除いた",
            "Pixels of disabled channels dropped",
        ));
    }
    if notes & 16 != 0 {
        parts.push(lang.pick("テキストは画素になった", "Text became pixels"));
    }
    parts.join(lang.pick("・", "; "))
}

impl AppState {
    // ───────── 複数選択 ─────────

    /// 選んでいるレイヤー（描く先を含む）。下から上の順。描く先が無ければ空。複数選択は描く先が同じあいだだけ有効。
    pub fn selected_layers(&self) -> Vec<LayerId> {
        let Some(active) = self
            .selected_layer
            .filter(|id| self.doc.layer(*id).is_some())
        else {
            return Vec::new();
        };
        let sel = &self.layer_ops.selection;
        let multi = sel.active == Some(active) && sel.ids.len() > 1;
        self.doc
            .layers()
            .iter()
            .map(|l| l.id())
            .filter(|id| *id == active || (multi && sel.ids.contains(id)))
            .collect()
    }

    /// 2 つ以上選んでいるか。
    pub fn has_multiple_layers_selected(&self) -> bool {
        self.selected_layers().len() > 1
    }

    /// 選ぶレイヤーをまとめて決める（`active` が描く先。`ids` に無ければ足す）。
    pub fn select_layers(&mut self, ids: impl IntoIterator<Item = LayerId>, active: LayerId) {
        let mut set: HashSet<LayerId> = ids
            .into_iter()
            .filter(|id| self.doc.layer(*id).is_some())
            .collect();
        set.insert(active);
        if self.selected_layer != Some(active) {
            self.set_edit_mask(false);
        }
        self.selected_layer = Some(active);
        self.layer_ops.selection.ids = set;
        self.layer_ops.selection.active = Some(active);
    }

    /// 修飾キー無しのクリック: そのレイヤーだけを選ぶ（起点にもする）。
    pub fn select_single_layer(&mut self, id: LayerId) {
        if self.selected_layer != Some(id) {
            self.set_edit_mask(false);
        }
        self.selected_layer = Some(id);
        self.layer_ops.selection = LayerSelection {
            ids: HashSet::new(),
            active: None,
            anchor: Some(id),
        };
    }

    /// Ctrl（Mac は Cmd）クリック: 選んでいなければ足して描く先に、選んでいれば外す（最後の 1 つは外さない。描く先を外したら、
    /// 残りのいちばん上が描く先）。
    pub fn toggle_layer_selected(&mut self, id: LayerId) {
        if self.doc.layer(id).is_none() {
            return;
        }
        self.layer_ops.selection.anchor = Some(id); // 次の Shift クリックの起点（外せなかった最後の 1 つでも）
        let mut current = self.selected_layers();
        if current.contains(&id) {
            if current.len() == 1 {
                return;
            }
            current.retain(|c| *c != id);
            let active = if Some(id) == self.selected_layer {
                *current.last().expect("残りがある")
            } else {
                self.selected_layer.expect("描く先")
            };
            self.select_layers(current, active);
        } else {
            current.push(id);
            self.select_layers(current, id);
        }
    }

    /// Shift クリック: 起点（最後に修飾キー無しか Ctrl で押したレイヤー）から押したレイヤーまでの、パネルの行（上から `rows`）を選ぶ。
    /// `add`（Ctrl + Shift）なら今の選択に足す。描く先は押したレイヤー。
    pub fn select_layer_range(&mut self, id: LayerId, add: bool, rows: &[LayerId]) {
        let Some(b) = rows.iter().position(|r| *r == id) else {
            return;
        };
        let anchor = self
            .layer_ops
            .selection
            .anchor
            .filter(|a| rows.contains(a))
            .or(self.selected_layer.filter(|a| rows.contains(a)));
        let a = anchor
            .and_then(|a| rows.iter().position(|r| *r == a))
            .unwrap_or(b);
        let range = rows[a.min(b)..=a.max(b)].iter().copied();
        let ids: Vec<LayerId> = if add {
            self.selected_layers().into_iter().chain(range).collect()
        } else {
            range.collect()
        };
        self.select_layers(ids, id);
        self.layer_ops.selection.anchor = anchor.or(Some(id));
    }

    // ───────── 結合 ─────────

    /// Ctrl+E の結合（複数選んでいれば選んだレイヤーを、グループならグループを、そうでなければ下のレイヤーと）。
    pub(crate) fn merge_down_selected(&mut self) -> Result<(), CoreError> {
        let active = self
            .selected_layer
            .filter(|id| self.doc.layer(*id).is_some())
            .ok_or(CoreError::LayerNotFound)?;
        let ids = self.selected_layers();
        let op = if self.doc.topmost_of(&ids)?.len() > 1 {
            MergeOp::Layers(ids)
        } else if self.doc.layer(active).is_some_and(|l| l.is_group()) {
            MergeOp::Group(active)
        } else {
            MergeOp::Down(active)
        };
        self.run_merge(op, Self::merge_tolerance())
    }

    pub(crate) fn merge_visible_layers(&mut self) -> Result<(), CoreError> {
        self.run_merge(MergeOp::Visible, Self::merge_tolerance())
    }

    fn merge_tolerance() -> u8 {
        yolu_core::Document::MERGE_ROUNDING_TOLERANCE
    }

    /// 結合する。見た目が許容差を超えて変わるなら、何も変えずに確認のウィンドウを出す（`tolerance` が 255 なら確かめ済みで、そのまま結合）。
    pub(crate) fn run_merge(&mut self, op: MergeOp, tolerance: u8) -> Result<(), CoreError> {
        let result = match &op {
            MergeOp::Down(id) => self.doc.merge_down(*id, tolerance),
            MergeOp::Layers(ids) => self.doc.merge_layers(ids, tolerance),
            MergeOp::Group(id) => self.doc.merge_group(*id, tolerance),
            MergeOp::Visible => {
                let name = self.lang.pick("結合", "Merged");
                self.doc.merge_visible(name, tolerance)
            }
        };
        match result {
            Ok(report) => {
                self.finish_merge(&report);
                Ok(())
            }
            Err(CoreError::MergeAppearance(report)) if tolerance != u8::MAX => {
                self.layer_ops.merge_confirm = Some(MergeConfirm {
                    op,
                    channels: report
                        .changed_by_channel
                        .iter()
                        .map(|(c, n)| (*c, *n))
                        .collect(),
                });
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn finish_merge(&mut self, report: &LayerMergeReport) {
        self.layer_ops.merge_confirm = None;
        self.select_new(report.result_id);
        let lang = self.lang;
        let name = self
            .doc
            .layer(report.result_id)
            .map(|l| l.name().to_owned())
            .unwrap_or_default();
        // 丸めの差の画素数は載せない（見た目が許容差を超えて変わるときは、結合の前に確認のウィンドウがチャンネルごとに見せる）
        let mut text = lang.pick(
            format!("結合しました: {name}"),
            format!("Merged into {name}"),
        );
        let notes = merge_notes_text(lang, report.notes);
        if notes.is_empty() {
            self.info(Source::Layer, text);
        } else {
            // 結合したが、但し書き（落ちた・変わった物）があれば気をつけること
            text = format!("{text} — {notes}");
            self.warn(Source::Layer, text);
        }
    }

    /// 確認のウィンドウの「結合する」: 見た目が変わるのを承知で、同じ結合を行う。
    pub(crate) fn confirm_merge(&mut self) -> Result<(), CoreError> {
        let Some(confirm) = self.layer_ops.merge_confirm.take() else {
            return Ok(());
        };
        self.run_merge(confirm.op, u8::MAX)
    }

    /// 確認のウィンドウの「やめる」。
    pub(crate) fn cancel_merge(&mut self) {
        if self.layer_ops.merge_confirm.take().is_some() {
            self.info(
                Source::Layer,
                self.lang.pick("結合をやめました。", "Merge cancelled."),
            );
        }
    }

    // ───────── 選んだレイヤーへの操作 ─────────

    /// 選んでいるレイヤー（複数ならそのまま全部）を新しいグループに入れる（違うグループのレイヤーどうしは core が断る）。
    pub(crate) fn group_selected_layers(&mut self) -> Result<(), CoreError> {
        let ids = self.selected_layers();
        let members = self.doc.topmost_of(&ids)?;
        if members.is_empty() {
            return Err(CoreError::LayerNotFound);
        }
        let name = self.new_layer_name(LayerKind::Group);
        let group = self.doc.group_layers(&members, &name)?;
        self.select_new(group);
        Ok(())
    }

    /// 選んでいるレイヤー（グループなら中身ごと）を複製し、複製を選ぶ。
    pub(crate) fn duplicate_selected_layers(&mut self) -> Result<(), CoreError> {
        let ids = self.selected_layers();
        if ids.is_empty() {
            return Err(CoreError::LayerNotFound);
        }
        let copies = self.doc.duplicate_layers(&ids)?;
        let active = *copies.last().expect("複製がある");
        self.select_layers(copies, active);
        self.set_edit_mask(false);
        Ok(())
    }

    /// 選んでいるレイヤーの表示を切り替える: どれか見えていれば全部隠し、全部隠れていれば全部見せる（Photoshop と同じ）。
    pub(crate) fn toggle_selected_visibility(&mut self) -> Result<(), CoreError> {
        let ids = self.selected_layers();
        if ids.is_empty() {
            return Err(CoreError::LayerNotFound);
        }
        let show = ids
            .iter()
            .all(|id| self.doc.layer(*id).is_some_and(|l| !l.visible()));
        self.doc.set_layers_visibility(&ids, show)
    }

    /// 選んでいるレイヤー（複数ならまとめて）を消す。何も残らなくなる削除は断る。
    pub(crate) fn delete_selected_layers(&mut self) -> Result<(), CoreError> {
        let ids = self.selected_layers();
        if ids.is_empty() {
            return Err(CoreError::LayerNotFound);
        }
        let members = self.doc.topmost_of(&ids)?;
        let removed: usize = members
            .iter()
            .map(|id| m2::subtree_len(&self.doc, *id))
            .sum();
        if removed >= self.doc.layers().len() {
            self.refuse(
                Source::Layer,
                self.lang.pick(
                    "最後のレイヤーは消せません。",
                    "Cannot delete the last layer.",
                ),
            );
            return Ok(());
        }
        // 消した塊のすぐ下のレイヤーを選ぶ（一番下の塊の始めの 1 つ下）
        let start = members
            .iter()
            .filter_map(|id| {
                self.doc
                    .layer_index(*id)
                    .map(|i| (i + 1).saturating_sub(m2::subtree_len(&self.doc, *id)))
            })
            .min()
            .unwrap_or(0);
        self.doc.remove_layers(&members)?;
        let layers = self.doc.layers();
        self.selected_layer = Some(layers[start.saturating_sub(1).min(layers.len() - 1)].id());
        self.layer_ops.selection = LayerSelection::default();
        self.set_edit_mask(false);
        Ok(())
    }

    /// 選んだレイヤーをまとめて、グループ `parent` の子の `position`（0 が一番下）へ動かす（ドラッグ）。
    pub(crate) fn move_selected_layers(
        &mut self,
        ids: &[LayerId],
        parent: Option<LayerId>,
        position: usize,
    ) -> Result<(), CoreError> {
        self.doc.move_layers(ids, parent, position)?;
        if let Some(p) = parent {
            self.m2.collapsed.remove(&p);
        }
        Ok(())
    }

    // ───────── ロック ─────────

    /// レイヤーのロックを付ける・外す（`flag` は個別の 1 種か、まとめて外すときの全部）。ロックは合成を変えないので 1 回の Undo。
    pub(crate) fn change_locks(
        &mut self,
        ids: &[LayerId],
        flag: LayerLocks,
        on: bool,
    ) -> Result<(), CoreError> {
        self.doc.change_layer_locks(ids, flag, on)?;
        let lang = self.lang;
        let what = if flag.bits() == 15 {
            lang.pick("すべてのロック", "all locks").to_owned()
        } else {
            let mut names = lock_names(lang, flag);
            if flag.contains(LayerLocks::ALL) {
                names.push(lock_name(lang, LayerLocks::ALL));
            }
            names.join(lang.pick("、", ", "))
        };
        self.info(
            Source::Layer,
            if on {
                lang.pick(format!("ロックしました: {what}"), format!("Locked: {what}"))
            } else {
                lang.pick(
                    format!("ロックを外しました: {what}"),
                    format!("Unlocked: {what}"),
                )
            },
        );
        Ok(())
    }

    // ───────── 変形 ─────────

    /// 動かすレイヤー: 選んでいるレイヤー（グループなら中身も）のうちラスターレイヤーだけ。塗りつぶし・調整は画素が無いので動かさない。
    pub fn transform_targets(&self) -> Vec<LayerId> {
        let Ok(members) = self.doc.topmost_of(&self.selected_layers()) else {
            return Vec::new();
        };
        self.doc
            .layers()
            .iter()
            .filter(|l| {
                l.kind() == LayerKind::Raster
                    && members
                        .iter()
                        .any(|m| *m == l.id() || m2::is_inside(&self.doc, l.id(), *m))
            })
            .map(|l| l.id())
            .collect()
    }

    /// 動かすものの範囲（複数のレイヤーならその和。選択範囲があればその中の画素だけ）。画素が無ければ None。
    /// `(x0, y0, x1, y1)`（キャンバスの座標、右・上は含まない）。
    pub fn transform_bounds(&self) -> Option<(i64, i64, i64, i64)> {
        let mut all: Option<(i64, i64, i64, i64)> = None;
        for id in self.transform_targets() {
            let Ok(Some(b)) = self.doc.transform_bounds(id, None) else {
                continue;
            };
            let b = (
                b.x as i64,
                b.y as i64,
                (b.x + b.width) as i64,
                (b.y + b.height) as i64,
            );
            all = Some(match all {
                None => b,
                Some(a) => (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)),
            });
        }
        all
    }

    /// 変形を 1 回の Undo で当てる。変わったか。動かすレイヤーや画素が無ければ短い理由で断る（文書は変えない）。
    pub(crate) fn apply_xform(&mut self, x: Xform) -> Result<bool, CoreError> {
        let lang = self.lang;
        let targets = self.transform_targets();
        if targets.is_empty() {
            self.refuse(
                Source::Transform,
                lang.pick(
                    "動かす画素のあるレイヤーがありません。",
                    "No layer with pixels to move.",
                ),
            );
            return Ok(false);
        }
        let Some(bounds) = self.transform_bounds() else {
            self.refuse(
                Source::Transform,
                if self.doc.selection().is_some() {
                    lang.pick(
                        "選択範囲の中に動かす画素がありません。",
                        "No pixels to move inside the selection.",
                    )
                } else {
                    lang.pick("動かす画素がありません。", "No pixels to move.")
                },
            );
            return Ok(false);
        };
        let (cx, cy) = (
            (bounds.0 + bounds.2) as f64 / 2.0,
            (bounds.1 + bounds.3) as f64 / 2.0,
        );
        // 90° の倍数の回転は、軸を画素の格子に合わせて画素をそのまま写す（半画素ずれると補間で滲む）
        let about = |degrees: f64| {
            let quarter = (degrees / 90.0).rem_euclid(2.0);
            if (quarter - 1.0).abs() < 1e-9 {
                (cx.round_ties_even(), cy.round_ties_even())
            } else {
                (cx, cy)
            }
        };
        let (transform, resampling, done) = match x {
            Xform::Move { dx, dy } => (
                Affine2D::translation(dx as f64, dy as f64),
                Resampling::Bilinear,
                lang.pick("移動しました。", "Moved."),
            ),
            Xform::Rotate90 { clockwise } => {
                let degrees = if clockwise { -90.0 } else { 90.0 };
                (
                    Affine2D::from_parts(about(degrees), (0.0, 0.0), degrees, (1.0, 1.0))?,
                    self.transform.resampling,
                    if clockwise {
                        lang.pick("時計回りに 90° 回転しました。", "Rotated 90° clockwise.")
                    } else {
                        lang.pick(
                            "反時計回りに 90° 回転しました。",
                            "Rotated 90° counter-clockwise.",
                        )
                    },
                )
            }
            Xform::Flip { horizontal } => (
                Affine2D::from_parts(
                    (cx, cy),
                    (0.0, 0.0),
                    0.0,
                    if horizontal { (-1.0, 1.0) } else { (1.0, -1.0) },
                )?,
                self.transform.resampling,
                if horizontal {
                    lang.pick("左右反転しました。", "Flipped horizontally.")
                } else {
                    lang.pick("上下反転しました。", "Flipped vertically.")
                },
            ),
            Xform::Numeric {
                dx,
                dy,
                degrees,
                sx,
                sy,
            } => {
                if sx == 0.0 || sy == 0.0 {
                    self.refuse(
                        Source::Transform,
                        lang.pick("拡大率は 0 にできません。", "Scale must not be 0%."),
                    );
                    return Ok(false);
                }
                (
                    Affine2D::from_parts(about(degrees), (dx, dy), degrees, (sx, sy))?,
                    self.transform.resampling,
                    lang.pick("変形しました。", "Transformed."),
                )
            }
            Xform::Affine(t) => (
                t,
                self.transform.resampling,
                lang.pick("変形しました。", "Transformed."),
            ),
        };
        // テキストレイヤーは画素でなく値（基準の点・回転・サイズ）を動かして描き直す。ほかのレイヤーと一緒なら 1 回の Undo にまとめる
        let (texts, pixels): (Vec<LayerId>, Vec<LayerId>) = targets
            .iter()
            .partition(|id| self.doc.layer(**id).is_some_and(|l| l.text().is_some()));
        let changed = if texts.is_empty() {
            self.doc.transform_layers(&targets, transform, resampling)?
        } else {
            if self.doc.selection().is_some() {
                self.refuse(
                    Source::Transform,
                    lang.with_reason(
                        lang.pick("テキストレイヤーは動かせません", "Cannot move a text layer"),
                        lang.pick(
                            "選択範囲の中だけは動かせない",
                            "it cannot move only inside a selection",
                        ),
                    ),
                );
                return Ok(false);
            }
            let mut moved = Vec::new();
            for id in &texts {
                let value = self
                    .doc
                    .layer(*id)
                    .and_then(|l| l.text())
                    .cloned()
                    .expect("テキストレイヤー");
                let next = match crate::textlayer::transformed(lang, &value, &transform) {
                    Ok(v) => v,
                    Err(reason) => {
                        self.refuse(Source::Transform, reason);
                        return Ok(false);
                    }
                };
                let mut next = next;
                let font = match self.text_font(&next.font) {
                    Ok((found, bytes)) => {
                        next.font = found;
                        bytes
                    }
                    Err(reason) => {
                        self.refuse(
                            Source::Transform,
                            lang.with_reason(
                                lang.pick(
                                    "テキストレイヤーを動かせません",
                                    "Cannot move a text layer",
                                ),
                                reason,
                            ),
                        );
                        return Ok(false);
                    }
                };
                moved.push((*id, next, font));
            }
            self.doc.batch(|d| {
                let mut changed = false;
                if !pixels.is_empty() {
                    changed |= d.transform_layers(&pixels, transform, resampling)?;
                }
                for (id, next, font) in &moved {
                    let before = d.undo_count();
                    d.set_text(*id, next.clone(), font, false)?;
                    changed |= d.undo_count() != before;
                }
                Ok(changed)
            })?
        };
        self.info(
            Source::Transform,
            if changed {
                done
            } else {
                lang.pick("変わりませんでした。", "Nothing changed.")
            },
        );
        Ok(changed)
    }
}
