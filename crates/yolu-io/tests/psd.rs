use std::io::{Cursor, Read};
use yolu_io::psd::{
    self, Adjustment, BlendMode, CompatibilityMode as Mode, Document, Layer, LayerKind, Limits,
    Mask,
};
fn i32le(r: &mut Cursor<Vec<u8>>) -> i32 {
    let mut b = [0; 4];
    r.read_exact(&mut b).unwrap();
    i32::from_le_bytes(b)
}
fn blob(r: &mut Cursor<Vec<u8>>) -> Option<Vec<u8>> {
    let n = i32le(r);
    if n < 0 {
        return None;
    }
    let mut b = vec![0; n as usize];
    r.read_exact(&mut b).unwrap();
    Some(b)
}
fn text(r: &mut Cursor<Vec<u8>>) -> String {
    String::from_utf8(blob(r).unwrap()).unwrap()
}
#[test]
fn csharp_corpus_modes_preservation_and_rewritten_bytes() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(include_bytes!("fixtures/psd/csharp.bin.gz").as_slice())
        .read_to_end(&mut bytes)
        .unwrap();
    let mut r = Cursor::new(bytes);
    let mut count = 0;
    let mut written = 0;
    let mut failures = Vec::new();
    while (r.position() as usize) < r.get_ref().len() {
        let label = text(&mut r);
        let bytes = blob(&mut r).unwrap();
        let mode =
            [Mode::EditableRaster, Mode::PreserveOnly, Mode::Rejected][i32le(&mut r) as usize];
        let mut n = [0u64; 10];
        for v in &mut n {
            let mut b = [0; 8];
            r.read_exact(&mut b).unwrap();
            *v = u64::from_le_bytes(b)
        }
        let limits = Limits {
            max_source_bytes: n[0] as usize,
            max_output_bytes: n[1] as usize,
            max_dimension: n[2] as u32,
            max_canvas_pixels: n[3],
            max_layers: n[4] as usize,
            max_decoded_bytes: n[5],
            max_metadata_bytes: n[6] as usize,
            max_name_code_units: n[7] as usize,
            max_diagnostics: n[8] as usize,
            max_group_depth: n[9] as usize,
        };
        let mut original = [0];
        r.read_exact(&mut original).unwrap();
        let codes = text(&mut r);
        let expected = blob(&mut r);
        count += 1;
        let actual = psd::read(&bytes, &limits).unwrap();
        if actual.mode() != mode {
            failures.push(format!(
                "#{count} {label}: {:?} != {mode:?}, C#={codes}, Rust={:?}",
                actual.mode(),
                actual.diagnostics()
            ));
            continue;
        }
        if mode != Mode::Rejected {
            let actual_codes: std::collections::BTreeSet<_> = actual
                .diagnostics()
                .iter()
                .map(|d| d.code.as_str())
                .collect();
            let expected_codes: std::collections::BTreeSet<_> =
                codes.split(',').filter(|s| !s.is_empty()).collect();
            if actual_codes != expected_codes {
                failures.push(format!(
                    "#{count} {label}: 診断コード不一致 {actual_codes:?} != {expected_codes:?}"
                ));
            }
        }
        assert_eq!(
            actual.original_bytes(),
            if original[0] != 0 {
                Some(bytes.as_slice())
            } else {
                None
            },
            "{label}"
        );
        if mode != Mode::EditableRaster {
            assert!(actual.document().is_none());
            assert!(psd::write_edited(&actual, &sample(), &limits).is_err())
        } else if let Some(expected) = expected {
            match psd::write_edited(&actual, actual.document().unwrap(), &limits) {
                Ok(actual) => {
                    if actual != expected {
                        let diff = actual.iter().zip(&expected).position(|(a, b)| a != b);
                        failures.push(format!(
                            "#{count} {label}: 書き戻し不一致 {diff:?} 長さ {}/{}",
                            actual.len(),
                            expected.len()
                        ))
                    } else {
                        written += 1
                    }
                }
                Err(e) => failures.push(format!("#{count} {label}: 書き戻し失敗 {e}")),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{}件不一致:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(count, 2337);
    assert_eq!(written, 481);
}
fn sample() -> Document {
    Document {
        width: 2,
        height: 2,
        layers: vec![Layer {
            id: 7,
            name: "色 🎨".into(),
            width: 2,
            height: 2,
            pixels_rgba: vec![
                255, 0, 0, 255, 0, 255, 0, 128, 12, 34, 56, 0, 20, 40, 60, 80,
            ],
            ..Layer::default()
        }],
        composite_rgba: None,
    }
}
#[test]
fn new_document_round_trip_and_original_copy_are_independent() {
    let d = sample();
    let mut bytes = psd::write(&d, &Limits::default()).unwrap();
    let r = psd::read(&bytes, &Limits::default()).unwrap();
    assert_eq!(r.document(), Some(&d));
    let original = bytes.clone();
    bytes.fill(0);
    let mut copy = r.copy_original_bytes().unwrap();
    copy.fill(0);
    assert_eq!(r.original_bytes(), Some(original.as_slice()));
    assert_eq!(
        psd::write_edited(&r, &d, &Limits::default()).unwrap(),
        original
    );
}
#[test]
fn bounded_stream_keeps_original_and_stops_at_cap_plus_one() {
    let b = psd::write(&sample(), &Limits::default()).unwrap();
    let mut stream = Cursor::new(&b);
    let r = psd::read_stream(&mut stream, &Limits::default()).unwrap();
    assert_eq!(r.mode(), Mode::EditableRaster);
    stream.set_position(0);
    let l = Limits {
        max_source_bytes: 26,
        ..Limits::default()
    };
    let r = psd::read_stream(&mut stream, &l).unwrap();
    assert_eq!(r.mode(), Mode::Rejected);
    assert!(r.original_bytes().is_none());
    assert_eq!(stream.position(), 27);
}
#[test]
fn core_preserves_ids_order_hidden_rgb_and_has_no_import_undo() {
    let mut d = sample();
    let mut top = d.layers[0].clone();
    top.id = 88;
    top.name = d.layers[0].name.clone();
    top.visible = false;
    top.opacity = 101;
    top.blend_mode = BlendMode::Multiply;
    top.clipping = true;
    d.layers.insert(0, top);
    let mut core = d.to_core().unwrap();
    assert!(!core.undo().unwrap());
    let export = Document::from_core(&core).unwrap();
    assert_eq!(
        export.layers.iter().map(|l| l.id).collect::<Vec<_>>(),
        vec![88, 7]
    );
    for (a, b) in d.layers.iter().zip(&export.layers) {
        assert_eq!(a, b)
    }
    assert_eq!(
        core.layers()[0]
            .pixel(yolu_core::Channel::Color, 0, 0)
            .unwrap()
            .to_array(),
        [12, 34, 56, 0]
    );
}
#[test]
fn every_m1_blend_and_clipping_round_trips() {
    for mode in BlendMode::ALL.into_iter().take(26) {
        let mut d = sample();
        let mut l = d.layers[0].clone();
        l.id = 9;
        l.blend_mode = mode;
        l.opacity = 183;
        l.clipping = true;
        d.layers.insert(0, l);
        let core = d.to_core().unwrap();
        let projected = Document::from_core(&core).unwrap();
        let bytes = psd::write(&projected, &Limits::default()).unwrap();
        let read = psd::read(&bytes, &Limits::default()).unwrap();
        assert_eq!(
            read.mode(),
            Mode::EditableRaster,
            "{mode:?}: {:?}",
            read.diagnostics()
        );
        let reread = read.to_core().unwrap();
        assert_eq!(
            core.composite(core.bounds()).unwrap(),
            reread.composite(reread.bounds()).unwrap()
        );
    }
}
#[test]
fn m2_layers_are_carried_into_core_and_off_canvas_pixels_are_refused_even_when_hidden() {
    // グループ・調整・塗りつぶし・マスク・ロックは core に入る（往復の試験は psd_m2.rs）
    let variants = [
        LayerKind::Group {
            children: vec![],
            divider_id: 0,
        },
        LayerKind::Adjustment(Adjustment::Invert),
        LayerKind::SolidColor([20, 40, 60]),
    ];
    for kind in variants {
        let mut d = sample();
        d.layers[0].kind = kind;
        d.layers[0].visible = false;
        assert!(d.core_issues().is_empty());
        d.to_core().unwrap();
    }
    let mut d = sample();
    d.layers[0].mask = Some(Mask {
        left: 0,
        top: 0,
        width: 1,
        height: 1,
        default_color: 255,
        enabled: false,
        density: 255,
        pixels: vec![255],
    });
    d.to_core().unwrap();
    d.layers[0].mask = None;
    d.layers[0].locks = 1;
    d.to_core().unwrap();
    d.layers[0].locks = 0;
    d.layers[0].left = -1;
    assert!(d.to_core().is_err());
    d.layers[0].visible = false;
    assert_eq!(d.core_issues().len(), 1, "隠していても切り捨てない");
}
#[test]
fn write_refuses_invalid_structure_limits_and_adjustments() {
    let d = sample();
    for limits in [
        Limits {
            max_output_bytes: 26,
            ..Limits::default()
        },
        Limits {
            max_decoded_bytes: 4,
            ..Limits::default()
        },
        Limits {
            max_metadata_bytes: 0,
            ..Limits::default()
        },
        Limits {
            max_name_code_units: 1,
            ..Limits::default()
        },
        Limits {
            max_dimension: 1,
            ..Limits::default()
        },
    ] {
        assert!(psd::write(&d, &limits).is_err())
    }
    let mut bad = d.clone();
    bad.layers.push(bad.layers[0].clone());
    assert!(psd::write(&bad, &Limits::default()).is_err());
    bad = d.clone();
    bad.layers[0].kind = LayerKind::Adjustment(Adjustment::Levels {
        input_black: 1,
        input_white: 0,
        output_black: 0,
        output_white: 255,
        gamma: 100,
    });
    assert!(psd::write(&bad, &Limits::default()).is_err());
    bad = d.clone();
    bad.layers[0].left = i32::MAX;
    assert!(psd::write(&bad, &Limits::default()).is_err());
    bad = d.clone();
    bad.layers[0].pixels_rgba.pop();
    assert!(psd::write(&bad, &Limits::default()).is_err());
}
#[test]
fn arbitrary_truncations_and_unknown_psb_preserve_without_panics() {
    let b = psd::write(&sample(), &Limits::default()).unwrap();
    for n in 0..b.len() {
        let r = psd::read(&b[..n], &Limits::default()).unwrap();
        assert_ne!(r.mode(), Mode::EditableRaster, "{n}");
        assert_eq!(r.original_bytes(), Some(&b[..n]));
    }
    let mut psb = b;
    psb[5] = 2;
    let r = psd::read(&psb, &Limits::default()).unwrap();
    assert_eq!(r.mode(), Mode::PreserveOnly);
    assert!(r.to_core().is_err());
    assert!(psd::write_edited(&r, &sample(), &Limits::default()).is_err());
}

#[test]
fn sparse_edge_tiles_empty_layer_and_id_collisions_round_trip() {
    use yolu_core::{Document as Core, LayerId, Rgba8};
    let mut core = Core::with_tile_size(17, 11, 8).unwrap();
    let empty = core.add_layer("同名").unwrap();
    let edge = core.add_layer("同名").unwrap();
    core.set_pixel(edge, 16, 10, Rgba8::new(17, 29, 43, 0))
        .unwrap();
    core.set_layer_opacity(empty, 0.5, false).unwrap();
    let core = core
        .with_persistent_ids(10, &[LayerId(1), LayerId(2)])
        .unwrap();
    let psd = Document::from_core(&core).unwrap();
    assert_eq!(
        psd.layers.iter().map(|l| l.id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        (
            psd.layers[0].left,
            psd.layers[0].top,
            psd.layers[0].width,
            psd.layers[0].height
        ),
        (16, 0, 1, 3)
    );
    assert_eq!(
        (
            psd.layers[1].width,
            psd.layers[1].height,
            psd.layers[1].opacity
        ),
        (1, 1, 128)
    );
    let imported = psd.to_core().unwrap();
    assert_eq!(
        imported.layers()[1]
            .surface(yolu_core::Channel::Color)
            .unwrap()
            .to_canvas_bytes(),
        core.layers()[1]
            .surface(yolu_core::Channel::Color)
            .unwrap()
            .to_canvas_bytes()
    );
    let second = Document::from_core(&imported).unwrap();
    assert_eq!(
        second.layers.iter().map(|l| l.id).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn active_stroke_refuses_export_until_cancelled() {
    let mut core = sample().to_core().unwrap();
    let stroke = core
        .begin_stroke(core.layers()[0].id(), &yolu_core::BrushSettings::default())
        .unwrap();
    assert!(Document::from_core(&core).is_err());
    core.cancel_stroke(stroke);
    assert!(Document::from_core(&core).is_ok());
}

#[test]
fn invalid_limits_and_large_dimensions_fail_before_allocating_pixels() {
    for limits in [
        Limits {
            max_source_bytes: 25,
            ..Limits::default()
        },
        Limits {
            max_group_depth: 1001,
            ..Limits::default()
        },
        Limits {
            max_diagnostics: 0,
            ..Limits::default()
        },
        Limits {
            max_layers: 32768,
            ..Limits::default()
        },
        Limits {
            max_dimension: 30001,
            ..Limits::default()
        },
    ] {
        assert!(psd::read(&[], &limits).is_err());
        assert!(psd::write(&sample(), &limits).is_err());
    }
    let mut d = sample();
    d.width = 8192;
    d.height = 8192;
    assert!(psd::write(&d, &Limits::default()).is_err());
    let mut bytes = psd::write(&sample(), &Limits::default()).unwrap();
    bytes[14..18].copy_from_slice(&8192u32.to_be_bytes());
    bytes[18..22].copy_from_slice(&8192u32.to_be_bytes());
    assert_eq!(
        psd::read(&bytes, &Limits::default()).unwrap().mode(),
        Mode::Rejected
    );
}
