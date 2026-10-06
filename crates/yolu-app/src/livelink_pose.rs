//! Live Link で受けるポーズの溜め場。Unity は形が変わるたびに（0.2 秒ごとに）ポーズを送る。画面のスレッドがフレームを回していない間
//! （最小化・隠れた窓・重い処理の最中）も、読むスレッドは受け続けるので、命令の列にそのまま積むと、ポーズの分だけ無制限に膨らむ。
//! 読むスレッドは、ポーズをここへメッシュごとに**最新だけ**を残して溜め、画面のスレッドが取り出す。
//!
//! 決まり:
//! - 溜めるのは同じ世代のポーズだけ。別の世代のポーズが来たら、溜めていた分は捨てる（世代はモデルを送り直すたびに替わり、古い世代のポーズは
//!   どのみち断られる）。
//! - 溜める量の上限は [`MAX_POSE_BYTES`]（位置と法線の合計）。超えるメッシュのポーズは溜めずに数える（`dropped`）。メッシュごとに 1 つしか持たない
//!   ので、正しいモデルの量を超えない。壊れた・悪意のある送り手がメッシュの番号を散らして送っても、上限で止まる。
//! - 順番: ポーズは、あとから来る別の命令（モデル・マテリアルの更新・閉じる・終わり）より先に適用されなければならない。読むスレッドは、別の命令を
//!   命令の列へ積む前に、溜めたポーズを先に列へ流す（`take`。溜め場は空になる）。溜め場が空のときに溜め始めたら、新しい「束」を始め、列へ「束の知らせ」
//!   （束の番号つき）を積む。画面のスレッドは、その知らせを列の順に読んだときに、同じ番号の束だけを取り出す（`take_batch`）。知らせより後ろの命令が
//!   来ていれば、束はもう列へ流れて（または次の束に替わって）いるので、取り出せない。こうして、取り出すポーズは、列でその知らせより前にある命令より
//!   新しく、後ろにある命令より古い。画面のスレッドが列を空にした直後に読むスレッドが新しいモデルとそのポーズを積んでも、ポーズが先に当たらない。

use std::collections::BTreeMap;
use std::sync::Mutex;

use yolu_protocol::{MeshPose, Pose};

/// 溜めておくポーズの量の上限（位置と法線の合計のバイト）。
pub const MAX_POSE_BYTES: usize = 256 << 20;

fn bytes_of(mesh: &MeshPose) -> usize {
    (mesh.positions.len() + mesh.normals.len()) * 12
}

#[derive(Default)]
struct Buffer {
    /// 今の（または最後に始めた）束の番号。溜め場が空のときに溜め始めるたびに 1 つ進む。
    batch: u64,
    generation: u32,
    meshes: BTreeMap<u32, MeshPose>,
    bytes: usize,
    /// 取り出されるまでに、上限のために溜めなかったメッシュのポーズの数。
    dropped: u64,
}

/// ポーズの溜め場（つながり 1 つにつき 1 つ）。
#[derive(Default)]
pub struct PoseSlot {
    inner: Mutex<Buffer>,
    limit: Option<usize>,
}

/// 取り出したポーズ。
#[derive(Debug, PartialEq)]
pub struct Taken {
    pub pose: Pose,
    /// 溜めきれずに捨てたメッシュのポーズの数。
    pub dropped: u64,
}

impl PoseSlot {
    pub fn new() -> PoseSlot {
        PoseSlot::default()
    }

    /// 溜める量の上限を決める（試験用。既定は [`MAX_POSE_BYTES`]）。
    pub fn with_limit(limit: usize) -> PoseSlot {
        PoseSlot {
            inner: Mutex::default(),
            limit: Some(limit),
        }
    }

    fn buffer(&self) -> std::sync::MutexGuard<'_, Buffer> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// ポーズを溜める（同じメッシュの前のポーズは置き換える）。溜め場が空だったなら新しい束を始め、その番号を返す（呼び手は、束の知らせを列へ積む。
    /// すでに束があれば None: 知らせは積んである）。
    pub fn put(&self, pose: Pose) -> Option<u64> {
        let limit = self.limit.unwrap_or(MAX_POSE_BYTES);
        let mut buf = self.buffer();
        let was_empty = buf.meshes.is_empty() && buf.dropped == 0;
        if was_empty {
            buf.batch += 1;
        }
        if buf.generation != pose.generation {
            // 別の世代: 溜めていた分は、もう適用先が無い
            buf.meshes.clear();
            buf.bytes = 0;
            buf.generation = pose.generation;
        }
        for mesh in pose.meshes {
            let size = bytes_of(&mesh);
            let old = buf.meshes.get(&mesh.mesh).map_or(0, bytes_of);
            if buf.bytes - old + size > limit {
                buf.dropped += 1;
                continue;
            }
            buf.bytes = buf.bytes - old + size;
            buf.meshes.insert(mesh.mesh, mesh);
        }
        was_empty.then_some(buf.batch)
    }

