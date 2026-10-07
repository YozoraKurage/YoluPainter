//! ウィンドウの置き場所（`windowpos`）の試験。画面の拡大率が違う・画面が増減する・縮む場合の、画素での置き場所と、起動のあとの合わせ込み・
//! 最大化の 1 画素。Windows の API に依らない純関数なので、Linux でも回る（Windows の画面の列挙・ウィンドウの手は `cargo check
//! --target x86_64-pc-windows-gnu` で組めることと、実機の確かめで見る）。
//! 覚えた置き場所（`windowpos::remember`）はプロセスで 1 つなので、これを使う試験があるこのファイルは束に入れず、直下に 1 本で置く。
use egui::{pos2, vec2, ViewportCommand};
use yolu_app::layout::WindowRecord;
use yolu_app::windowpos::{
    maximized_client, plan, Actual, Edges, Monitor, Placement, PxRect, Settle, DEFAULT_SIZE,
};

/// 1920×1080・100%・主の画面。作業領域は下にタスクバー（40 画素）を除いた所。
fn left() -> Monitor {
    Monitor {
        bounds: PxRect::from_origin_size(0, 0, 1920, 1080),
        work: PxRect::from_origin_size(0, 0, 1920, 1040),
        scale: 1.0,
        primary: true,
    }
}

/// 論理 1920×1080・150%（2880×1620）・右隣。
fn right() -> Monitor {
    Monitor {
        bounds: PxRect::from_origin_size(1920, 0, 2880, 1620),
        work: PxRect::from_origin_size(1920, 0, 2880, 1580),
        scale: 1.5,
        primary: false,
    }
}

/// 主の画面の左隣（座標が負）・100%。
fn farleft() -> Monitor {
    Monitor {
        bounds: PxRect::from_origin_size(-1920, 0, 1920, 1080),
        work: PxRect::from_origin_size(-1920, 0, 1920, 1080),
        scale: 1.0,
        primary: false,
    }
}

/// ウィンドウの記録（点。画素 = 点 × `scale`）を、画素の位置・大きさから作る。
fn record(px: PxRect, scale: f32, maximized: bool) -> WindowRecord {
    WindowRecord {
        position: [px.left as f32 / scale, px.top as f32 / scale],
        size: [px.width() as f32 / scale, px.height() as f32 / scale],
        pixels_per_point: scale,
        maximized,
    }
}

fn inside(rect: PxRect, area: PxRect) -> bool {
    rect.left >= area.left
        && rect.top >= area.top
        && rect.right <= area.right
        && rect.bottom <= area.bottom
}

// ───────── 起動の置き場所 ─────────

#[test]
fn a_window_recorded_on_a_scaled_second_screen_comes_back_there_in_physical_pixels() {
    // 150% の右の画面（仮想スクリーンの x=2220）にいたウィンドウ。記録の点は 1480.0・66.666664（画素 ÷ 1.5）。
    // 点を主の画面の拡大率（100%）で画素に直すと x=1480 になり、左の画面に出てしまう（点は画面をまたいで意味を持たない）
    let saved = record(PxRect::from_origin_size(2220, 100, 2100, 1350), 1.5, false);
    assert!((saved.position[0] - 1480.0).abs() < 1e-3);
    let naive_x = (saved.position[0] * left().scale).round() as i32;
    assert!(
        naive_x < right().bounds.left,
        "点を主の画面の拡大率で画素に直す従来の置き方は、左の画面（x={naive_x}）に出る"
    );
    let place = plan(Some(&saved), &[left(), right()]).unwrap();
    assert_eq!(place.rect, PxRect::from_origin_size(2220, 100, 2100, 1350));
    assert_eq!(place.scale, 1.5);
    assert!(inside(place.rect, right().work));
    // ウィンドウを作るときの論理の大きさは、置く画面の拡大率で割った値（記録の点のまま）
    let [w, h] = place.size_points();
    assert!(
        (w - 1400.0).abs() < 0.01 && (h - 900.0).abs() < 0.01,
        "{w} {h}"
    );
    // 画面の並びの順に依らない
    assert_eq!(plan(Some(&saved), &[right(), left()]).unwrap(), place);
}

