//! 書き出しのウィンドウの 2 つの一覧: 左の書き出すテクスチャセット（チェック）と、右の書き出すファイル（書く前に出す名前・色空間・もうあるか）。
//!
//! - セットのチェックは、外したセットの uid だけを覚える（`WindowState::unchecked`。プロジェクトの間だけで、.ylp・設定には入れない）。
//!   外していないセットは全部入りなので、セットを足せば入り、消せば（uid が無くなって）一覧から消え、名前の変更はそのまま追いかける。
//!   出力テンプレートが「今のチャンネル」のときは、一覧は今のセットだけを示し、選べない。読むだけのセットは書き出せないので、外れたまま触れない。
//! - 書くファイルの一覧は、書き出しの道（`plan_export`・`plan_channels`）が名前を決める関数（`template_names`・`channel_names`）をそのまま呼んで求める。
//!   文書は写さず、塗り広げも画像づくりもしない（名前と色空間だけ）ので、ウィンドウが開いている間は毎フレーム求めてよく、表示と実際に書く名前がずれない。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use yolu_core::export::ExportTemplate;
use yolu_io::export::existing_files;

use super::window::ExportForm;
use super::{default_channel_file_name, stem, ChannelNames, Which};
use crate::bake::OcclusionState;
use crate::state::AppState;

/// 左の一覧の 1 行（テクスチャセット）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetRow {
    pub uid: u32,
    /// セットの今の名前（名前を変えれば、そのまま替わる）。
    pub name: String,
    /// 書き出すか。
    pub checked: bool,
    /// チェックを替えられるか（今のチャンネルの 1 枚と、読むだけのセットは替えられない）。
    pub enabled: bool,
    /// 読むだけで書き出せないセットか。
    pub read_only: bool,
}

/// 右の一覧の 1 行（書くファイル）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRow {
    /// 書くファイルの名前（出力先のフォルダの中の）。
    pub name: String,
    /// sRGB の色か（そうでなければリニア）。
    pub srgb: bool,
    /// 出力先にもうあって、置き換えることになるか。
    pub exists: bool,
}

/// 書くファイルの一覧。書けない理由（書く物が無い・同じファイルになる名前・読むだけのセット）があれば、その文。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    pub files: Vec<FileRow>,
    pub problem: Option<String>,
}

impl AppState {
    /// 左の一覧の行（今の出力テンプレートに合わせた形）。
    pub fn export_set_rows(&self) -> Vec<SetRow> {
        let unchecked = self
            .export_chosen()
            .map(|w| w.unchecked.as_slice())
            .unwrap_or_default();
        let row = |i: usize, selectable: bool| {
            let set = self.sets.get(i).expect("範囲内");
            let read_only = set.read_only.is_some();
            SetRow {
                uid: set.uid,
                name: set.name.clone(),
                checked: !read_only && (!selectable || !unchecked.contains(&set.uid)),
                enabled: selectable && !read_only,
                read_only,
            }
        };
        if self.export_form().writes_file() {
            return vec![row(self.sets.current_index(), false)];
        }
        (0..self.sets.len()).map(|i| row(i, true)).collect()
    }

    /// 書き出すセットの uid（左の一覧でチェックが入っている物。並びはセットの並び）。
    pub fn export_checked_uids(&self) -> Vec<u32> {
        self.export_set_rows()
            .into_iter()
            .filter(|r| r.checked)
            .map(|r| r.uid)
            .collect()
    }

    /// 書き出しのウィンドウが、焼いた AO の照合のために、今のモデルの入力を追っているか（出力テンプレートがテンプレートで、チェックしたセットに焼いた AO が
    /// あるとき。AO を使うのはテンプレートだけ）。追っていないと、`release_idle_bake_input` が毎フレーム作りかけを手放し、一覧が入力を求めて
    /// 作り直すことを繰り返す（`bake_input_followed`）。
    pub(crate) fn export_window_follows_input(&self) -> bool {
        if !self.export.window.open || self.export_form().template_id().is_none() {
            return false;
        }
        let checked = self.export_checked_uids();
        (0..self.sets.len()).any(|i| {
            self.has_baked_occlusion(i)
                && self
                    .sets
                    .get(i)
                    .is_some_and(|set| checked.contains(&set.uid))
        })
    }

    /// 書き出しに使える焼いた AO があるか（ウィンドウの一覧用。モデルの入力を作っている間は、照合できないので使えない扱い。作りかけは
    /// `export_window_follows_input` が手放させない）。
    fn export_has_occlusion(&mut self, index: usize) -> bool {
        if !self.has_baked_occlusion(index) {
            return false;
        }
        let input = self.bake_input_nowait().and_then(Result::ok);
        self.occlusion_state(index, input.as_deref()) == OcclusionState::Current
    }

