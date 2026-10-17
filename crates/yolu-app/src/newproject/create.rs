//! 新規プロジェクトを作る。ウィンドウの行を検めて文書を全部作ってから（ここまでが失敗しうる所）、モデルを 3D ビューに入れ、セットを入れ替える
//! （失敗しない）。モデルはウィンドウが別のスレッドで読んだものをそのまま入れる。1 つの行は、選んだマテリアルを全部持つ 1 つのセットになる。

use super::{new_set_document, NpWindow, Prep, MAX_NAME, MAX_SETS, RESOLUTIONS};
use crate::bake::BakeAction;
use crate::engine::NormalSettings;
use crate::model::SceneModel;
use crate::sets::{first_set_name, guid_string, MaterialRef, TextureSets};
use crate::shelf::ShelfState;
use crate::state::{Action, AppState};
use crate::view3d::pose;

/// 作るセット 1 つ（名前・名前をマテリアルから付けたままか・鍵・大きさ）。
type NewSet = (String, bool, Vec<MaterialRef>, u32);

/// ウィンドウの行から、作るセットを決める。モデルのマテリアルの組があれば、マテリアルの無い行は捨てる（描いた物が無いので）。全部の行に
/// マテリアルが無ければ断る。組が無ければ（モデル無し）、行ごとにどのマテリアルにも付けないセット（鍵は 0 から順の仮のスロットの番号）。
/// 名前・大きさが決まりに合わなければ断る。
fn new_sets(win: &NpWindow, app: &AppState) -> Result<Vec<NewSet>, String> {
    let lang = app.lang;
    let groups = win.groups(app);
    let mut out: Vec<NewSet> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for (i, d) in win.drafts.iter().enumerate() {
        let materials = d.materials();
        if !groups.is_empty() && materials.is_empty() {
            continue;
        }
        let name = d.name.trim();
        if name.is_empty()
            || name.encode_utf16().count() > MAX_NAME
            || name.chars().any(|c| c.is_control())
        {
            return Err(lang.f(
                "newproject-a-texture-set-name-is",
                &[("max_name", &MAX_NAME)],
            ));
        }
        if names
            .iter()
            .any(|n| n.to_uppercase() == name.to_uppercase())
        {
            return Err(lang.f("newproject-two-texture-sets-are-named", &[("name", &name)]));
        }
        names.push(name.to_owned());
        if d.size.0 != d.size.1 || !RESOLUTIONS.contains(&d.size.0) {
            return Err(lang.t("newproject-choose-one-of-the-resolutions").into());
        }
        let (keys, auto_name) = if groups.is_empty() {
            let auto = name == first_set_name(lang);
            (vec![MaterialRef::PendingSlot(i as u16)], auto)
        } else {
            let keys: Vec<MaterialRef> = materials
                .iter()
                .filter_map(|g| groups.get(*g))
                .map(|g| g.key.clone())
                .collect();
            // 名前を最初のマテリアルから付けたままなら、モデルの照合で付け直してよい
            let auto = materials
                .first()
                .and_then(|g| groups.get(*g))
                .is_some_and(|g| name == g.name);
            (keys, auto)
        };
        out.push((name.to_owned(), auto_name, keys, d.size.0));
    }
    if out.is_empty() {
        return Err(lang.t("newproject-no-texture-set-is-chosen").into());
    }
    if out.len() > MAX_SETS {
        return Err(crate::lang::refusals::set_limit(lang));
    }
    Ok(out)
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
    let groups = win.groups(app);
    let chosen = new_sets(win, app)?;
    let normal = NormalSettings::DEFAULT.with_file_direction(win.normal);
    let tile = crate::engine::Document::DEFAULT_TILE_SIZE;
    // セットの文書を全部作る（失敗しうるのはここまで）
    let mut parts: Vec<(
        String,
        String,
        bool,
        Vec<MaterialRef>,
        crate::engine::Document,
    )> = Vec::new();
    for (name, auto_name, keys, size) in chosen {
        let doc = new_set_document(size, size, tile, lang, win.template.channels(), normal)?;
        parts.push((guid_string(doc.id()), name, auto_name, keys, doc));
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
    parts: Vec<(
        String,
        String,
        bool,
        Vec<MaterialRef>,
        crate::engine::Document,
    )>,
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
