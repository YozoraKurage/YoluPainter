//! 骨の名前の左右の決まり（ポーズのプリセットを左右反転して当てるときの、対になる骨の名前）。
//!
//! 名前の 1 つの区切り（`.` `_` `-` 空白で分けた 1 つ）が左右を表すとき、その名前を反対側へ置き換える。決まりは次のとおり
//! （上から順に、最初に当てはまったものだけを置き換える）。
//!
//! 1. 1 文字の区切り `L`・`R`（小文字も）: `Arm.L`・`Arm_R`・`L_Arm`・`Arm.L.001`。区切りが 1 つも無い名前（`L` だけ）は対象にしない。
//! 2. 単語 `Left`・`Right`（大文字・小文字・先頭だけ大文字）: `LeftArm`・`left_arm`・`Arm_Left`・`ArmRight`。語の前後が
//!    英字でないか、小文字から大文字へ替わる所のときだけ（`Leftover`・`Copyright` は対象にしない）。
//! 3. 先頭か末尾の `左`・`右`: `左上腕`・`腕右`。
//!
//! 骨の名前の道（根からの名前の並び）は、道の名前それぞれに当てる。どの名前も左右を表さない道は、対にならない（None）。

/// 区切りの文字。
fn is_delimiter(c: char) -> bool {
    matches!(c, '.' | '_' | '-' | ' ')
}

fn swap_letter(c: char) -> char {
    match c {
        'L' => 'R',
        'R' => 'L',
        'l' => 'r',
        'r' => 'l',
        other => other,
    }
}

/// 1 文字の区切り `L`・`R` を反対側へ（最初の 1 つ）。
fn swap_single_letter(name: &str) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    if !chars.iter().any(|c| is_delimiter(*c)) {
        return None;
    }
    let mut start = 0;
    while start <= chars.len() {
        let end = chars[start..]
            .iter()
            .position(|c| is_delimiter(*c))
            .map_or(chars.len(), |p| start + p);
        if end - start == 1 && matches!(chars[start], 'L' | 'R' | 'l' | 'r') {
            let mut out = chars.clone();
            out[start] = swap_letter(out[start]);
            return Some(out.into_iter().collect());
        }
        start = end + 1;
    }
    None
}

/// 単語 `left`・`right` の位置と長さ（語の前後が区切りか、大文字小文字の境のものだけ）。
fn find_word(chars: &[char]) -> Option<(usize, usize, bool)> {
    let matches_at = |i: usize, word: &str| -> bool {
        let n = word.chars().count();
        i + n <= chars.len()
            && chars[i..i + n]
                .iter()
                .zip(word.chars())
                .all(|(a, b)| a.to_ascii_lowercase() == b)
    };
    for i in 0..chars.len() {
        for (word, is_left) in [("left", true), ("right", false)] {
            if !matches_at(i, word) {
                continue;
            }
            let n = word.len();
            let before_ok = i == 0
                || !chars[i - 1].is_alphabetic()
                || (chars[i].is_uppercase() && chars[i - 1].is_lowercase());
            let after_ok = i + n == chars.len()
                || !chars[i + n].is_alphabetic()
                || (chars[i + n].is_uppercase() && chars[i + n - 1].is_lowercase());
            if before_ok && after_ok {
                return Some((i, n, is_left));
            }
        }
    }
    None
}

/// 単語 `Left`・`Right` を反対側へ（大文字小文字の形を保つ）。
fn swap_word(name: &str) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    let (at, len, is_left) = find_word(&chars)?;
    let original = &chars[at..at + len];
    let to = if is_left { "right" } else { "left" };
    let all_upper = original.iter().all(|c| c.is_uppercase());
    let replacement: String = if all_upper {
        to.to_uppercase()
    } else if original[0].is_uppercase() {
        let mut it = to.chars();
        it.next()
            .map(|f| f.to_uppercase().chain(it).collect())
            .unwrap_or_default()
    } else {
        to.to_owned()
    };
    let mut out: String = chars[..at].iter().collect();
    out += &replacement;
    out.extend(chars[at + len..].iter());
    Some(out)
}