    /// 今のチャンネルの PNG 1 枚の、書く名前（出力先のファイルの名前。決まっていなければ既定の名前）と、出力先のフォルダ。
    fn preview_channel_png(&self, destination: Option<&Path>) -> (Option<PathBuf>, Named) {
        let lang = self.lang;
        let set = self.sets.current_index();
        let name = destination
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| default_channel_file_name(self));
        let dir = destination
            .and_then(Path::parent)
            .filter(|d| !d.as_os_str().is_empty())
            .map(Path::to_path_buf);
        let named = match self.sets.get(set).and_then(|s| s.read_only.clone()) {
            Some(reason) => Err(crate::lang::refusals::read_only_set(lang, &reason)),
            None => self
                .channel_names(
                    &Which::One {
                        set,
                        channel: self.m2.paint_channel,
                        file_name: name,
                    },
                    &[set],
                )
                .map(|n| channel_files(&n)),
        };
        (dir, named)
    }

    /// テンプレートの書き出しの、書く名前（チェックしたセットぶん）。
    fn preview_template(&mut self, id: &str, selected: &[u32]) -> Named {
        let lang = self.lang;
        let Some(template) = ExportTemplate::built_in_by_id(id) else {
            return Err(lang.pick(
                format!("書き出しのテンプレート「{id}」はありません。"),
                format!("No export template \"{id}\"."),
            ));
        };
        let indices = self.exportable_sets(&mut Vec::new(), Some(selected));
        let has_occlusion: Vec<bool> = indices
            .iter()
            .map(|&i| self.export_has_occlusion(i))
            .collect();
        let stem = stem(self);
        self.template_names(&template, &stem, &indices, &has_occlusion)
            .map(|n| {
                n.planned
                    .iter()
                    .map(|p| (p.file_name.clone(), n.template.images[p.image].srgb()))
                    .collect()
            })
    }

    /// チャンネルごとの書き出しの、書く名前（チェックしたセットぶん）。
    fn preview_channels(&self, selected: &[u32]) -> Named {
        let indices = self.exportable_sets(&mut Vec::new(), Some(selected));
        if indices.is_empty() {
            return Ok(Vec::new());
        }
        let stem = stem(self);
        self.channel_names(
            &Which::All {
                stem: &stem,
                selected: None,
            },
            &indices,
        )
        .map(|n| channel_files(&n))
    }

    /// 今の出力テンプレート・出力先・チェックしたセットで書くファイルの一覧（書く前に出す。書き出しの道と同じ名前）。
    pub fn export_preview(&mut self) -> Preview {
        let form = self.export_form();
        let destination = self.export_destination();
        let selected = self.export_checked_uids();
        // 1 つもチェックが無いときは、書く物が無いことを一覧の中の文にしない（「書き出す」が押せないことで分かる）
        if form.selects_sets() && selected.is_empty() {
            return Preview::default();
        }
        // 書くファイルを入れるフォルダ（もうあるかの確かめ用）と、書くファイルの（名前, sRGB か）
        let (dir, named) = if form.writes_file() {
            self.preview_channel_png(destination.as_deref())
        } else if let Some(id) = form.template_id() {
            (destination, self.preview_template(id, &selected))
        } else {
            (destination, self.preview_channels(&selected))
        };
        match named {
            Ok(named) => {
                let names: Vec<String> = named.iter().map(|(n, _)| n.clone()).collect();
                let existing = self.export.window.exists.lookup(dir.as_deref(), names);
                Preview {
                    files: named
                        .into_iter()
                        .map(|(name, srgb)| FileRow {
                            exists: existing.contains(&name),
                            name,
                            srgb,
                        })
                        .collect(),
                    problem: None,
                }
            }
            Err(problem) => Preview {
                files: Vec::new(),
                problem: Some(problem),
            },
        }
    }
}

/// 書くファイルがもう出力先にあるかの調べを、使い回す間隔（外から足された・消されたファイルに、一覧が追いつくまで）。
const EXISTS_RECHECK: Duration = Duration::from_secs(1);

/// 出力先にもうあるファイルの調べ（ディスクを調べるので、毎フレームはやらない）。出力先と名前の並びが同じなら、次の 3 つのときだけ調べ直す: ウィンドウを開いたとき・
/// 書き終えたとき（`invalidate`）と、前の調べから `EXISTS_RECHECK` たったとき。
#[derive(Debug, Default)]
pub(super) struct ExistsCache {
    dir: Option<PathBuf>,
    names: Vec<String>,
    found: Vec<String>,
    at: Option<Instant>,
    /// ディスクを調べた回数（試験が、毎フレーム調べていないことを数える）。
    checks: u64,
}

impl ExistsCache {
    /// 次の `lookup` で、必ず調べ直す。
    pub(super) fn invalidate(&mut self) {
        self.at = None;
    }

    /// `names` のうち、`dir` にもうあるファイル（`dir` が決まっていなければ無し）。
    fn lookup(&mut self, dir: Option<&Path>, names: Vec<String>) -> Vec<String> {
        let fresh = self.at.is_some_and(|at| at.elapsed() < EXISTS_RECHECK)
            && self.dir.as_deref() == dir
            && self.names == names;
        if !fresh {
            self.found = dir
                .map(|d| {
                    existing_files(d, names.iter().map(String::as_str))
                        .iter()
                        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                        .collect()
                })
                .unwrap_or_default();
            self.dir = dir.map(Path::to_path_buf);
            self.names = names;
            self.at = Some(Instant::now());
            self.checks += 1;
        }
        self.found.clone()
    }
}

impl super::window::WindowState {
    /// 試験用: 書くファイルの有無を、ディスクで調べた回数。
    #[doc(hidden)]
    pub fn exists_checks(&self) -> u64 {
        self.exists.checks
    }
}

/// 書くファイルの（名前, sRGB か）。書けなければ理由。
type Named = Result<Vec<(String, bool)>, String>;

/// チャンネルの書き出しの名前の決まりから、（名前, sRGB か）の並び。
fn channel_files(names: &ChannelNames) -> Vec<(String, bool)> {
    names
        .wanted
        .iter()
        .zip(&names.image_of)
        .map(|((_, _, name), &i)| (name.clone(), names.images[i].srgb()))
        .collect()
}

impl ExportForm {
    /// 出力テンプレートを選ぶとき、チェックの一覧が選べる形か（今のチャンネルの 1 枚は、今のセットだけ）。
    pub fn selects_sets(self) -> bool {
        !self.writes_file()
    }
}