    /// 溜めたポーズを全部取り出す（無ければ None）。読むスレッドが、別の命令を列へ積む前に、溜めた分を列へ流すのに使う。
    pub fn take(&self) -> Option<Taken> {
        Self::drain(&mut self.buffer())
    }

    /// 束の知らせ（`put` が返した番号）を列の順に読んだ画面のスレッドが、その束のポーズを取り出す。その束がもう列へ流れた・次の束に替わっていれば None。
    pub fn take_batch(&self, batch: u64) -> Option<Taken> {
        let mut buf = self.buffer();
        if buf.batch != batch {
            return None;
        }
        Self::drain(&mut buf)
    }

    fn drain(buf: &mut Buffer) -> Option<Taken> {
        if buf.meshes.is_empty() && buf.dropped == 0 {
            return None;
        }
        let meshes: Vec<MeshPose> = std::mem::take(&mut buf.meshes).into_values().collect();
        let dropped = std::mem::take(&mut buf.dropped);
        buf.bytes = 0;
        Some(Taken {
            pose: Pose {
                generation: buf.generation,
                meshes,
            },
            dropped,
        })
    }

    /// 溜めているポーズの量（位置と法線の合計のバイト。試験・診断用）。
    pub fn pending_bytes(&self) -> usize {
        self.buffer().bytes
    }

