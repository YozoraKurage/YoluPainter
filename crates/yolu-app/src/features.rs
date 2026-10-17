//! 機能の登録の表 `FEATURES`（描いている・ドラッグしている最中か・ツールを替えたときの片づけ・ペイントのモードを出るときの片づけ・
//! 文書を替えたときの片づけ・毎フレームの受け取り）。機能を 1 つ足すときは、機能のモジュールに `pub(crate) const FEATURE: Feature` を置き、
//! `FEATURES` に並べる（中心の `is_stroking`・`switch_to`・`set_mode`・`document_replaced`・`frame_body` は表を回すだけ）。
//! 片づけと受け取りは表の順に呼ぶ（順に意味がある。並びを変えたら、下のテストの並びも直す）。
//!
//! 表に入れない物: 機能の片づけでない物（サブツールの設定の出入り・効果の欄・ギズモの確定・文書の世代）と、毎フレームの受け取りのうち
//! egui の `Context` や引数が要る物・アプリの外枠の仕事（保存・外からの操作・Live Link・復旧の待ち）。外枠の仕事の間の位置は `PollStage`。

use crate::state::AppState;

/// 毎フレームの受け取りを呼ぶ位置（`YoluApp::frame_body` の外枠の仕事の間）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollStage {
    /// 落とした PSD の件数を知らせる前（別のスレッドの仕事の終わりと予算の同期）。
    BeforeDroppedPsds,
    /// 保存の結果を受ける前。
    BeforeSaving,
    /// 外からの操作の後、文字の色を描画色に合わせる前。
    BeforeTextColor,
    /// 復旧の書き置きの後（3D ビューの表示を今のセットに合わせる）。
    AfterRecovery,
}

/// 片づけ・受け取りの 1 つ。
pub type Hook = fn(&mut AppState);

/// 機能の表の 1 行。欄が None の物は、その並びに入らない。
pub struct Feature {
    /// 重ならない名前（並びのテストに使う）。
    pub id: &'static str,
    /// 描いている・ドラッグしている最中か（ほかの編集・取り消し・保存・モードの切り替えを断る）。
    pub stroking: Option<fn(&AppState) -> bool>,
    /// ツールが替わったときの片づけ（途中の形・ドラッグを捨てる）。
    pub tool_left: Option<Hook>,
    /// ペイントのモードを出るときの片づけ。
    pub paint_left: Option<Hook>,
    /// 今の文書を別の文書に替えたときの片づけ（前の文書を指す物を捨てる）。
    pub document_replaced: Option<Hook>,
    /// 毎フレームの受け取り（呼ぶ位置と関数）。
    pub poll: Option<(PollStage, Hook)>,
}

impl Feature {
    /// 欄を持たない行。
    pub const fn new(id: &'static str) -> Feature {
        Feature {
            id,
            stroking: None,
            tool_left: None,
            paint_left: None,
            document_replaced: None,
            poll: None,
        }
    }
}

/// 機能の表（片づけと受け取りはこの順）。
pub const FEATURES: &[Feature] = &[
    crate::eyedrop::FEATURE,
    crate::canvas::FEATURE,
    crate::clone_source::FEATURE,
    crate::view3d::input::FEATURE,
    crate::selection::FEATURE,
    crate::transform::FEATURE,
    crate::region::FEATURE,
    crate::pathtool::FEATURE,
    crate::gradient::FEATURE,
    crate::drafting::FEATURE,
    crate::rulers::FEATURE,
    crate::objects::FEATURE,
    crate::bake::FEATURE,
    crate::export::FEATURE,
    crate::prefs::BUDGETS,
    crate::prefs::TILE_CACHE,
    crate::psd::FEATURE,
    crate::distribute::FEATURE,
    crate::brushes::import::FEATURE,
    crate::brushes::clipstudio::FEATURE,
    crate::fx::FEATURE,
    crate::newproject::FEATURE,
    crate::update::FEATURE,
    crate::clipboard::FEATURE,
    crate::textlayer::FEATURE,
    crate::model::FEATURE,
    crate::uv_wireframe::overlap::FEATURE,
    crate::dropin::FEATURE,
];