#[test]
fn a_window_on_a_screen_left_of_the_primary_keeps_its_negative_position() {
    let saved = record(PxRect::from_origin_size(-1500, 100, 1200, 800), 1.0, false);
    let place = plan(Some(&saved), &[left(), farleft()]).unwrap();
    assert_eq!(place.rect, PxRect::from_origin_size(-1500, 100, 1200, 800));
    assert_eq!(place.scale, 1.0);
}

#[test]
fn when_the_recorded_screen_is_gone_the_window_is_centered_on_the_primary_screen() {
    let saved = record(PxRect::from_origin_size(2220, 100, 2100, 1350), 1.5, true);
    // 右の画面が無くなった。記録の点（1400×900）の大きさのまま主の画面の作業領域の中央へ。最大化の記録は引き継ぐ
    let place = plan(Some(&saved), &[left()]).unwrap();
    assert_eq!(place.rect, PxRect::from_origin_size(260, 70, 1400, 900));
    assert_eq!(place.scale, 1.0);
    assert!(place.maximized);
}

#[test]
fn a_title_bar_hidden_behind_the_taskbar_or_off_every_screen_is_not_visible() {
    // 上の帯がタスクバーの下（画面の中だが作業領域の外）。画面全体で見ると見えているが、つかめない
    let behind = record(PxRect::from_origin_size(300, 1050, 1000, 700), 1.0, false);
    let place = plan(Some(&behind), &[left()]).unwrap();
    assert_eq!(
        place.rect,
        PxRect::from_origin_size(460, 170, 1000, 700),
        "主の画面の作業領域の中央へ"
    );
    // 右へ大きく外れた
    let away = record(PxRect::from_origin_size(9000, 100, 1200, 800), 1.0, false);
    let place = plan(Some(&away), &[left()]).unwrap();
    assert!(inside(place.rect, left().work), "{place:?}");
}

#[test]
fn a_shrunk_screen_pulls_the_window_inside_the_work_area_and_shrinks_it_to_fit() {
    let small = Monitor {
        bounds: PxRect::from_origin_size(0, 0, 1366, 768),
        work: PxRect::from_origin_size(0, 0, 1366, 728),
        scale: 1.0,
        primary: true,
    };
    // 1920×1080 のときの大きさと位置（1600×900 を (100, 80)）。1366×768 になった
    let saved = record(PxRect::from_origin_size(100, 80, 1600, 900), 1.0, false);
    let place = plan(Some(&saved), &[small]).unwrap();
    assert_eq!(place.rect, PxRect::from_origin_size(0, 0, 1366, 728));
    // 下へはみ出すウィンドウも、作業領域に収まる所まで上げる
    let low = record(PxRect::from_origin_size(100, 900, 1200, 900), 1.0, false);
    let place = plan(Some(&low), &[left()]).unwrap();
    assert_eq!(place.rect, PxRect::from_origin_size(100, 140, 1200, 900));
}

#[test]
fn the_same_logical_size_gets_more_pixels_when_the_screen_scale_grew() {
    // 100% の画面で記録したウィンドウが、同じ場所の画面が 150% になって戻る。点（論理の大きさ）は変えず画素は増える
    let saved = record(PxRect::from_origin_size(0, 0, 1200, 800), 1.0, false);
    let grown = Monitor {
        bounds: PxRect::from_origin_size(0, 0, 2880, 1620),
        work: PxRect::from_origin_size(0, 0, 2880, 1560),
        scale: 1.5,
        primary: true,
    };
    let place = plan(Some(&saved), &[grown]).unwrap();
    assert_eq!(place.rect.width(), 1800);
    assert_eq!(place.rect.height(), 1200);
    assert!(inside(place.rect, grown.work));
}

#[test]
fn the_first_start_centers_a_default_size_window_in_the_primary_work_area() {
    let place = plan(None, &[left()]).unwrap();
    assert_eq!(DEFAULT_SIZE, [1600.0, 960.0]);
    assert_eq!(place.rect, PxRect::from_origin_size(160, 40, 1600, 960));
    assert!(!place.maximized);
    // 主の画面が一覧の先頭でなくても、主の画面
    assert_eq!(plan(None, &[right(), left()]).unwrap(), place);
    // 150% の主の画面: 論理の大きさ（1600×960）を画素にして、作業領域（2880×1580）の中央
    let scaled = Monitor {
        scale: 1.5,
        bounds: right().bounds,
        work: right().work,
        primary: true,
    };
    assert_eq!(
        plan(None, &[scaled]).unwrap().rect,
        PxRect::from_origin_size(1920 + 240, 70, 2400, 1440)
    );
}

