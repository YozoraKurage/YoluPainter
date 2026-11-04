//! UDIM のテクスチャセット: テクスチャセットの右クリックのメニューの「UDIM」（入・切。1 回の Undo）と「UDIM のタイルを今のモデルに合わせる」。
//!
//! 文書の操作は core（`Document::enable_udim`・`disable_udim`・`sync_udim`）。ここは、今のモデルのそのセットのマテリアルの三角形が使っている
//! タイル（UDIM の写しの前の UV の重心のタイル）を洗い出して渡すことと、押せない理由・知らせ。3D ビューのモデルの写しは
//! `AppState::sync_view3d` が文書の並びから毎フレーム合わせる。

use crate::notice::Source;
use crate::state::AppState;

/// UDIM の操作（テクスチャセットの uid）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UdimAction {
    /// UDIM にする・やめる。
    Toggle(u32),
    /// 表に無いタイルを今のモデルから足す。
    Sync(u32),
}

/// 「UDIM」の項目を押せない理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    /// モデルが無い・セットがモデルのどのマテリアルにも付いていない。
    NoModel,
    /// 使っているタイルが 1 つ以下（UV が 0〜1 の中だけ）。
    OneTile,
}

impl AppState {
    /// 今のモデル（UDIM の写しの前の形）の、セット `index` の描くマテリアルの三角形が使っているタイル（昇順）。モデルが無い・セットが
    /// どのマテリアルにも付いていなければ None。
    pub fn model_udim_tiles(&self, index: usize) -> Option<Vec<u16>> {
        let model = self.view3d.full_model()?.unmapped();
        let material = self.set_material(index)?;
        Some(yolu_core::udim::used_tiles(
            model.geometry.triangles(),
            |m| material.contains(m),
        ))
    }

    /// セット `index` の「UDIM」の項目を押せない理由（UDIM のセットは、やめるのでいつも押せる）。
    pub fn udim_blocker(&self, index: usize) -> Option<Blocker> {
        if self.set_doc(index).udim().is_some() {
            return None;
        }
        match self.model_udim_tiles(index) {
            None => Some(Blocker::NoModel),
            Some(t) if t.len() < 2 => Some(Blocker::OneTile),
            Some(_) => None,
        }
    }

    /// セット `index` が UDIM で、今のモデルが表に無いタイルを使っているか（「合わせる」の項目を出す）。
    pub fn udim_needs_sync(&self, index: usize) -> bool {
        let Some(layout) = self.set_doc(index).udim() else {
            return false;
        };
        self.model_udim_tiles(index)
            .is_some_and(|t| t.iter().any(|x| layout.cell_of_tile(*x).is_none()))
    }

    /// セット `index` の「UDIM」の項目を押せない理由の短い文（ツールチップ。押せるなら None）。
    pub fn udim_blocked(&self, index: usize) -> Option<&'static str> {
        self.udim_blocker(index)
            .map(|why| self.udim_blocker_text(why))
    }

    /// 項目を押せない理由の短い文（ツールチップ）。
    pub fn udim_blocker_text(&self, why: Blocker) -> &'static str {
        match why {
            Blocker::NoModel => self.lang.t("udim-needs-a-model"),
            Blocker::OneTile => self.lang.t("udim-one-tile"),
        }
    }

    pub fn udim_apply(&mut self, action: UdimAction) {
        let uid = match action {
            UdimAction::Toggle(u) | UdimAction::Sync(u) => u,
        };
        let Some(index) = self.session.sets.index_of(uid) else {
            return;
        };
        if let Some(reason) = self
            .session
            .sets
            .get(index)
            .and_then(|s| s.read_only.clone())
        {
            let text = crate::lang::refusals::read_only_set(self.lang, &reason);
            return self.refuse(Source::TextureSet, text);
        }
        // 操作は今のセットの文書へ（Undo もそのセットの履歴）。ほかのセットなら先に切り替える（描いている最中は、切り替えと文書の操作が断る）
        if let Err(e) = self.switch_set(index) {
            return self.refuse(Source::TextureSet, e);
        }
        let lang = self.lang;
        let tiles = self.model_udim_tiles(index);
        self.session.doc.end_coalescing();
        let result = match action {
            UdimAction::Toggle(_) if self.session.doc.udim().is_some() => self
                .session
                .doc
                .disable_udim()
                .map(|_| lang.t("udim-turned-off")),
            UdimAction::Toggle(_) => match self.udim_blocker(index) {
                Some(why) => {
                    let text = self.udim_blocker_text(why);
                    return self.refuse(Source::TextureSet, text);
                }
                None => self
                    .session
                    .doc
                    .enable_udim(tiles.unwrap_or_default())
                    .map(|_| lang.t("udim-turned-on")),
            },
            UdimAction::Sync(_) => match self.session.doc.sync_udim(tiles.unwrap_or_default()) {
                Ok(Some(_)) => Ok(lang.t("udim-synced")),
                Ok(None) => Ok(lang.t("udim-already-synced")),
                Err(e) => Err(e),
            },
        };
        match result {
            Ok(text) => {
                // キャンバスの大きさが変わるので、全体が見える表示にする
                self.view.fit();
                self.sync_view3d();
                self.info(Source::TextureSet, text);
            }
            Err(e) => {
                let text = lang.with_reason(lang.t("udim-could-not-change"), lang.core_error(&e));
                self.refuse(Source::TextureSet, text);
            }
        }
    }
}
