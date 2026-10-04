//! 焼いた ID マップの色の取得と比較。Empty は選ばず、余白は選ぶ。
use crate::{
    mesh_maps::{BakedMeshMap, MeshMapKind},
    CoreError,
};
pub const DEFAULT_TOLERANCE: u8 = 8;
fn require(map: &BakedMeshMap) -> Result<(), CoreError> {
    if map.kind() != MeshMapKind::Id {
        Err(CoreError::InvalidArgument("ID マップが必要"))
    } else {
        Ok(())
    }
}
pub fn try_get(map: &BakedMeshMap, x: i64, y: i64) -> Result<Option<u32>, CoreError> {
    require(map)?;
    if x < 0 || y < 0 || x >= map.width() as i64 || y >= map.height() as i64 {
        return Ok(None);
    }
    let i = y as usize * map.width() + x as usize;
    if map.coverage()[i] == 0 {
        return Ok(None);
    }
    let d = &map.data()[i * 3..i * 3 + 3];
    let b = |v: u16| (v as u32 * 255 + 32767) / 65535;
    Ok(Some(b(d[0]) << 16 | b(d[1]) << 8 | b(d[2])))
}
pub fn try_get_at_uv(map: &BakedMeshMap, u: f64, v: f64) -> Result<Option<u32>, CoreError> {
    require(map)?;
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return Ok(None);
    }
    try_get(
        map,
        ((u * map.width() as f64).floor() as i64).min(map.width() as i64 - 1),
        ((v * map.height() as f64).floor() as i64).min(map.height() as i64 - 1),
    )
}
pub fn near(a: u32, b: u32, tolerance: u8) -> bool {
    [0, 8, 16]
        .into_iter()
        .all(|s| ((a >> s) & 255).abs_diff((b >> s) & 255) <= tolerance as u32)
}
pub fn hex(rgb: u32) -> String {
    format!("#{:06X}", rgb & 0xffffff)
}
impl crate::SelectionMask {
    pub fn from_id_colors(
        doc: &crate::Document,
        map: &BakedMeshMap,
        colors: &[u32],
        tolerance: u8,
    ) -> Result<Self, CoreError> {
        require(map)?;
        if map.width() != doc.width() as usize || map.height() != doc.height() as usize {
            return Err(CoreError::InvalidArgument("ID マップの大きさ"));
        }
        if colors.iter().any(|c| *c > 0xffffff) {
            return Err(CoreError::InvalidArgument("ID の色"));
        }
        Ok(Self::build(
            doc.width(),
            doc.height(),
            doc.tile_size(),
            (0, 0, doc.width() as i64, doc.height() as i64),
            |x, y| {
                if try_get(map, x, y)
                    .unwrap()
                    .is_some_and(|c| colors.iter().any(|wanted| near(c, *wanted, tolerance)))
                {
                    255
                } else {
                    0
                }
            },
        ))
    }
}