#[test]
fn the_first_start_fits_a_small_work_area_without_going_below_the_minimum_size() {
    // 1366×768（作業領域 1366×728）: 既定の大きさ（1600×960）は作業領域に縮む
    let small = Monitor {
        bounds: PxRect::from_origin_size(0, 0, 1366, 768),
        work: PxRect::from_origin_size(0, 0, 1366, 728),
        scale: 1.0,
        primary: true,
    };
    let place = plan(None, &[small]).unwrap();
    assert_eq!(place.rect, PxRect::from_origin_size(0, 0, 1366, 728));
    // 作業領域がウィンドウの最小の大きさ（960×640）より小さい画面では、最小の大きさを割らない（OS が最小の大きさを守るので、目標もそれに合わせる）
    let tiny = Monitor {
        work: PxRect::from_origin_size(0, 0, 800, 560),
        ..small
    };
    let place = plan(None, &[tiny]).unwrap();
    assert_eq!((place.rect.width(), place.rect.height()), (960, 640));
    assert_eq!(
        (place.rect.left, place.rect.top),
        (0, 0),
        "作業領域の左上より外へは出さない"
    );
    // 記録のあるウィンドウでも、寄せるときに落ちない（最小の大きさが作業領域より大きい）
    let saved = record(PxRect::from_origin_size(200, 100, 1200, 800), 1.0, false);
    let place = plan(Some(&saved), &[tiny]).unwrap();
    assert_eq!((place.rect.left, place.rect.top), (0, 0));
}

#[test]
fn nothing_is_planned_when_the_screens_are_unknown() {
    assert_eq!(plan(None, &[]), None);
    let saved = record(PxRect::from_origin_size(0, 0, 1200, 800), 1.0, false);
    assert_eq!(plan(Some(&saved), &[]), None);
}

#[test]
fn a_maximized_record_stays_maximized_on_its_own_screen() {
    let saved = record(PxRect::from_origin_size(2220, 100, 2100, 1350), 1.5, true);
    let place = plan(Some(&saved), &[left(), right()]).unwrap();
    assert!(place.maximized);
    assert_eq!(
        place.scale, 1.5,
        "最大化は、置いた画面で行う（先に通常の矩形をその画面へ置く）"
    );
}

// ───────── 起動のあとの合わせ込み ─────────

fn target(rect: PxRect, scale: f32, maximized: bool) -> Placement {
    Placement {
        rect,
        scale,
        maximized,
    }
}

fn actual(position: [i32; 2], size: [i32; 2], scale: f32) -> Option<Actual> {
    Some(Actual {
        position,
        size,
        scale,
    })
}

#[test]
fn settling_moves_first_and_sizes_only_when_the_window_is_on_the_target_screen() {
    let want = target(PxRect::from_origin_size(2220, 100, 2100, 1350), 1.5, false);
    let mut settle = Settle::new(want);
    // 作ったのは主の画面（100%）。拡大率の違う画面へ動かすと OS がウィンドウの大きさを拡大率の比で変えるので、まず位置だけ
    let step = settle.step(actual([300, 200], [1400, 900], 1.0));
    assert_eq!(
        step.commands,
        vec![ViewportCommand::OuterPosition(pos2(2220.0, 100.0))]
    );
    assert!(step.settling);
    // 動いて 150% になり、OS が大きさを変えた（論理の大きさが同じ 1400×900 → 2100×1350 になるはずが、ずれた）。今度は大きさだけ、今の拡大率で
    let step = settle.step(actual([2220, 100], [2400, 1440], 1.5));
    assert_eq!(
        step.commands,
        vec![ViewportCommand::InnerSize(vec2(1400.0, 900.0))]
    );
    assert!(step.settling);
    // 合った
    let step = settle.step(actual([2220, 100], [2100, 1350], 1.5));
    assert!(step.commands.is_empty() && !step.settling);
}

