//! .ylp から読んだメッシュマップの条件を、ベイクの設定へそろえる（Unity 版の `LoadMeshMapFiles` の `ApplyKindKey`・`ApplySourceKey`）。
//!
//! 焼く設定はウィンドウに 1 つ（セットごとではなく、保存もしない）。マップが古いかどうかは今の設定との照合で決まるので、設定が既定のまま
//! だと、既定でない設定で焼いたマップは開いただけで「設定が変わった」と古くなり、それを読む Generator が効かない。読み込んだ
//! マップの由来（余白・アンチエイリアス・マップごとの設定の文字列・焼く元）から設定を戻して、開いただけでは古くしない。
//! 設定の文字列は `MeshBakeSettings::kind_key` が作る `名前=値;…` で、読めない値（知らない書き手の文字列）は今の値のまま残す。

use yolu_core::mesh_maps::{
    BakedMeshMap, MeshBakeSettings, MeshIdSource, MeshMapKind, MeshOccluders, MeshOcclusionFalloff,
};

/// `名前=値;名前=値` の文字列から、名前の値。
fn field<'a>(key: &'a str, name: &str) -> Option<&'a str> {
    key.split(';').find_map(|part| {
        let (n, v) = part.split_once('=')?;
        (n == name).then_some(v)
    })
}

