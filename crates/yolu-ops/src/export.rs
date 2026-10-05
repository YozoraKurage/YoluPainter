//! 書き出しの命令（`export.channels`・`export.textures`・`export.psd`）。セットの文書（core）から、yolu-io の書き出しを通してファイルへ書く。
//!
//! - 書くのは検証済みの一時ファイルから 1 枚ずつの置き換え（PNG は `yolu_io::export::write_images`、PSD は同じ流儀でここ）。途中で失敗しても、
//!   もうあるファイルは変わらない（置き換えの途中の失敗だけ、済んだ分が `data` に出る）。
//! - もうあるファイルを置き換えるときは `confirm: true` が要る。無ければ、書く前に、置き換えるファイルの一覧つきで断る（何も書かない）。
//! - 文書は変えない。効いていない効果（マップ・モデルの入力が無い Generator など）は、書き出しに入らないことを `notes` に書く。
//! - 塗り広げ（パディング）・焼いた AO は、モデルから作るもので、画面なしでは渡せないので使わない。

use std::fs::{self, OpenOptions};
use std::io::Seek;
use std::path::{Path, PathBuf};

use serde_json::json;
use yolu_core::export::{ExportImage, ExportImageKind, ExportTemplate};
use yolu_core::{Channel, ChannelKind, Document};
use yolu_io::export::{
    existing_files, file_name, plan_template, sanitize, write_images, write_template, ExportFile,
    Overwrite, PlanSet, SetExport, WriteOptions,
};
use yolu_io::psd::{self, Compression, ExportControl, ExportOptions, NoteAction};

use crate::command::{ExportChannelsArgs, ExportPsdArgs, ExportTexturesArgs, PsdMode};
use crate::doc_ops::inactive_texts;
use crate::error::{ErrorCode, Noun, OpError};
use crate::host::{ExportJob, SetView};
use crate::path::PathPolicy;
use crate::refs::{channel_name, resolve_channel};
use crate::reply::{Exported, ExportedFile, Reply};
use crate::text::Text;

/// 書き出しを当てる。
pub fn run(view: &SetView<'_>, policy: &PathPolicy, job: &ExportJob<'_>) -> Result<Reply, OpError> {
    match job {
        ExportJob::Channels(a) => channels(view, policy, a),
        ExportJob::Textures(a) => textures(view, policy, a),
        ExportJob::Psd(a) => psd_file(view, policy, a),
    }
}

fn stem_of(view: &SetView<'_>, name: &Option<String>) -> String {
    name.clone().unwrap_or_else(|| view.stem.to_owned())
}

/// ファイル名にセット名を入れるか（セットが複数のとき、同じ名前の別セットを上書きしない）。
fn set_label<'a>(view: &'a SetView<'a>) -> Option<&'a str> {
    (view.set_count > 1).then_some(view.name)
}

fn confirm_files(files: &[PathBuf]) -> OpError {
    OpError::confirm_required(
        "もうあるファイルを置き換えます。confirm: true を付けてください",
        "Existing files would be replaced; pass confirm: true",
        Some(json!({"files": files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()})),
    )
}

fn overwrite_for(existing: &[PathBuf], confirm: bool) -> Result<Overwrite, OpError> {
    if existing.is_empty() {
        Ok(Overwrite::Refuse)
    } else if confirm {
        Ok(Overwrite::Replace)
    } else {
        Err(confirm_files(existing))
    }
}

fn exported(written: Vec<yolu_io::export::WrittenImage>, skipped: Vec<String>, doc: &Document) -> Reply {
    Reply::Exported(Exported {
        files: written
            .into_iter()
            .map(|w| ExportedFile {
                path: w.path.display().to_string(),
                width: w.width,
                height: w.height,
                replaced: w.replaced,
            })
            .collect(),
        skipped,
        notes: inactive_texts(doc),
    })
}

// ───────── チャンネルの PNG ─────────

