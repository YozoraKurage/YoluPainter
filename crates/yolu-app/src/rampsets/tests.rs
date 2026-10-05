use super::*;
use std::path::PathBuf;
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::generator::{ColorStop, LuminanceCorrection, MixMode, OpacityStop};
use yolu_core::Rgba8;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-rampsets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn curve(points: &[(f64, f64)]) -> Curve {
    Curve::new(points.iter().map(|&(x, y)| CurvePoint { x, y }).collect()).unwrap()
}

fn sample() -> Ramp {
    let stop = |position, rgb: [u8; 3], midpoint| ColorStop {
        position,
        color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
        midpoint,
    };
    Ramp::new(
        vec![
            stop(0.0, [10, 20, 90], 0.4),
            stop(0.3515625, [200, 60, 30], 0.62),
            stop(1.0, [250, 240, 200], 0.5),
        ],
        vec![
            OpacityStop {
                position: 0.0,
                opacity: 1.0,
                midpoint: 0.5,
            },
            OpacityStop {
                position: 1.0,
                opacity: 0.25,
                midpoint: 0.5,
            },
        ],
        Some(
            curve(&[(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)])
                .points()
                .to_vec(),
        ),
    )
    .unwrap()
    .with_segment_curve(1, Some(curve(&[(0.0, 0.0), (0.25, 0.1), (1.0, 1.0)])))
    .unwrap()
}

#[test]
fn every_built_in_group_has_valid_distinct_named_ramps_in_both_languages() {
    let groups = builtin::groups([0.2, 0.4, 0.6, 1.0], [0.9, 0.8, 0.1, 1.0]);
    assert!(groups.len() >= 4, "色味の違う組が数組");
    assert_eq!(groups.len() + 1, RampSets::group_count());
    let mut names = std::collections::BTreeSet::new();
    for g in &groups {
        assert!(!g.items.is_empty());
        assert_ne!(g.ja, g.en);
        for item in &g.items {
            assert!(names.insert(item.ja), "{} が重なる", item.ja);
            assert!(!item.ja.is_empty() && !item.en.is_empty());
            assert!(item.ja.chars().count() <= 12, "{}", item.ja);
            // 組み込みは混色なし・PSD に書ける（位置と中点が刻みに乗る）
            assert!(!item.ramp.uses_mixing());
            for c in item.ramp.colors() {
                assert!(
                    (c.position * 4096.0).fract().abs() < 1e-9 || c.position == 0.0,
                    "{}",
                    item.ja
                );
                assert_eq!(c.midpoint, 0.5);
            }
        }
    }
    // 「基本」のメイン→サブは描画色から作る
    let basic = &groups[0];
    let from = basic.items.iter().find(|i| i.en == "Main to Sub").unwrap();
    assert_eq!(from.ramp.colors()[0].color, Rgba8::new(51, 102, 153, 255));
    assert_eq!(from.ramp.colors()[1].color, Rgba8::new(230, 204, 26, 255));
}

#[test]
fn a_saved_set_reads_back_exactly_and_the_file_form_is_stable() {
    let sets = vec![
        UserRamp {
            name: "夕空 = 1".into(),
            ramp: sample(),
        },
        UserRamp {
            name: "plain".into(),
            ramp: Ramp::default(),
        },
    ];
    let text = store::encode(&sets);
    assert!(text.starts_with("yolupainter-gradients 1\n"));
    assert!(text.contains("ramp.1.colors=0:0a145a:0.4;0.3515625:c83c1e:0.62;1:faf0c8:0.5\n"));
    assert!(text.contains("ramp.1.segment.1=0:0;0.25:0.1;1:1\n"));
    assert!(
        !text.contains("ramp.1.segment.0"),
        "曲線の無い区間は書かない"
    );
    assert!(!text.contains("ramp.2.curve"), "直線の値のカーブは書かない");
    assert_eq!(store::decode(&text).unwrap(), sets);
    // 書き直しても同じ文
    assert_eq!(store::encode(&store::decode(&text).unwrap()), text);
}