fn number(key: &str, name: &str) -> Option<f64> {
    field(key, name)?.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn samples(key: &str) -> Option<i32> {
    field(key, "samples")?.parse::<i32>().ok().filter(|v| (1..=1024).contains(v))
}

fn occluders(key: &str) -> Option<MeshOccluders> {
    match field(key, "occluders")? {
        "WholeModel" => Some(MeshOccluders::WholeModel),
        "TargetSlotOnly" => Some(MeshOccluders::TargetSlotOnly),
        _ => None,
    }
}

/// 読み込んだマップ 1 枚の条件を設定へ写す（範囲の外の値は今の値のまま。写したあとの設定は検査を通る）。
pub fn adopt(settings: &mut MeshBakeSettings, map: &BakedMeshMap) {
    let p = map.provenance();
    let mut next = settings.clone();
    next.padding = p.padding;
    next.antialiasing = p.antialiasing;
    let key = p.settings_key.as_str();
    let distance = |name: &str| number(key, name).filter(|v| *v > 0.0 && *v <= 4.0);
    let spread = |name: &str| number(key, name).filter(|v| (1.0..=180.0).contains(v));
    match p.kind {
        MeshMapKind::AmbientOcclusion | MeshMapKind::BentNormal => {
            if let Some(v) = samples(key) {
                next.ao_samples = v;
            }
            if let Some(v) = distance("max") {
                next.ao_max_distance = v;
            }
            if let Some(v) = spread("spread") {
                next.ao_spread_degrees = v;
            }
            if let Some(v) = occluders(key) {
                next.occluders = v;
            }
            match field(key, "backfaces") {
                Some("ignore") => next.ao_ignore_backfaces = true,
                Some("occlude") => next.ao_ignore_backfaces = false,
                _ => {}
            }
            if p.kind == MeshMapKind::AmbientOcclusion {
                match field(key, "falloff") {
                    Some("None") => next.ao_falloff = MeshOcclusionFalloff::None,
                    Some("Linear") => next.ao_falloff = MeshOcclusionFalloff::Linear,
                    _ => {}
                }
            }
        }
        MeshMapKind::Thickness => {
            if let Some(v) = samples(key) {
                next.thickness_samples = v;
            }
            if let Some(v) = distance("max") {
                next.thickness_max_distance = v;
            }
            if let Some(v) = spread("spread") {
                next.thickness_spread_degrees = v;
            }
            if let Some(v) = occluders(key) {
                next.occluders = v;
            }
        }
        MeshMapKind::Curvature => {
            if let Some(v) = number(key, "radius").filter(|v| (0.001..=0.5).contains(v)) {
                next.curvature_radius = v;
            }
        }
        MeshMapKind::Id => {
            next.id_source = match field(key, "source") {
                Some("MaterialSlot") => MeshIdSource::MaterialSlot,
                Some("Mesh") => MeshIdSource::Mesh,
                Some("VertexColor") => MeshIdSource::VertexColor,
                Some("UvIsland") => MeshIdSource::UvIsland,
                Some("MeshPart") => MeshIdSource::MeshPart,
                Some("MaterialAsset") => MeshIdSource::MaterialAsset,
                _ => next.id_source,
            };
        }
        MeshMapKind::WorldNormal
        | MeshMapKind::Position
        | MeshMapKind::TangentNormal
        | MeshMapKind::Height
        | MeshMapKind::Opacity => {}
    }
    // 焼く元（高ポリの参照の距離・向きの取り方）
    if let Some(rest) = p.source.strip_prefix("Reference:") {
        if let Some(v) = number(rest, "frontal").filter(|v| *v > 0.0 && *v <= 4.0) {
            next.reference_frontal = v;
        }
        if let Some(v) = number(rest, "rear").filter(|v| *v > 0.0 && *v <= 4.0) {
            next.reference_rear = v;
        }
        match field(rest, "cage") {
            Some("average") => next.reference_average_normals = true,
            Some("vertex") => next.reference_average_normals = false,
            _ => {}
        }
        match field(rest, "match") {
            Some("name") => next.reference_match_by_name = true,
            Some("all") => next.reference_match_by_name = false,
            _ => {}
        }
    }
    if next.validate().is_ok() {
        *settings = next;
    }
}

/// 読み込んだマップの種類を、焼くマップの一覧に足す（並びは `MeshMapKind::ALL` の順）。1 つ目のセットでは読み込んだものだけにし、
/// 2 つ目からは前の分に足す。
pub fn adopt_kinds(settings: &mut MeshBakeSettings, loaded: &[MeshMapKind], keep_previous: bool) {
    let mut kinds: Vec<MeshMapKind> = if keep_previous {
        settings.maps.clone()
    } else {
        Vec::new()
    };
    for k in loaded {
        if !kinds.contains(k) {
            kinds.push(*k);
        }
    }
    kinds.sort_by_key(|k| MeshMapKind::ALL.iter().position(|a| a == k));
    if !kinds.is_empty() {
        settings.maps = kinds;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::mesh_maps::MeshMapProvenance;

    fn provenance(kind: MeshMapKind, padding: i32, key: &str, source: &str) -> BakedMeshMap {
        let p = MeshMapProvenance {
            kind,
            engine_version: 1,
            mesh_hash: "m".repeat(64),
            topology_hash: "t".repeat(64),
            uv_channel: 0,
            width: 2,
            height: 2,
            target_slot: 0,
            target_slots: vec![0],
            padding,
            antialiasing: 2,
            settings_key: key.into(),
            space: "World".into(),
            pose: "Static".into(),
            source: source.into(),
            bounds_min: [0.0; 3],
            bounds_max: [1.0; 3],
        };
        let n = 4 * kind.channels();
        BakedMeshMap::new(p, vec![0; n], vec![0; 4]).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn the_settings_follow_the_conditions_of_the_loaded_maps() {
        let mut settings = MeshBakeSettings::default();
        let ao = provenance(
            MeshMapKind::AmbientOcclusion,
            4,
            "samples=8;max=0.25;spread=120;falloff=None;backfaces=ignore;occluders=TargetSlotOnly",
            "Self",
        );
        adopt(&mut settings, &ao);
        assert_eq!((settings.padding, settings.antialiasing), (4, 2));
        assert_eq!(settings.ao_samples, 8);
        assert_eq!(settings.ao_max_distance, 0.25);
        assert_eq!(settings.ao_spread_degrees, 120.0);
        assert_eq!(settings.ao_falloff, MeshOcclusionFalloff::None);
        assert!(settings.ao_ignore_backfaces);
        assert_eq!(settings.occluders, MeshOccluders::TargetSlotOnly);
        // 戻した設定は、そのマップの設定の文字列を作り直せる（開いただけでは古くならない）
        assert_eq!(
            settings.kind_key(MeshMapKind::AmbientOcclusion, ""),
            ao.provenance().settings_key
        );
        let curvature = provenance(MeshMapKind::Curvature, 4, "radius=0.05", "Self");
        adopt(&mut settings, &curvature);
        assert_eq!(settings.curvature_radius, 0.05);
        assert_eq!(settings.kind_key(MeshMapKind::Curvature, ""), "radius=0.05");
        let id = provenance(MeshMapKind::Id, 4, "source=MeshPart;algorithm=2", "Self");
        adopt(&mut settings, &id);
        assert_eq!(settings.id_source, MeshIdSource::MeshPart);
    }

    #[test]
    fn unreadable_keys_and_out_of_range_values_leave_the_settings_alone() {
        let mut settings = MeshBakeSettings::default();
        let before = settings.kind_key(MeshMapKind::AmbientOcclusion, "");
        let junk = provenance(
            MeshMapKind::AmbientOcclusion,
            16,
            "samples=999999;max=-3;spread=nan;falloff=Cubic;occluders=Everything",
            "Elsewhere",
        );
        adopt(&mut settings, &junk);
        assert_eq!(settings.kind_key(MeshMapKind::AmbientOcclusion, ""), before);
    }

    #[test]
    fn the_baked_kinds_are_listed_in_the_standard_order() {
        let mut settings = MeshBakeSettings::default();
        adopt_kinds(&mut settings, &[MeshMapKind::Curvature, MeshMapKind::Position], false);
        assert_eq!(settings.maps, [MeshMapKind::Position, MeshMapKind::Curvature]);
        adopt_kinds(&mut settings, &[MeshMapKind::WorldNormal], true);
        assert_eq!(
            settings.maps,
            [MeshMapKind::WorldNormal, MeshMapKind::Position, MeshMapKind::Curvature]
        );
    }
}