fn channels(view: &SetView<'_>, policy: &PathPolicy, args: &ExportChannelsArgs) -> Result<Reply, OpError> {
    let doc = view.editable_doc()?;
    let dir = policy.resolve(&args.dir)?;
    let mut list: Vec<Channel> = Vec::new();
    if args.channels.is_empty() {
        list.extend(doc.channels().into_iter().filter(|c| yolu_core::export::uses(doc, *c)));
        if list.is_empty() {
            list.push(Channel::Color);
        }
    } else {
        for name in &args.channels {
            let channel = resolve_channel(doc, name)?;
            if !list.contains(&channel) {
                list.push(channel);
            }
        }
    }
    let stem = stem_of(view, &args.name);
    let images: Vec<ExportImage> = list
        .iter()
        .map(|c| {
            let suffix = sanitize(&channel_name(doc, *c));
            let suffix = if suffix.is_empty() || suffix.starts_with('.') { format!("Channel{}", c.index()) } else { suffix };
            let kind = match doc.channel_info(*c).map(|i| i.kind) {
                Some(ChannelKind::Normal) => ExportImageKind::Normal,
                _ => ExportImageKind::BaseColor,
            };
            ExportImage::of(suffix, kind)
        })
        .collect();
    let mut files = Vec::new();
    for image in &images {
        files.push(ExportFile {
            name: file_name(&stem, set_label(view), image).map_err(|e| OpError::from_export(&e))?,
            image,
            width: doc.width(),
            height: doc.height(),
        });
    }
    let existing = existing_files(&dir, files.iter().map(|f| f.name.as_str()));
    let overwrite = overwrite_for(&existing, args.confirm)?;
    let budget = doc.stroke_budget_bytes();
    let written = write_images(&dir, &files, &WriteOptions { overwrite, cancel: None }, |i| {
        yolu_core::export::channel_image(doc, list[i], budget).map_err(Into::into)
    })
    .map_err(|e| OpError::from_export(&e))?;
    Ok(exported(written, Vec::new(), doc))
}

// ───────── テンプレートの PNG ─────────

fn textures(view: &SetView<'_>, policy: &PathPolicy, args: &ExportTexturesArgs) -> Result<Reply, OpError> {
    let doc = view.editable_doc()?;
    let dir = policy.resolve(&args.dir)?;
    let id = args.template.as_deref().unwrap_or("unity-standard");
    let template = ExportTemplate::built_in_by_id(id).ok_or_else(|| {
        OpError::not_found(Noun::Template, id).with_data(json!({
            "templates": ExportTemplate::built_in().iter().map(|t| t.id.clone()).collect::<Vec<_>>()
        }))
    })?;
    let stem = stem_of(view, &args.name);
    let set = PlanSet { name: set_label(view), document: doc, has_occlusion: false };
    let planned = plan_template(&stem, &[set], &template).map_err(|e| OpError::from_export(&e))?;
    let existing = existing_files(&dir, planned.iter().map(|p| p.file_name.as_str()));
    let overwrite = overwrite_for(&existing, args.confirm)?;
    let report = write_template(
        &dir,
        &template,
        &SetExport {
            stem: &stem,
            set_name: set_label(view),
            document: doc,
            occlusion: None,
            padding: None,
            max_working_bytes: doc.stroke_budget_bytes(),
        },
        &WriteOptions { overwrite, cancel: None },
    )
    .map_err(|e| OpError::from_export(&e))?;
    Ok(exported(report.written, report.skipped, doc))
}

// ───────── PSD ─────────

fn note_text(note: &psd::ExportNote) -> Text {
    let name = &note.layer;
    let en = match &note.action {
        NoteAction::BakedFilters(_) => format!("Layer \"{name}\": its filters were baked into the pixels"),
        NoteAction::BakedFill(_) => format!("Layer \"{name}\": its image, projection, decal or gradient was baked into the pixels"),
        NoteAction::BakedTranslucentFill => format!("Layer \"{name}\": a translucent fill was baked into the pixels"),
        NoteAction::BakedPath => format!("Layer \"{name}\": a path layer was written as pixels; the path itself is not kept"),
        NoteAction::BakedMaskFilters(_) => format!("Layer \"{name}\": the mask filters were baked into the mask"),
        NoteAction::BakedInvertedMask => format!("Layer \"{name}\": an inverted mask was baked into the mask"),
        NoteAction::BakedClippedGroup => format!("Group \"{name}\": a clipped group became one raster layer"),
        NoteAction::DroppedClippingMark => format!("Group \"{name}\": a clipping mark that had no effect was dropped"),
        NoteAction::DroppedFilters(_) => format!("Layer \"{name}\": inactive filters were dropped (the pixels do not change)"),
        NoteAction::DroppedMaskFilters(_) => format!("Layer \"{name}\": inactive mask filters were dropped"),
        NoteAction::DroppedAnchor => format!("Layer \"{name}\": its anchor was dropped"),
        NoteAction::DroppedMaskAnchor => format!("Layer \"{name}\": its mask anchor was dropped"),
        NoteAction::NormalBlend => format!("Channel \"{name}\": normal layers overlap as colors in a PSD"),
        NoteAction::Rounded { max_diff, .. } => {
            format!("Layer \"{name}\": an adjustment was rounded to PSD steps (largest composite difference {max_diff})")
        }
        NoteAction::ExpandedGradientCurve { max_diff, .. } => {
            format!("Layer \"{name}\": a gradient was expanded to stops (approximation; largest composite difference {max_diff})")
        }
    };
    Text::new(note.message(), en)
}

/// 一時ファイルの持ち主（置き換える前に失敗・panic で手放すと消す）。
struct Temp(Option<PathBuf>);