#[test]
fn settling_asks_for_position_and_size_together_on_the_same_screen_scale() {
    let want = target(PxRect::from_origin_size(160, 40, 1400, 900), 1.0, false);
    let mut settle = Settle::new(want);
    let step = settle.step(actual([0, 0], [1600, 960], 1.0));
    assert_eq!(
        step.commands,
        vec![
            ViewportCommand::OuterPosition(pos2(160.0, 40.0)),
            ViewportCommand::InnerSize(vec2(1400.0, 900.0))
        ]
    );
    // 位置だけ合っていれば、大きさだけ
    let step = settle.step(actual([160, 40], [1600, 960], 1.0));
    assert_eq!(
        step.commands,
        vec![ViewportCommand::InnerSize(vec2(1400.0, 900.0))]
    );
}

#[test]
fn settling_converts_pixels_with_the_windows_current_scale() {
    // egui-winit は OuterPosition・InnerSize を「点 × 今の拡大率」で画素に直す。今の拡大率で割って渡せば画素が合う
    let want = target(PxRect::from_origin_size(1000, 500, 1500, 1000), 1.25, false);
    let mut settle = Settle::new(want);
    let step = settle.step(actual([0, 0], [800, 600], 1.25));
    assert_eq!(
        step.commands,
        vec![
            ViewportCommand::OuterPosition(pos2(800.0, 400.0)),
            ViewportCommand::InnerSize(vec2(1200.0, 800.0))
        ]
    );
}

#[test]
fn settling_tolerates_a_rounding_difference_but_not_more() {
    let want = target(PxRect::from_origin_size(100, 100, 1000, 700), 1.0, false);
    let mut settle = Settle::new(want);
    assert!(settle
        .step(actual([102, 98], [1002, 698], 1.0))
        .commands
        .is_empty());
    let step = settle.step(actual([103, 100], [1000, 700], 1.0));
    assert_eq!(
        step.commands,
        vec![ViewportCommand::OuterPosition(pos2(100.0, 100.0))]
    );
}

#[test]
fn settling_waits_for_the_window_information_and_gives_up_after_a_limit() {
    let want = target(PxRect::from_origin_size(100, 100, 1000, 700), 1.0, true);
    let mut settle = Settle::new(want);
    // ウィンドウの情報がまだ無い間は、何も頼まずに待つ
    let step = settle.step(None);
    assert!(step.commands.is_empty() && step.settling);
    // 合わないまま上限のフレームを超えたら、今のまま終わる（利用者が動かしたときに争い続けない）。最大化の記録は最後に頼む
    let mut last = step;
    for _ in 0..100 {
        last = settle.step(actual([500, 500], [1000, 700], 1.0));
        if !last.settling {
            break;
        }
    }
    assert!(!last.settling);
    assert_eq!(last.commands, vec![ViewportCommand::Maximized(true)]);
}

#[test]
fn a_maximized_record_is_maximized_after_the_rect_is_in_place() {
    let want = target(PxRect::from_origin_size(2220, 100, 2100, 1350), 1.5, true);
    let mut settle = Settle::new(want);
    let step = settle.step(actual([2220, 100], [2100, 1350], 1.5));
    assert_eq!(step.commands, vec![ViewportCommand::Maximized(true)]);
    assert!(!step.settling);
}

#[test]
fn the_actual_rect_comes_from_the_viewport_info_in_pixels() {
    let mut info = egui::ViewportInfo::default();
    assert_eq!(Actual::from_viewport(&info), None);
    info.native_pixels_per_point = Some(1.5);
    info.outer_rect = Some(egui::Rect::from_min_size(
        pos2(1480.0, 66.666664),
        vec2(1400.0, 900.0),
    ));
    assert_eq!(Actual::from_viewport(&info), None, "内側の矩形が無い");
    info.inner_rect = Some(egui::Rect::from_min_size(
        pos2(1480.0, 66.666664),
        vec2(1400.0, 900.0),
    ));
    assert_eq!(
        Actual::from_viewport(&info),
        actual([2220, 100], [2100, 1350], 1.5)
    );
}

// ───────── 自動で隠すタスクバーのための最大化の 1 画素 ─────────

