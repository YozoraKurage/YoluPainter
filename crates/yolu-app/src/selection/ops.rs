//! 選択範囲を使う操作（選択範囲の下のボタンの帯が押す）: 描画色で塗りつぶす・消去・コピーして新しいレイヤー・マスクにする。
//! どれも 1 回の Undo（複数の段を作る操作は `Document::batch` で 1 段にまとめる）。計算と断りは core（`fill_material`・`fill_mask`・
//! `copy_pixels`・`paste_as_layer`・`add_layer_mask`）に任せ、ここは「どの層の何を」と、知らせだけを持つ。

use crate::engine::LayerKind;
use crate::state::AppState;

impl AppState {
    fn no_selection_text(&self) -> String {
        self.lang
            .pick("選択範囲がありません。", "No selection.")
            .into()
    }

    /// 選択範囲を、描画色（マテリアルで塗るなら組の全部。マスクを描いているならマスクの白）で塗りつぶす。`erase` なら消す
    /// （アルファを減らす。マスクなら隠す）。範囲の量の割合だけ効く（縁のぼかしはそのまま縁になる）。
    pub(super) fn sel_fill(&mut self, erase: bool) -> Result<String, String> {
        let lang = self.lang;
        if self.doc.selection().is_none() {
            return Err(self.no_selection_text());
        }
        let layer = crate::region::tools::paint_gate(self)?;
        let result = if self.m2.edit_mask {
            let reveal = crate::region::tools::mask_reveals(self, layer, erase);
            self.doc.fill_mask(layer, 1.0, None, reveal)
        } else {
            let channels = self.paint_channels();
            self.doc.fill_material(layer, &channels, 1.0, None, erase)
        };
        let changed = result.map_err(|e| crate::matpaint::refusal_text(lang, &e))?;
        let (done, same) = if erase {
            (
                lang.pick("選択範囲を消去しました。", "Erased the selection."),
                lang.pick("消す画素がありません。", "Nothing to erase."),
            )
        } else {
            (
                lang.pick("選択範囲を塗りつぶしました。", "Filled the selection."),
                lang.pick("塗りつぶしは変わりません。", "The fill did not change anything."),
            )
        };
        Ok(if changed { done } else { same }.into())
    }

    /// 選択範囲の画素（選んでいる層の描くチャンネル。マスクを描いているならマスク）を、新しいレイヤーとして元の位置に足す。
    /// OS のクリップボードもアプリの中のクリップボードも変えない。足したレイヤーを選び、選択範囲は今のまま残す（貼り付けは選択を
    /// 外すので、同じ選択へ戻す。全部 1 回の Undo）。
    pub(super) fn sel_to_new_layer(&mut self) -> Result<String, String> {
        let lang = self.lang;
        let Some(selection) = self.doc.selection().cloned() else {
            return Err(self.no_selection_text());
        };
        let (id, channel, mask) = self.clip_target()?;
        let limit = self.doc.stroke_budget_bytes();
        let name = self.layer_name_for_new();
        let result = self
            .doc
            .batch(|doc| {
                let clip = doc.copy_pixels(id, channel, mask, limit)?;
                let pasted = doc.paste_as_layer(&clip, channel, Some(&name), Some(id))?;
                doc.set_selection(Some(selection.clone()))?;
                Ok(pasted)
            })
            .map_err(|e| lang.core_error(&e))?;
        self.selected_layer = Some(result.layer);
        self.set_edit_mask(false);
        self.ensure_selection();
        Ok(format!(
            "{}: {name}",
            lang.pick(
                "コピーして新しいレイヤーにしました",
                "Copied to a new layer"
            )
        ))
    }

    /// 選択範囲を、選んでいる層のマスクにする: マスクが無ければ足し、選択範囲の外を隠す（マスクがあれば、その上に重ねて隠す。
    /// 選択範囲の縁の量は、その分だけ隠さない）。マスクを描く状態にし、選択範囲は今のまま残す。
    pub(super) fn sel_to_mask(&mut self) -> Result<String, String> {
        let lang = self.lang;
        let Some(selection) = self.doc.selection().cloned() else {
            return Err(self.no_selection_text());
        };
        let Some(id) = self.selected_layer.filter(|id| self.doc.layer(*id).is_some()) else {
            return Err(lang
                .pick("レイヤーが選ばれていません。", "No layer is selected.")
                .into());
        };
        let had_mask = self.doc.layer(id).is_some_and(|l| l.mask().is_some());
        let outside = selection.invert();
        // 外を「隠す」側へ書く値は、反転したマスクでは逆（保存値は隠す量。反転では 0 が隠れる）。足したばかりのマスクは反転していない
        let reveal = crate::region::tools::mask_reveals(self, id, true);
        self.doc
            .batch(|doc| {
                if !had_mask {
                    doc.add_layer_mask(id)?;
                }
                // 範囲の編集は「渡した範囲と今の選択範囲の重なり」だけ効くので、外を隠すあいだは選択を外し、あとで戻す
                doc.clear_selection()?;
                doc.fill_mask(id, 1.0, Some(&outside), reveal)?;
                doc.set_selection(Some(selection.clone()))?;
                Ok(())
            })
            .map_err(|e| lang.core_error(&e))?;
        self.selected_layer = Some(id);
        self.set_edit_mask(true);
        let kind = self.doc.layer(id).map(|l| l.kind());
        Ok(match (had_mask, kind) {
            (true, _) => lang.pick(
                "選択範囲の外をマスクで隠しました。",
                "Hid everything outside the selection in the mask.",
            ),
            (false, Some(LayerKind::Group)) => lang.pick(
                "グループにマスクを足しました。",
                "Added a mask to the group.",
            ),
            _ => lang.pick("選択範囲からマスクを作りました。", "Made a mask from the selection."),
        }
        .into())
    }
}
