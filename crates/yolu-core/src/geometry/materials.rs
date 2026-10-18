//! テクスチャセットが描くマテリアルの組（三角形の `material` の番号の組）。1 つのセットは、複数のマテリアルの面を同じテクスチャに描ける。
//! 面を絞る口は `Option<&MaterialSet>` を受け、None はどの面も、組は組に入るマテリアルの面だけを扱う。

/// マテリアルの組の番号（昇順・重なり無し）。空の組はどの面にも当たらない。
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MaterialSet(Vec<i32>);

impl MaterialSet {
    /// 番号の並びから作る（並べ替えて重なりを除く）。
    pub fn new(materials: impl IntoIterator<Item = i32>) -> MaterialSet {
        let mut list: Vec<i32> = materials.into_iter().collect();
        list.sort_unstable();
        list.dedup();
        MaterialSet(list)
    }

    /// マテリアル 1 つの組。
    pub fn one(material: i32) -> MaterialSet {
        MaterialSet(vec![material])
    }

    /// 組に入るか。
    #[inline]
    pub fn contains(&self, material: i32) -> bool {
        match self.0.as_slice() {
            [one] => *one == material,
            list => list.binary_search(&material).is_ok(),
        }
    }

    /// 番号の並び（昇順）。
    pub fn as_slice(&self) -> &[i32] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// いちばん小さい番号（空なら None）。
    pub fn first(&self) -> Option<i32> {
        self.0.first().copied()
    }
}

/// 面を絞る口の決まり: None はどのマテリアルも、組は組に入るマテリアルだけを通す。
#[inline]
pub fn admits(set: Option<&MaterialSet>, material: i32) -> bool {
    set.is_none_or(|s| s.contains(material))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_is_sorted_without_repeats_and_none_admits_every_material() {
        let set = MaterialSet::new([3, 1, 3, 2]);
        assert_eq!(set.as_slice(), [1, 2, 3]);
        assert!(set.contains(2) && !set.contains(0));
        assert!(MaterialSet::one(5).contains(5) && !MaterialSet::one(5).contains(4));
        assert!(
            !MaterialSet::default().contains(0),
            "空の組はどの面にも当たらない"
        );
        assert!(admits(None, -7));
        assert!(admits(Some(&set), 1) && !admits(Some(&set), 4));
    }
}
