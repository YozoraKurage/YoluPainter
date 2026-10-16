//! 新規プロジェクトを作る。ウィンドウの値を検めて文書を全部作ってから（ここまでが失敗しうる所）、モデルを 3D ビューに入れ、セットを入れ替える
//! （失敗しない）。モデルはウィンドウが別のスレッドで読んだものをそのまま入れる。

use super::{new_set_document, unique, NpWindow, Prep, RESOLUTIONS};
use crate::bake::BakeAction;
use crate::engine::NormalSettings;
use crate::model::SceneModel;
use crate::sets::{first_set_name, guid_string, MaterialRef, TextureSets};
use crate::shelf::ShelfState;
use crate::state::{Action, AppState};
use crate::view3d::pose;

/// ウィンドウで選んだマテリアルの組（番号の順。選び直していなければ先頭から上限まで）。モデルが無ければ空。1 つも選んでいなければ断る。
fn chosen_groups(
    win: &NpWindow,
    count: usize,
    lang: crate::lang::Lang,
) -> Result<Vec<usize>, String> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let chosen = win.chosen(count);
    if chosen.is_empty() {
        return Err(lang.t("newproject-no-texture-set-is-chosen").into());
    }
    Ok(chosen)
}

/// ウィンドウの値で新しいプロジェクトを作る。断るときは何も変えず、理由を返す。
/// 作った結果の知らせ（種類と文。続けてベイクを始めたなら、その知らせを後ろに添え、種類も重いほう）。
pub(super) fn create_from_window(
    app: &mut AppState,
    win: &mut NpWindow,
) -> Result<(crate::notice::Kind, String), String> {
    let lang = app.lang;
    if app.is_stroking() {
        return Err(crate::lang::refusals::during_stroke(lang).into());
    }
    if app.is_saving() {
        return Err(crate::lang::refusals::saving(lang).into());
    }
    if !win.is_ready() {
        return Err(lang.t("newproject-the-model-is-not-ready").into());
    }
    if !RESOLUTIONS.contains(&win.resolution) {
        return Err(lang.t("newproject-choose-one-of-the-resolutions").into());
    }
    let groups = win.groups(app);
    let chosen = chosen_groups(win, groups.len(), lang)?;
    let normal = NormalSettings::DEFAULT.with_file_direction(win.normal);
    let tile = crate::engine::Document::DEFAULT_TILE_SIZE;
    // セットの文書を全部作る（失敗しうるのはここまで）
    let mut parts: Vec<(String, String, bool, MaterialRef, crate::engine::Document)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut make = |name: &str, key: MaterialRef| -> Result<(), String> {
        let doc = new_set_document(
            win.resolution,
            win.resolution,
            tile,
            lang,
            win.template.channels(),
            normal,
        )?;
        let unique_name = unique(name, names.iter().map(String::as_str));
        names.push(unique_name.clone());
        parts.push((guid_string(doc.id()), unique_name, true, key, doc));
        Ok(())
    };
    if chosen.is_empty() {
        make(first_set_name(lang), MaterialRef::PendingSlot(0))?;
    } else {
        for g in &chosen {
            make(&groups[*g].name, groups[*g].key.clone())?;
        }
    }
    let bake = win.bake;
    let sets_count = parts.len();
    let over_limit = win.over_limit(groups.len());
    // 準備したモデルをウィンドウから取り出す（以降は失敗しない）
    let model = match std::mem::replace(&mut win.prep, Prep::Idle) {
        Prep::Ready { path, model, .. } => Some((path, model)),
        _ => None,
    };
    win.model = None;
    Ok(install(app, parts, model, sets_count, over_limit, bake))
}

/// 作った文書とモデルを入れる（失敗しない）。知らせの文を返す。
fn install(
    app: &mut AppState,
    parts: Vec<(String, String, bool, MaterialRef, crate::engine::Document)>,
    model: Option<(std::path::PathBuf, Box<pose::PreparedModel>)>,
    sets_count: usize,
    over_limit: usize,
    bake: bool,
) -> (crate::notice::Kind, String) {
    let lang = app.lang;
    app.np_project_replaced();
    app.session.doc.end_coalescing();
    let mut model_name = model.as_ref().map(|(_, m)| m.rig().name().to_owned());
    let has_model = model.is_some();
    match model {
        Some((path, prepared)) => {
            pose::install_prepared(&mut app.view3d, *prepared);
            pose::request_focus(&mut app.view3d);
            if let Some(session) = app.view3d.pose.session.as_ref() {
                app.session.model = Some(SceneModel::from_rig(&session.rig));
            }
            app.np.model_file = Some(path);
        }
        None => app.drop_project_model(),
    }
    // モデルを選ばなかったとき、Live Link のモデルが付いていれば、それが今のプロジェクトのモデル（そのマテリアルにセットを付ける）
    if model_name.is_none() {
        model_name = app
            .session
            .model
            .as_ref()
            .filter(|m| m.is_link())
            .map(|m| m.name.clone());
    }
    let (sets, doc) = TextureSets::from_new_parts(parts);
    app.replace_sets_with(sets, doc, false);
    // ウィンドウで選んだ解像度（Live Link の元の絵が、最初のセットの大きさを黙って替えない）
    app.session.resolution_chosen = true;
    app.reset_project_identity(lang.t("newproject-untitled").into(), ShelfState::default());
    app.sync_mesh_map_view();
    // 上限を超えるマテリアルがあれば、セットにならなかった数を言う（黙って落とさない）
    let over = if over_limit > 0 {
        lang.f(
            "newproject-more-materials-are-over-the",
            &[("over_limit", &over_limit)],
        )
    } else {
        String::new()
    };
    let created = match &model_name {
        None => lang.t("newproject-new-project-without-a-model").to_owned(),
        Some(name) if sets_count == 1 => lang.f("newproject-new-project-for", &[("name", &name)]),
        Some(name) => lang.f(
            "newproject-new-project-for-with-texture",
            &[
                ("name", &name),
                ("sets_count", &sets_count),
                ("over", &over),
            ],
        ),
    };
    if bake && has_model {
        app.info(crate::notice::Source::Project, created.clone());
        app.apply(Action::Bake(BakeAction::Start));
        if app.message != created {
            let kind = app.message_kind().worse(crate::notice::Kind::Info);
            return (kind, format!("{created} {}", app.message));
        }
    }
    (crate::notice::Kind::Info, created)
}