impl Temp {
    fn path(&self) -> &Path {
        self.0.as_deref().expect("手放していない一時ファイル")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn io_error(what: &str, path: &Path, e: &std::io::Error) -> OpError {
    OpError::new(
        ErrorCode::Io,
        format!("{what}（{}）: {e}", path.display()),
        format!("{what} ({}): {e}", path.display()),
    )
}

fn psd_file(view: &SetView<'_>, policy: &PathPolicy, args: &ExportPsdArgs) -> Result<Reply, OpError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let doc = view.editable_doc()?;
    let path = policy.resolve(&args.path)?;
    let is_psd = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("psd"));
    if !is_psd {
        return Err(OpError::new(
            ErrorCode::PathRefused,
            "PSD のファイル名は .psd で終わります",
            "A PSD file name must end with .psd",
        )
        .with_data(json!({"path": path.display().to_string()})));
    }
    let channel = match &args.channel {
        Some(name) => resolve_channel(doc, name)?,
        None => Channel::Color,
    };
    let replaced = match fs::symlink_metadata(&path) {
        Ok(m) if m.is_file() => true,
        Ok(_) => {
            return Err(OpError::new(
                ErrorCode::PathRefused,
                "行き先が通常のファイルではありません",
                "The destination is not a regular file",
            )
            .with_data(json!({"path": path.display().to_string()})))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(io_error("行き先を調べられません", &path, &e)),
    };
    if replaced && !args.confirm {
        return Err(confirm_files(std::slice::from_ref(&path)));
    }
    let ctl = ExportControl { cancel: None, source_budget: Some(view.source_budget), max_file_bytes: None };
    let mode = match args.mode {
        PsdMode::Bake => psd::ExportMode::Bake,
        PsdMode::Flat => psd::ExportMode::Flat,
    };
    let plan = psd::plan_export(doc, &ExportOptions::new(channel, mode), &ctl)
        .map_err(|e| OpError::from_io(&e))?;
    if !plan.blockers.is_empty() {
        let issues: Vec<String> = plan.blockers.iter().map(|b| b.message()).collect();
        return Err(OpError::new(
            ErrorCode::Unsupported,
            format!("PSD に書けない層があります: {}", issues.join(" / ")),
            format!("{} layer(s) cannot be written to a PSD without losing information", issues.len()),
        )
        .with_data(json!({"issues": issues})));
    }
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|e| io_error("フォルダを作れません", dir, &e))?;
    let temp_path = dir.join(format!(".yolu-export-{}-{}.pending~", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&temp_path)
        .map_err(|e| io_error("一時ファイルを作れません", &temp_path, &e))?;
    let temp = Temp(Some(temp_path));
    let written = plan
        .write_psd(doc, &ctl, &mut file, Compression::Rle)
        .map_err(|e| OpError::from_io(&yolu_io::Error::from(e)))?;
    file.sync_all().map_err(|e| io_error("書き出しを確定できません", temp.path(), &e))?;
    // 読み戻して確かめる（読めない・書いたものと違う PSD は置き換えない）
    file.rewind().map_err(|e| io_error("読み戻せません", temp.path(), &e))?;
    let verified = psd::verify_stream(&mut std::io::BufReader::new(&file), None)
        .map_err(|e| OpError::from_io(&e))?;
    let mismatch = || {
        OpError::new(
            ErrorCode::Io,
            "書いた PSD の読み戻しが一致しません（置き換えていません）",
            "The written PSD did not read back identically; nothing was replaced",
        )
    };
    if verified.bytes != written.bytes || verified.layers != written.layers {
        return Err(mismatch());
    }
    file.rewind().map_err(|e| io_error("読み戻せません", temp.path(), &e))?;
    if !written.checksum.matches(&mut file, None).map_err(|e| OpError::from_io(&e))? {
        return Err(mismatch());
    }
    drop(file);
    // 置き換える。新しいファイルは「あれば失敗」の hard_link（確かめたあとに、別のファイルができていても上書きしない）
    if replaced {
        fs::rename(temp.path(), &path).map_err(|e| io_error("置き換えられません", &path, &e))?;
    } else {
        match fs::hard_link(temp.path(), &path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(confirm_files(std::slice::from_ref(&path)))
            }
            Err(e) => return Err(io_error("書けません", &path, &e)),
        }
    }
    // rename で一時ファイルは無くなっている。hard_link のときは、ここで Temp が消す
    let mut notes: Vec<Text> = plan.notes.iter().map(note_text).collect();
    notes.extend(inactive_texts(doc));
    Ok(Reply::Exported(Exported {
        files: vec![ExportedFile {
            path: path.display().to_string(),
            width: doc.width(),
            height: doc.height(),
            replaced,
        }],
        skipped: Vec::new(),
        notes,
    }))
}