/// どれかの機能が描いている・ドラッグしている最中か。
pub fn stroking(app: &AppState) -> bool {
    FEATURES.iter().any(|f| f.stroking.is_some_and(|s| s(app)))
}

/// `skip` の行を除いて、どれかの機能が描いている・ドラッグしている最中か。
pub fn stroking_except(app: &AppState, skip: &str) -> bool {
    FEATURES
        .iter()
        .filter(|f| f.id != skip)
        .any(|f| f.stroking.is_some_and(|s| s(app)))
}

/// ツールが替わった（表の順に片づける）。
pub fn tool_left(app: &mut AppState) {
    for f in FEATURES {
        if let Some(left) = f.tool_left {
            left(app);
        }
    }
}

/// ペイントのモードを出る（表の順に片づける）。
pub fn paint_left(app: &mut AppState) {
    for f in FEATURES {
        if let Some(left) = f.paint_left {
            left(app);
        }
    }
}

/// 今の文書を別の文書に替えた（表の順に片づける）。
pub fn document_replaced(app: &mut AppState) {
    for f in FEATURES {
        if let Some(replaced) = f.document_replaced {
            replaced(app);
        }
    }
}

/// `stage` の位置の毎フレームの受け取り（表の順）。
pub fn poll(app: &mut AppState, stage: PollStage) {
    for f in FEATURES {
        if let Some((at, poll)) = f.poll {
            if at == stage {
                poll(app);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(has: impl Fn(&Feature) -> bool) -> Vec<&'static str> {
        FEATURES.iter().filter(|f| has(f)).map(|f| f.id).collect()
    }

    fn polled(stage: PollStage) -> Vec<&'static str> {
        ids(|f| f.poll.is_some_and(|(at, _)| at == stage))
    }

    #[test]
    fn ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for f in FEATURES {
            assert!(seen.insert(f.id), "重なった id: {}", f.id);
        }
    }

    #[test]
    fn stroking_rows() {
        assert_eq!(
            ids(|f| f.stroking.is_some()),
            [
                "canvas",
                "view3d.input",
                "transform",
                "region",
                "path",
                "drafting",
                "rulers",
            ]
        );
    }

    #[test]
    fn tool_left_order() {
        assert_eq!(
            ids(|f| f.tool_left.is_some()),
            [
                "selection",
                "transform",
                "path",
                "gradient",
                "drafting",
                "text"
            ]
        );
    }

    /// パスの途中の形は、ペイントのモードを出ても残す（ツールを替えたときだけ捨てる）。
    #[test]
    fn paint_left_order() {
        assert_eq!(
            ids(|f| f.paint_left.is_some()),
            ["selection", "transform", "gradient", "drafting", "text"]
        );
    }

    #[test]
    fn document_replaced_order() {
        assert_eq!(
            ids(|f| f.document_replaced.is_some()),
            [
                "eyedrop",
                "canvas",
                "clone_source",
                "view3d.input",
                "drafting",
                "rulers",
                "objects",
            ]
        );
    }

    #[test]
    fn poll_order() {
        assert_eq!(
            polled(PollStage::BeforeDroppedPsds),
            ["bake", "export", "budgets", "tile_cache", "psd", "dropin"]
        );
        assert_eq!(polled(PollStage::BeforeSaving), ["distribute"]);
        assert_eq!(
            polled(PollStage::BeforeTextColor),
            [
                "brushes.import",
                "brushes.csp",
                "fx",
                "newproject",
                "update",
                "clipboard",
                "text",
            ]
        );
        assert_eq!(
            polled(PollStage::AfterRecovery),
            ["view3d.sync", "uv.overlap"]
        );
    }
}