#[test]
fn mixing_is_not_part_of_a_set_and_the_segment_curves_are() {
    let dir = temp("mixing");
    let mut sets = RampSets::default();
    sets.attach(dir.clone());
    let mixed = sample().with_mixing(MixMode::Perceptual, LuminanceCorrection::Max);
    let at = sets.add("a", &mixed, Lang::Ja).unwrap();
    let kept = &sets.user()[at].ramp;
    assert_eq!(kept.mix_mode(), MixMode::Standard);
    assert!(kept.segment_curve(1).is_some());
    let mut again = RampSets::default();
    again.attach(dir.clone());
    assert_eq!(again.user(), sets.user());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_files_are_refused_with_the_reason() {
    let head = "yolupainter-gradients 1\n";
    let good = "ramp.1.name=a\nramp.1.colors=0:000000:0.5;1:ffffff:0.5\nramp.1.opacities=0:1:0.5;1:1:0.5\n";
    assert_eq!(store::decode(&format!("{head}{good}")).unwrap().len(), 1);
    let cases: Vec<(String, &str)> = vec![
        ("hello\n".into(), "NotGradients"),
        ("yolupainter-gradients 2\n".into(), "NewerVersion"),
        (format!("{head}ramp.1.name\n"), "Syntax"),
        (format!("{head}colour.1.name=a\n"), "UnknownKey"),
        (format!("{head}ramp.0.name=a\n"), "UnknownKey"),
        (format!("{head}{good}ramp.1.name=b\n"), "DuplicateKey"),
        (format!("{head}{good}ramp.1.size=3\n"), "UnknownKey"),
        (format!("{head}ramp.1.name=a\n"), "BadValue"),
        (format!("{head}ramp.1.name=\x01 \nramp.1.colors=0:000000:0.5;1:ffffff:0.5\nramp.1.opacities=0:1:0.5;1:1:0.5\n"), "BadValue"),
        // 色
        (format!("{head}ramp.1.name=a\nramp.1.colors=0:00000:0.5;1:ffffff:0.5\nramp.1.opacities=0:1:0.5;1:1:0.5\n"), "BadValue"),
        (format!("{head}ramp.1.name=a\nramp.1.colors=0:gg0000:0.5;1:ffffff:0.5\nramp.1.opacities=0:1:0.5;1:1:0.5\n"), "BadValue"),
        (format!("{head}ramp.1.name=a\nramp.1.colors=0:000000;1:ffffff:0.5\nramp.1.opacities=0:1:0.5;1:1:0.5\n"), "BadValue"),
        (format!("{head}ramp.1.name=a\nramp.1.colors=0:000000:0.5\nramp.1.opacities=0:1:0.5;1:1:0.5\n"), "BadValue"),
        (format!("{head}ramp.1.name=a\nramp.1.colors=0:000000:NaN;1:ffffff:0.5\nramp.1.opacities=0:1:0.5;1:1:0.5\n"), "BadValue"),
        (format!("{head}ramp.1.name=a\nramp.1.colors=0:000000:0.5;2:ffffff:0.5\nramp.1.opacities=0:1:0.5;1:1:0.5\n"), "BadValue"),
        (format!("{head}ramp.1.name=a\nramp.1.colors=0:000000:0.5;0:ffffff:0.5\nramp.1.opacities=0:1:0.5;1:1:0.5\n"), "BadValue"),
        (format!("{head}ramp.1.name=a\nramp.1.colors=0:000000:0.5;1:ffffff:0\nramp.1.opacities=0:1:0.5;1:1:0.5\n"), "BadValue"),
        // 不透明度・カーブ・区間
        (format!("{head}ramp.1.name=a\nramp.1.colors=0:000000:0.5;1:ffffff:0.5\nramp.1.opacities=0:1.5:0.5;1:1:0.5\n"), "BadValue"),
        (format!("{head}{good}ramp.1.curve=0:0;0.01:0.5;1:1\n"), "BadValue"),
        (format!("{head}{good}ramp.1.curve=0:0;1\n"), "BadValue"),
        (format!("{head}{good}ramp.1.segment.1=0:0;1:1\n"), "BadValue"),
        (format!("{head}{good}ramp.1.segment.x=0:0;1:1\n"), "UnknownKey"),
        (format!("{head}{good}ramp.1.segment.0=0:0;1:1\nramp.1.segment.0=0:0;1:1\n"), "DuplicateKey"),
    ];
    for (text, kind) in cases {
        let error = store::decode(&text).unwrap_err();
        assert!(
            format!("{error:?}").starts_with(kind),
            "{text:?} -> {error:?}"
        );
        assert!(!error.describe(Lang::Ja).is_empty() && !error.describe(Lang::En).is_empty());
    }
    let many: String = (1..=MAX_USER as u32 + 1)
        .map(|n| {
            format!(
                "ramp.{n}.name=p\nramp.{n}.colors=0:000000:0.5;1:ffffff:0.5\nramp.{n}.opacities=0:1:0.5;1:1:0.5\n"
            )
        })
        .collect();
    assert!(matches!(
        store::decode(&format!("{head}{many}")),
        Err(StoreError::TooMany)
    ));
}

/// 一番長い書き方のランプ（色 32・不透明度 32・値のカーブ 16 点・全区間 31 本の混合率曲線が 16 点、長い小数）。
fn longest() -> Ramp {
    let third = 1.0 / 3.0;
    let colors = (0..32)
        .map(|i| ColorStop {
            position: f64::from(i) / 31.0 * 0.999_999_9,
            color: Rgba8::new(255, 254, 253, 255),
            midpoint: 0.123_456_789_012_345_67 + f64::from(i) * 1e-3,
        })
        .collect();
    let opacities = (0..32)
        .map(|i| OpacityStop {
            position: f64::from(i) / 31.0 * 0.999_999_9,
            opacity: third,
            midpoint: 0.234_567_890_123_456_7,
        })
        .collect();
    let points: Vec<CurvePoint> = (0..16)
        .map(|i| CurvePoint {
            x: if i == 15 { 1.0 } else { f64::from(i) / 15.0 },
            y: third + f64::from(i) * 1e-9,
        })
        .collect();
    let wavy = Curve::new(points.clone()).unwrap();
    Ramp::new(colors, opacities, Some(points))
        .unwrap()
        .with_segment_curves(vec![Some(wavy); 31])
        .unwrap()
}

#[test]
fn the_largest_number_of_longest_gradients_is_saved_and_read_back() {
    let dir = temp("largest");
    let store = RampSetStore::new(dir.clone());
    let sets: Vec<UserRamp> = (0..MAX_USER)
        .map(|_| UserRamp {
            name: "\u{20BB7}".repeat(crate::brushes::MAX_NAME_CHARS),
            ramp: longest(),
        })
        .collect();
    store
        .save(&sets)
        .unwrap_or_else(|e| panic!("{MAX_USER} 個を保存できない: {e:?}"));
    let size = std::fs::metadata(store.path()).unwrap().len();
    assert!(size <= store::MAX_FILE_BYTES, "{size}");
    assert_eq!(store.load().unwrap(), sets, "読み戻した中身が違う");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_file_that_is_too_large_or_newer_is_not_read_and_not_overwritten() {
    let dir = temp("unreadable");
    let path = dir.join(store::FILE_NAME);
    std::fs::write(&path, "yolupainter-gradients 9\nfuture=1\n").unwrap();
    let mut sets = RampSets::default();
    sets.attach(dir.clone());
    assert!(matches!(sets.problem(), Some(StoreError::NewerVersion(_))));
    assert!(sets.user().is_empty());
    // 読めなかったファイルは触らず、足す・名前を変える・消すは理由つきで断る（一覧にだけ残して再起動で黙って消えることがない）
    let refused = sets.add("a", &sample(), Lang::Ja).unwrap_err();
    assert!(matches!(refused, SetsError::Unreadable(..)), "{refused:?}");
    assert!(sets.user().is_empty());
    assert!(matches!(sets.rename(0, "b"), Err(SetsError::Index)));
    assert!(matches!(sets.remove(0), Err(SetsError::Index)));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "yolupainter-gradients 9\nfuture=1\n"
    );
    let ja = refused.describe(Lang::Ja);
    let en = refused.describe(Lang::En);
    assert!(
        ja.contains("保存できません") && ja.contains("新しい形式です（9）"),
        "{ja}"
    );
    assert!(
        en.contains("Cannot save") && en.contains("A newer format (9)"),
        "{en}"
    );
    std::fs::write(&path, vec![b'x'; store::MAX_FILE_BYTES as usize + 1]).unwrap();
    let mut big = RampSets::default();
    big.attach(dir.clone());
    assert!(matches!(big.problem(), Some(StoreError::TooLarge)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn adding_renaming_and_removing_are_saved_and_the_last_removal_deletes_the_file() {
    let dir = temp("edit");
    let mut sets = RampSets::default();
    sets.attach(dir.clone());
    assert!(sets.problem().is_none());
    assert_eq!(sets.add("", &sample(), Lang::Ja).unwrap(), 0);
    assert_eq!(sets.user()[0].name, "グラデーション 1", "名前が空なら番号");
    assert_eq!(sets.add("  夕空  ", &Ramp::default(), Lang::En).unwrap(), 1);
    assert_eq!(sets.user()[1].name, "夕空");
    sets.rename(0, "朝").unwrap();
    assert!(matches!(sets.rename(0, "   "), Err(SetsError::Name)));
    assert!(matches!(sets.rename(9, "x"), Err(SetsError::Index)));
    let mut again = RampSets::default();
    again.attach(dir.clone());
    assert_eq!(
        again
            .user()
            .iter()
            .map(|u| u.name.as_str())
            .collect::<Vec<_>>(),
        ["朝", "夕空"]
    );
    sets.remove(0).unwrap();
    assert!(matches!(sets.remove(5), Err(SetsError::Index)));
    sets.remove(0).unwrap();
    assert!(
        !dir.join(store::FILE_NAME).exists(),
        "1 つも無ければファイルを消す"
    );
    let mut empty = RampSets::default();
    empty.attach(dir.clone());
    assert!(empty.user().is_empty() && empty.problem().is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_limit_stops_adding_and_a_failed_save_leaves_the_list_as_it_was() {
    let dir = temp("limit");
    let mut sets = RampSets::default();
    sets.attach(dir.clone());
    for _ in 0..MAX_USER {
        sets.add("g", &Ramp::default(), Lang::Ja).unwrap();
    }
    assert!(matches!(
        sets.add("one more", &Ramp::default(), Lang::Ja),
        Err(SetsError::TooMany)
    ));
    assert_eq!(sets.user().len(), MAX_USER);
    // 保存できない場所（フォルダの代わりにファイルがある）: 足せず、名前も消すのも元のまま
    let blocked = temp("blocked");
    let file = blocked.join("gradients");
    std::fs::write(&file, "x").unwrap();
    let mut stuck = RampSets {
        store: Some(RampSetStore::new(file.clone())),
        ..Default::default()
    };
    assert!(stuck.add("a", &sample(), Lang::Ja).is_err());
    assert!(stuck.user().is_empty());
    let mut kept = RampSets::default();
    kept.attach(dir.clone());
    // 保存先を壊して名前の変更・削除も失敗する
    kept.store = Some(RampSetStore::new(file));
    assert!(kept.rename(0, "new").is_err());
    assert_eq!(kept.user()[0].name, "g");
    assert!(kept.remove(0).is_err());
    assert_eq!(kept.user().len(), MAX_USER);
    for lang in Lang::ALL {
        assert!(!SetsError::TooMany.describe(lang).is_empty());
        assert!(!SetsError::Name.describe(lang).is_empty());
        assert!(!SetsError::Index.describe(lang).is_empty());
    }
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(blocked);
}

#[test]
fn groups_list_their_entries_and_the_user_group_comes_last() {
    let mut sets = RampSets::default();
    let main = [0.0, 0.0, 0.0, 1.0];
    let sub = [1.0, 1.0, 1.0, 1.0];
    assert_eq!(RampSets::user_group(), RampSets::group_count() - 1);
    let basic = sets.entries(Lang::Ja, main, sub);
    assert_eq!(basic[0].name, "黒→白");
    assert_eq!(sets.entries(Lang::En, main, sub)[0].name, "Black to White");
    assert!(!sets.showing_user());
    sets.show_group(RampSets::user_group());
    assert!(sets.showing_user());
    assert!(sets.entries(Lang::Ja, main, sub).is_empty());
    sets.add("mine", &sample(), Lang::Ja).unwrap();
    assert_eq!(sets.entries(Lang::Ja, main, sub)[0].name, "mine");
    sets.selected = Some(0);
    sets.show_group(1);
    assert_eq!(sets.selected, None, "組を替えたら選びは外す");
    sets.show_group(99);
    assert_eq!(sets.group, RampSets::user_group(), "範囲外は最後の組");
    for g in 0..RampSets::group_count() {
        assert!(!RampSets::group_name(Lang::Ja, g).is_empty());
        assert!(!RampSets::group_name(Lang::En, g).is_empty());
    }
}