#[test]
fn a_maximized_window_leaves_one_pixel_on_each_autohide_edge_only() {
    let work = PxRect::from_origin_size(0, 0, 1920, 1080);
    assert_eq!(maximized_client(work, Edges::default()), work);
    let bottom = Edges {
        bottom: true,
        ..Edges::default()
    };
    assert_eq!(
        maximized_client(work, bottom),
        PxRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1079
        }
    );
    let top = Edges {
        top: true,
        ..Edges::default()
    };
    assert_eq!(
        maximized_client(work, top),
        PxRect {
            left: 0,
            top: 1,
            right: 1920,
            bottom: 1080
        }
    );
    let left = Edges {
        left: true,
        ..Edges::default()
    };
    assert_eq!(
        maximized_client(work, left),
        PxRect {
            left: 1,
            top: 0,
            right: 1920,
            bottom: 1080
        }
    );
    let right = Edges {
        right: true,
        ..Edges::default()
    };
    assert_eq!(
        maximized_client(work, right),
        PxRect {
            left: 0,
            top: 0,
            right: 1919,
            bottom: 1080
        }
    );
    let all = Edges {
        left: true,
        top: true,
        right: true,
        bottom: true,
    };
    assert_eq!(
        maximized_client(work, all),
        PxRect {
            left: 1,
            top: 1,
            right: 1919,
            bottom: 1079
        }
    );
    assert!(all.any() && !Edges::default().any());
    // 別の画面（座標が負）でも、その画面の端から
    let far = PxRect::from_origin_size(-1920, 0, 1920, 1080);
    assert_eq!(
        maximized_client(far, bottom),
        PxRect {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1079
        }
    );
    // 壊れた小さな矩形は縮めない（裏返らない）
    let tiny = PxRect::from_origin_size(0, 0, 2, 2);
    assert_eq!(maximized_client(tiny, all), tiny);
}

// ───────── 起動の手続き（ウィンドウの情報から頼みを送る） ─────────

fn frame(ctx: &egui::Context, info: egui::ViewportInfo) -> (bool, Vec<ViewportCommand>) {
    let mut input = egui::RawInput::default();
    input.viewports.insert(egui::ViewportId::ROOT, info);
    let mut settling = false;
    let output = ctx.run_ui(input, |ui| settling = yolu_app::windowpos::settle(ui.ctx()));
    // （egui 自身が出す頼み（テーマなど）は除き、置き場所の頼みだけを見る）
    let commands = output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|v| v.commands.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|c| {
            matches!(
                c,
                ViewportCommand::OuterPosition(_)
                    | ViewportCommand::InnerSize(_)
                    | ViewportCommand::Maximized(_)
            )
        })
        .collect();
    output.drop_without_applying_deltas();
    (settling, commands)
}

fn info(position: [f32; 2], size: [f32; 2], scale: f32) -> egui::ViewportInfo {
    egui::ViewportInfo {
        native_pixels_per_point: Some(scale),
        outer_rect: Some(egui::Rect::from_min_size(
            pos2(position[0], position[1]),
            vec2(size[0], size[1]),
        )),
        inner_rect: Some(egui::Rect::from_min_size(
            pos2(position[0], position[1]),
            vec2(size[0], size[1]),
        )),
        minimized: Some(false),
        fullscreen: Some(false),
        ..Default::default()
    }
}

/// 覚えた置き場所を、フレームごとに合わせにいき、合ったら覚えを消す（この試験だけが、プロセスで 1 つの覚えを使う）。
#[test]
fn the_remembered_placement_is_driven_frame_by_frame_and_then_forgotten() {
    let ctx = egui::Context::default();
    // 覚えが無ければ何もしない（普通のフレーム・試験のウィンドウ）
    assert_eq!(
        frame(&ctx, info([0.0, 0.0], [800.0, 600.0], 1.0)),
        (false, vec![])
    );
    yolu_app::windowpos::remember(target(
        PxRect::from_origin_size(2220, 100, 2100, 1350),
        1.5,
        false,
    ));
    // 作ったウィンドウは主の画面（100%）の (300, 200)・1400×900 点。位置だけを頼む。その間は「合わせている」と返す
    let (settling, commands) = frame(&ctx, info([300.0, 200.0], [1400.0, 900.0], 1.0));
    assert!(settling);
    assert_eq!(
        commands,
        vec![ViewportCommand::OuterPosition(pos2(2220.0, 100.0))]
    );
    // 動いて 150% になった。大きさが合っていれば終わり、覚えは消える
    let (settling, commands) = frame(&ctx, info([1480.0, 66.666664], [1400.0, 900.0], 1.5));
    assert_eq!((settling, commands), (false, vec![]));
    assert_eq!(
        frame(&ctx, info([0.0, 0.0], [800.0, 600.0], 1.0)),
        (false, vec![]),
        "合わせ終えたあとは何もしない"
    );
}