    /// 溜めているメッシュの数（試験・診断用）。
    pub fn pending_meshes(&self) -> usize {
        self.buffer().meshes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(index: u32, vertices: usize, x: f32) -> MeshPose {
        MeshPose {
            mesh: index,
            positions: vec![[x, 0.0, 0.0]; vertices],
            normals: vec![[0.0, 1.0, 0.0]; vertices],
        }
    }

    fn pose(generation: u32, meshes: Vec<MeshPose>) -> Pose {
        Pose { generation, meshes }
    }

    #[test]
    fn only_the_newest_pose_of_each_mesh_is_kept() {
        let slot = PoseSlot::new();
        assert!(slot.take().is_none());
        assert!(
            slot.put(pose(2, vec![mesh(0, 4, 1.0), mesh(1, 4, 1.0)]))
                .is_some(),
            "空だったので束を始める"
        );
        // 続けて来たポーズは束を始めない（溜め場は膨らまない）
        assert!(slot.put(pose(2, vec![mesh(0, 4, 2.0)])).is_none());
        assert!(slot
            .put(pose(2, vec![mesh(0, 4, 3.0), mesh(2, 2, 3.0)]))
            .is_none());
        assert_eq!(
            (slot.pending_meshes(), slot.pending_bytes()),
            (3, (4 + 4 + 2) * 24)
        );
        let taken = slot.take().unwrap();
        assert_eq!(taken.dropped, 0);
        assert_eq!(taken.pose.generation, 2);
        let xs: Vec<(u32, f32)> = taken
            .pose
            .meshes
            .iter()
            .map(|m| (m.mesh, m.positions[0][0]))
            .collect();
        assert_eq!(
            xs,
            vec![(0, 3.0), (1, 1.0), (2, 3.0)],
            "メッシュごとに最新だけ。番号の順"
        );
        assert!(slot.take().is_none(), "取り出したら空");
        assert_eq!(slot.pending_bytes(), 0);
        // 取り出したあとの次のポーズは、また束を始める
        assert!(slot.put(pose(2, vec![mesh(0, 1, 5.0)])).is_some());
    }

    #[test]
    fn a_pile_of_poses_does_not_grow_the_slot() {
        // 画面のスレッドが止まっている間に、同じモデルのポーズが何万回来ても、溜め場は 1 つのモデルの量のまま
        let slot = PoseSlot::new();
        for i in 0..50_000u32 {
            slot.put(pose(
                1,
                vec![mesh(0, 1000, i as f32), mesh(1, 500, i as f32)],
            ));
        }
        assert_eq!(slot.pending_meshes(), 2);
        assert_eq!(slot.pending_bytes(), (1000 + 500) * 24);
        assert_eq!(
            slot.take().unwrap().pose.meshes[0].positions[0][0],
            49_999.0
        );
    }

    #[test]
    fn a_pose_of_another_generation_replaces_the_pile() {
        let slot = PoseSlot::new();
        slot.put(pose(1, vec![mesh(0, 4, 1.0), mesh(1, 4, 1.0)]));
        assert!(
            slot.put(pose(2, vec![mesh(0, 4, 9.0)])).is_none(),
            "溜め場は空でなかった"
        );
        let taken = slot.take().unwrap();
        assert_eq!(taken.pose.generation, 2);
        assert_eq!(
            taken.pose.meshes.len(),
            1,
            "前の世代のメッシュ 1 は残らない"
        );
        assert_eq!(taken.pose.meshes[0].positions[0][0], 9.0);
    }

    #[test]
    fn poses_over_the_limit_are_dropped_and_counted_not_kept() {
        // 上限は 1 つのメッシュの置き換えでは数えない（同じメッシュは前の分を引く）。別のメッシュを散らして送られても上限で止まる
        let slot = PoseSlot::with_limit(10 * 24);
        slot.put(pose(1, vec![mesh(0, 6, 1.0)]));
        slot.put(pose(1, vec![mesh(0, 8, 2.0)])); // 置き換え: 8 頂点（192 バイト）
        assert_eq!(slot.pending_bytes(), 8 * 24);
        slot.put(pose(1, vec![mesh(1, 3, 1.0), mesh(2, 1, 1.0)])); // 3 頂点は入らず、1 頂点は入る
        assert_eq!(slot.pending_meshes(), 2);
        let taken = slot.take().unwrap();
        assert_eq!(taken.dropped, 1);
        assert_eq!(
            taken.pose.meshes.iter().map(|m| m.mesh).collect::<Vec<_>>(),
            vec![0, 2]
        );
        // 量が全部上限を超えるだけでも、捨てたことは取り出しで分かる（空のポーズが来る）
        let slot = PoseSlot::with_limit(24);
        assert!(slot.put(pose(1, vec![mesh(0, 5, 1.0)])).is_some());
        let taken = slot.take().unwrap();
        assert_eq!((taken.dropped, taken.pose.meshes.len()), (1, 0));
        assert!(slot.take().is_none());
        // 上限の既定は 256 MiB
        assert_eq!(MAX_POSE_BYTES, 256 << 20);
    }

    #[test]
    fn a_batch_is_taken_only_by_its_own_notice() {
        let slot = PoseSlot::new();
        let first = slot.put(pose(1, vec![mesh(0, 2, 1.0)])).expect("束 1");
        assert!(slot.put(pose(1, vec![mesh(0, 2, 2.0)])).is_none());
        // 知らせ以外の番号では取り出せない
        assert!(slot.take_batch(first + 1).is_none());
        assert!(slot.take_batch(first.wrapping_sub(1)).is_none());
        assert_eq!(slot.pending_meshes(), 1, "取り出せなかった束はそのまま");
        let taken = slot.take_batch(first).unwrap();
        assert_eq!(
            taken.pose.meshes[0].positions[0][0], 2.0,
            "束の中では最新だけ"
        );
        assert!(slot.take_batch(first).is_none(), "取り出したら空");
    }

    #[test]
    fn a_notice_does_not_take_poses_that_came_after_a_later_command() {
        // 列: [束 1 の知らせ, 流した束 1 のポーズ, モデル, 束 2 の知らせ]。画面のスレッドが束 1 の知らせを読む時点で、溜め場には、モデルの
        // あとに来た束 2 のポーズがある。束 1 の知らせでそれを取って、モデルより先に当ててはならない（前の世代のモデルに当てて断られる）
        let slot = PoseSlot::new();
        let first = slot.put(pose(1, vec![mesh(0, 2, 1.0)])).unwrap();
        // 読むスレッドが、モデルを列へ積む前に、溜めたポーズを列へ流す
        let flushed = slot.take().unwrap();
        assert_eq!(flushed.pose.generation, 1);
        // モデルのあとのポーズ（世代 2）。溜め場は空だったので、新しい束
        let second = slot.put(pose(2, vec![mesh(0, 2, 9.0)])).unwrap();
        assert_ne!(first, second);
        // 画面のスレッドは束 1 の知らせを先に読む: 取り出すものは無い（流れた）。束 2 のポーズは動かない
        assert!(slot.take_batch(first).is_none());
        assert_eq!(slot.pending_meshes(), 1);
        // 束 2 の知らせ（モデルのあと）で取り出す
        let taken = slot.take_batch(second).unwrap();
        assert_eq!(
            (taken.pose.generation, taken.pose.meshes[0].positions[0][0]),
            (2, 9.0)
        );
    }

    #[test]
    fn a_notice_for_a_batch_that_was_flushed_finds_nothing_even_if_the_slot_is_empty() {
        let slot = PoseSlot::new();
        let first = slot.put(pose(1, vec![mesh(0, 1, 1.0)])).unwrap();
        assert!(slot.take().is_some());
        assert!(slot.take_batch(first).is_none());
        // 空の溜め場に溜め始めるたびに、番号は進む
        let second = slot.put(pose(1, vec![mesh(0, 1, 2.0)])).unwrap();
        assert!(second > first);
    }
}