/// 先頭か末尾の `左`・`右` を反対側へ。
fn swap_kanji(name: &str) -> Option<String> {
    let swap = |c: char| match c {
        '左' => Some('右'),
        '右' => Some('左'),
        _ => None,
    };
    let first = name.chars().next()?;
    if let Some(to) = swap(first) {
        let mut out = String::new();
        out.push(to);
        out.extend(name.chars().skip(1));
        return Some(out);
    }
    let last = name.chars().last()?;
    let to = swap(last)?;
    let keep = name.chars().count() - 1;
    let mut out: String = name.chars().take(keep).collect();
    out.push(to);
    Some(out)
}

/// 骨の名前の反対側（左右を表さない名前は None）。
pub fn mirror_name(name: &str) -> Option<String> {
    swap_single_letter(name)
        .or_else(|| swap_word(name))
        .or_else(|| swap_kanji(name))
}

/// 骨の名前の道の反対側（どの名前も左右を表さなければ None）。
pub fn mirror_path<S: AsRef<str>>(path: &[S]) -> Option<Vec<String>> {
    let mut changed = false;
    let out: Vec<String> = path
        .iter()
        .map(|name| match mirror_name(name.as_ref()) {
            Some(m) => {
                changed = true;
                m
            }
            None => name.as_ref().to_owned(),
        })
        .collect();
    changed.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delimited_single_letters_swap() {
        for (a, b) in [
            ("Arm.L", "Arm.R"),
            ("Arm_R", "Arm_L"),
            ("arm_l", "arm_r"),
            ("L_Arm", "R_Arm"),
            ("Arm-L", "Arm-R"),
            ("Arm L", "Arm R"),
            ("Arm.L.001", "Arm.R.001"),
            ("Strand_R_02", "Strand_L_02"),
        ] {
            assert_eq!(mirror_name(a).as_deref(), Some(b), "{a}");
            assert_eq!(mirror_name(b).as_deref(), Some(a), "{b}");
        }
        // 区切りの無い 1 文字・語の一部の L/R は対にしない
        for name in ["L", "R", "Leg", "Rib", "Arm", "Root", "Spine_01", "Bone.001"] {
            assert_eq!(mirror_name(name), None, "{name}");
        }
    }

    #[test]
    fn left_and_right_words_swap_keeping_the_case() {
        for (a, b) in [
            ("LeftArm", "RightArm"),
            ("left_arm", "right_arm"),
            ("Arm_Left", "Arm_Right"),
            ("ArmLeft", "ArmRight"),
            ("LEFT_ARM", "RIGHT_ARM"),
            ("Left Hand Index1", "Right Hand Index1"),
            ("mixamorig:LeftHand", "mixamorig:RightHand"),
        ] {
            assert_eq!(mirror_name(a).as_deref(), Some(b), "{a}");
            assert_eq!(mirror_name(b).as_deref(), Some(a), "{b}");
        }
        // 語の一部は対にしない
        for name in ["Leftover", "Copyright", "Bright", "Lefty", "Rightmost", "Upleft"] {
            assert_eq!(mirror_name(name), None, "{name}");
        }
    }

    #[test]
    fn leading_or_trailing_kanji_swap() {
        for (a, b) in [("左上腕", "右上腕"), ("右足", "左足"), ("腕左", "腕右"), ("左", "右")] {
            assert_eq!(mirror_name(a).as_deref(), Some(b), "{a}");
            assert_eq!(mirror_name(b).as_deref(), Some(a), "{b}");
        }
        assert_eq!(mirror_name("左右"), Some("右右".to_owned()), "先頭の 1 文字だけ");
        for name in ["上腕", "腰", "背骨", "手首の左側"] {
            assert_eq!(mirror_name(name), None, "{name}");
        }
    }

    #[test]
    fn a_path_mirrors_every_name_and_is_none_when_nothing_is_sided() {
        let path = ["腰", "背骨", "胸", "右肩", "右上腕"];
        assert_eq!(
            mirror_path(&path).unwrap(),
            ["腰", "背骨", "胸", "左肩", "左上腕"]
        );
        // 上の骨だけが左右の名前でも、道としては対になる
        assert_eq!(
            mirror_path(&["Root", "Arm_L", "Hand"]).unwrap(),
            ["Root", "Arm_R", "Hand"]
        );
        assert_eq!(mirror_path(&["腰", "背骨", "頭"]), None);
        assert_eq!(mirror_path::<&str>(&[]), None);
    }
}
