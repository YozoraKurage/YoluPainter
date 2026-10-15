//! パスのブラシの筆圧の切り替え（`PathBrush` の 2 つ目、`PressureSwitches`）: 正本の `brush` の塊の `pressure_size`・`pressure_opacity`・
//! `pressure_flow` に、消しゴムの印の後ろの同じ順で書き、読み戻すと同じ切り替え。書き直しても同じバイト（形式は変えない）。
use yolu_core::paths::{CanvasPath, CanvasPoint, LayerPathEntry, PathBrush};
use yolu_core::{BrushSettings, Channel, Document, LayerPath, PressureSwitches, Rgba8};
use yolu_io::{NativeDocument, NativeValue};

fn doc_with(pressure: PressureSwitches) -> Document {
    let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
    let layer = doc.add_layer("パス").unwrap();
    let path = CanvasPath {
        style: Default::default(),
        id: 7,
        channel: Channel::Color,
        brush: PathBrush(
            BrushSettings {
                radius: 2.5,
                color: Rgba8::new(200, 30, 20, 255),
                ..BrushSettings::default()
            },
            pressure,
        ),
        points: vec![
            CanvasPoint::new(3.5, 4.0, 0.25).unwrap(),
            CanvasPoint::new(27.25, 21.5, 0.75).unwrap(),
        ],
        material: None,
    };
    doc.set_canvas_paths(
        layer,
        vec![LayerPathEntry {
            name: String::new(),
            visible: true,
            path: LayerPath::Canvas(path),
        }],
    )
    .unwrap();
    doc
}

#[test]
fn the_pressure_switches_of_a_path_brush_round_trip_in_the_same_fields_and_bytes() {
    for bits in 0..8u8 {
        let pressure = PressureSwitches {
            size: bits & 1 != 0,
            opacity: bits & 2 != 0,
            flow: bits & 4 != 0,
        };
        let doc = doc_with(pressure);
        let native = NativeDocument::from_core(&doc).unwrap();
        // 消しゴムの印・大きさ・不透明度・流量の順に並ぶ 4 つの真偽の欄
        let names = [
            "erase",
            "pressure_size",
            "pressure_opacity",
            "pressure_flow",
        ];
        let fields = native.fields();
        let at = fields
            .iter()
            .position(|f| f.path.ends_with(".brush.erase"))
            .expect("消しゴムの印の欄");
        let values: Vec<(String, NativeValue)> = fields[at..at + 4]
            .iter()
            .map(|f| {
                (
                    f.path.rsplit('.').next().unwrap().to_owned(),
                    f.value.clone(),
                )
            })
            .collect();
        let expect: Vec<(String, NativeValue)> = names
            .iter()
            .zip([false, pressure.size, pressure.opacity, pressure.flow])
            .map(|(n, v)| ((*n).to_owned(), NativeValue::Bool(v)))
            .collect();
        assert_eq!(values, expect, "{bits}");
        let bytes = native.to_bytes();
        let back = NativeDocument::read(&bytes).unwrap().to_core().unwrap();
        let brush_of = |d: &Document| match &d.layers()[0].paths()[0].path {
            LayerPath::Canvas(c) => c.brush,
            LayerPath::Surface(s) => s.brush,
        };
        assert_eq!(brush_of(&back).1, pressure, "{bits}");
        assert_eq!(brush_of(&back), brush_of(&doc), "{bits}");
        assert_eq!(
            NativeDocument::from_core(&back).unwrap().to_bytes(),
            bytes,
            "{bits}"
        );
    }
}
