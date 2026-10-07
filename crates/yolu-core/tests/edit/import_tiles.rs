//! タイルをまとめて読み込む口（`import_tiles_with`）が、並びの順に 1 枚ずつ `import_tile` を呼ぶのと同じ結果・同じ誤りになることの試験。
//! まとめる口は中身を書くこと・確かめと詰めをワーカーへ分けるので、スレッド 1・4 で見る。
//! 口は詰めたタイルの場所が 16 MiB までの束ごとに作って置く。小さなタイル（1 KiB）のキャンバスは全部が 1 つの束に入るので、4 MiB のタイル
//! （1024²。束は 4 枚）のキャンバスでも同じ試験を回し、2 つ目以降の束での面の借り直し・面を作った最初の 1 枚だけで面を消す判定・束をまたぐ
//! 予算と誤りの位置を通す。

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use yolu_core::{Channel, CoreError, Document, LayerId, TileCoord};

const MIB: u64 = 1 << 20;

/// 試験のキャンバス。端のタイルがはみ出すように、寸法はタイルの大きさの倍数にしない。
#[derive(Clone, Copy, Debug)]
struct Scene {
    ts: u32,
    width: u32,
    height: u32,
    /// 口が 1 つの束に入れるタイルの数（束の大きさ 16 MiB ÷ タイルの大きさ。口の側の定数と合わせる）。
    batch: usize,
}

/// 1 KiB のタイル（16²）。キャンバス 70×50 の 5×4 = 20 枚の全部が 1 つの束に入る。
const SMALL: Scene = Scene {
    ts: 16,
    width: 70,
    height: 50,
    batch: 16384,
};

/// 4 MiB のタイル（1024²）。キャンバス 3500×2300 の 4×3 = 12 枚が、4 枚ずつ 3 つの束になる。
const BIG: Scene = Scene {
    ts: 1024,
    width: 3500,
    height: 2300,
    batch: 4,
};

impl Scene {
    fn tile_bytes(self) -> usize {
        (self.ts * self.ts * 4) as usize
    }

    fn columns(self) -> u32 {
        self.width.div_ceil(self.ts)
    }

    fn count(self) -> usize {
        (self.columns() * self.height.div_ceil(self.ts)) as usize
    }

    /// 端のタイル（右下の隅。余白が 0 でなければ断られる）の並びの位置。
    fn corner(self) -> usize {
        self.count() - 1
    }

    fn coord(self, i: usize) -> TileCoord {
        TileCoord::new(i as u32 % self.columns(), i as u32 / self.columns())
    }

    /// 下の層と、読み込む先の層（Color に 2 枚あらかじめ置いた）。
    fn canvas(self) -> (Document, LayerId) {
        let mut d = Document::with_tile_size(self.width, self.height, self.ts).unwrap();
        let below = d.add_layer("below").unwrap();
        d.import_tile(
            below,
            Channel::Color,
            TileCoord::new(1, 1),
            &vec![9; self.tile_bytes()],
        )
        .unwrap();
        let id = d.add_layer("target").unwrap();
        d.import_tile(
            id,
            Channel::Color,
            TileCoord::new(0, 0),
            &vec![5; self.tile_bytes()],
        )
        .unwrap();
        d.import_tile(id, Channel::Color, TileCoord::new(2, 1), &self.pattern(3))
            .unwrap();
        (d, id)
    }

    fn pattern(self, seed: u32) -> Vec<u8> {
        (0..self.tile_bytes() as u32)
            .map(|i| (i.wrapping_mul(2_654_435_761).wrapping_add(seed * 97) >> 13) as u8)
            .collect()
    }

    /// キャンバスの外の余白を 0 にしたタイル。
    fn trimmed(self, d: &Document, coord: TileCoord, mut bytes: Vec<u8>) -> Vec<u8> {
        let ts = self.ts;
        if (coord.x + 1) * ts <= d.width() && (coord.y + 1) * ts <= d.height() {
            return bytes;
        }
        for y in 0..ts {
            for x in 0..ts {
                if coord.x * ts + x >= d.width() || coord.y * ts + y >= d.height() {
                    let p = ((y * ts + x) * 4) as usize;
                    bytes[p..p + 4].fill(0);
                }
            }
        }
        bytes
    }

    /// 全画素・一様・透明・端のタイルを混ぜた並び（キャンバスの全部のタイル。同じ座標は 1 度）。全画素のタイルが束ごとに入る。
    fn tiles(self, d: &Document) -> Vec<(TileCoord, Vec<u8>)> {
        (0..self.count())
            .map(|i| {
                let c = self.coord(i);
                let bytes = match i % 3 {
                    0 => self.pattern(i as u32),
                    1 => vec![40; self.tile_bytes()],
                    _ => vec![0; self.tile_bytes()],
                };
                (c, self.trimmed(d, c, bytes))
            })
            .collect()
    }

    /// 全部のタイルを、今の中身から 3 枚に 1 枚だけ違う中身にした並び（変わらないタイルと変わるタイルが束をまたいで混ざる）。
    fn partly_changed(self, d: &Document) -> Vec<(TileCoord, Vec<u8>)> {
        let mut list = self.tiles(d);
        for i in (1..list.len()).step_by(3) {
            let c = list[i].0;
            list[i].1 = self.trimmed(d, c, self.pattern(1000 + i as u32));
        }
        list
    }
}

fn with_threads<T: Send>(threads: usize, run: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(run)
}

/// 文書の比べる姿: 層の面の画素（無ければ None）と、確保量・版・履歴の数・チャンネルが有効か（Debug で出さない。画素は大きい）。
struct State {
    faces: Vec<Option<Vec<u8>>>,
    rest: String,
}

fn state(d: &Document, id: LayerId) -> State {
    let l = d.layer(id).unwrap();
    State {
        faces: [Channel::Color, Channel::Normal]
            .iter()
            .map(|c| l.surface(*c).map(|s| s.canvas_bytes().unwrap()))
            .collect(),
        rest: format!(
            "alloc {} rev {} undo {} enabled {}",
            d.allocated_bytes(),
            d.revision(),
            d.undo_count(),
            l.is_channel_enabled(Channel::Normal)
        ),
    }
}

fn assert_same_state(got: &State, expected: &State, what: &str) {
    assert_eq!(got.rest, expected.rest, "{what}");
    assert_eq!(got.faces.len(), expected.faces.len(), "{what}");
    for (k, (g, e)) in got.faces.iter().zip(&expected.faces).enumerate() {
        assert_eq!(g.is_some(), e.is_some(), "{what}: 面 {k} の有無");
        assert!(g == e, "{what}: 面 {k} の画素");
    }
}

/// まとめる口で読み込む（座標ごとに並びの中身を書く）。結果と、中身を書く関数が呼ばれた回数（口が束ごとに作る数を見る）を返す。
fn together_counting(
    d: &mut Document,
    id: LayerId,
    channel: Channel,
    tiles: &[(TileCoord, Vec<u8>)],
) -> (Result<usize, CoreError>, usize) {
    let bytes: HashMap<TileCoord, &Vec<u8>> = tiles.iter().map(|(c, b)| (*c, b)).collect();
    assert_eq!(bytes.len(), tiles.len(), "同じ座標は 1 度");
    let coords: Vec<TileCoord> = tiles.iter().map(|(c, _)| *c).collect();
    let calls = AtomicUsize::new(0);
    let result = d.import_tiles_with(id, channel, &coords, |c, buf| {
        calls.fetch_add(1, Ordering::Relaxed);
        buf.copy_from_slice(bytes[&c])
    });
    (result, calls.into_inner())
}

fn together(
    d: &mut Document,
    id: LayerId,
    channel: Channel,
    tiles: &[(TileCoord, Vec<u8>)],
) -> Result<usize, CoreError> {
    together_counting(d, id, channel, tiles).0
}

/// 1 枚ずつ呼ぶ形（参照）。変わった数と、最初の誤り。誤りなら、断られたタイルの並びの位置も返す。
fn one_by_one(
    d: &mut Document,
    id: LayerId,
    channel: Channel,
    tiles: &[(TileCoord, Vec<u8>)],
) -> (Result<usize, CoreError>, Option<usize>) {
    let mut changed = 0;
    for (i, (c, bytes)) in tiles.iter().enumerate() {
        match d.import_tile(id, channel, *c, bytes) {
            Ok(did) => changed += usize::from(did),
            Err(e) => return (Err(e), Some(i)),
        }
    }
    (Ok(changed), None)
}

/// 同じ並びを、まとめる口と 1 枚ずつの形で読み込んで比べる。`room` があれば、あらかじめ置いた量にその分だけ足した予算にする。
/// 参照の結果と、断られたタイルの並びの位置を返す。
fn compare(
    scene: Scene,
    threads: usize,
    channel: Channel,
    room: Option<u64>,
    edit: impl Fn(&Document, &mut Vec<(TileCoord, Vec<u8>)>) + Sync,
) -> (Result<usize, CoreError>, Option<usize>) {
    with_threads(threads, || {
        let (mut a, id) = scene.canvas();
        let (mut b, id_b) = scene.canvas();
        if let Some(room) = room {
            let budget = a.allocated_bytes() + room;
            a.set_source_budget_bytes(budget).unwrap();
            b.set_source_budget_bytes(budget).unwrap();
        }
        let mut list = scene.tiles(&a);
        edit(&a, &mut list);
        let what = format!("{scene:?} スレッド {threads} {channel:?} 余裕 {room:?}");
        let (expected, at) = one_by_one(&mut a, id, channel, &list);
        let (got, filled) = together_counting(&mut b, id_b, channel, &list);
        assert_eq!(got, expected, "{what}");
        // 口は束ごとに作る: 断られたタイルを含む束までを作り、その先の束は作らない（予算を超えるのに置けないタイルを作りすぎない）
        let built = at.map_or(list.len(), |at| {
            ((at / scene.batch + 1) * scene.batch).min(list.len())
        });
        assert_eq!(filled, built, "{what}: 作ったタイルの数");
        assert_same_state(&state(&b, id_b), &state(&a, id), &what);
        (expected, at)
    })
}

/// 順に読み込んだ回ごとに、まとめる口と 1 枚ずつの形が同じ結果・同じ姿になる。
fn compare_rounds(
    scene: Scene,
    threads: usize,
    channel: Channel,
    rounds: impl Fn(&Document) -> Vec<Vec<(TileCoord, Vec<u8>)>> + Sync,
) {
    with_threads(threads, || {
        let (mut a, id) = scene.canvas();
        let (mut b, id_b) = scene.canvas();
        for (n, list) in rounds(&a).iter().enumerate() {
            let what = format!("{scene:?} スレッド {threads} {channel:?} {n} 回目");
            let expected = one_by_one(&mut a, id, channel, list).0;
            assert!(expected.is_ok(), "{what}");
            assert_eq!(together(&mut b, id_b, channel, list), expected, "{what}");
            assert_same_state(&state(&b, id_b), &state(&a, id), &what);
        }
    });
}

#[test]
fn importing_tiles_together_matches_importing_them_one_by_one() {
    for scene in [SMALL, BIG] {
        for threads in [1, 4] {
            for channel in [Channel::Color, Channel::Normal] {
                let (result, refused) = compare(scene, threads, channel, None, |_, _| {});
                assert!(result.is_ok() && refused.is_none(), "{scene:?} {channel:?}");
                // 同じ並びをもう 1 度（全部が前と同じ中身）、そのあと 3 枚に 1 枚だけ違う中身（束をまたいで混ざる）
                compare_rounds(scene, threads, channel, |d| {
                    vec![scene.tiles(d), scene.tiles(d), scene.partly_changed(d)]
                });
            }
            // 前と同じ中身（あらかじめ置いたタイルと同じ）と違う中身を混ぜた短い並び
            compare_rounds(scene, threads, Channel::Color, |d| {
                vec![
                    scene.tiles(d),
                    vec![
                        (TileCoord::new(0, 0), vec![5; scene.tile_bytes()]),
                        (TileCoord::new(1, 0), scene.pattern(99)),
                        (TileCoord::new(2, 1), scene.pattern(3)),
                    ],
                ]
            });
        }
    }
}

/// 途中のタイルが断られたら、その前のタイルは読み込んだまま同じ誤り（余白・キャンバスの外）。最初のタイルで断られたら、作った面は消える。
/// 大きなタイルのキャンバスでは、断られる位置を最初・束の最後・次の束の最初・束の途中・最後の束にする（時間のため、スレッドは 4 だけ。
/// 1 でも同じことは小さなタイルのキャンバスと、上の試験で見ている）。
#[test]
fn a_refused_tile_stops_at_the_same_place_with_the_same_error() {
    for (scene, places, thread_counts) in [
        (SMALL, vec![0, 7, 19], vec![1, 4]),
        (BIG, vec![0, 3, 4, 6, 8, 11], vec![4]),
    ] {
        for &threads in &thread_counts {
            for channel in [Channel::Color, Channel::Normal] {
                for &at in &places {
                    // 余白が 0 でない端のタイル（並びの at 番目へ動かす）
                    let (result, refused) = compare(scene, threads, channel, None, |_, list| {
                        list.swap(at, scene.corner());
                        list[at].1 = vec![1; scene.tile_bytes()];
                    });
                    assert!(result.is_err(), "{scene:?} {channel:?} {at}");
                    assert_eq!(refused, Some(at));
                    // キャンバスの外の座標
                    let (result, refused) = compare(scene, threads, channel, None, |_, list| {
                        list[at].0 = TileCoord::new(9, 9);
                    });
                    assert!(result.is_err(), "{scene:?} {channel:?} {at}");
                    assert_eq!(refused, Some(at));
                }
            }
        }
    }
}

/// 予算を超えるタイルで止まるときも、同じ誤り・同じ姿（それより前のタイルは読み込んだまま、最初のタイルなら面は消える）。
/// 大きなタイルのキャンバスでは、超える位置が 3 つの束のどれにもなるまで、余裕を 3 MiB ずつ増やして回す（全画素のタイルは 4 MiB なので、
/// 増やし方が 4 MiB 未満なら、超えるタイルが 1 枚ずつ順に変わる）。
#[test]
fn a_tile_over_the_budget_stops_at_the_same_place_with_the_same_error() {
    for (scene, rooms, thread_counts) in [
        (SMALL, vec![0, 1500, 3000], vec![1, 4]),
        (BIG, (0..=8).map(|k| k * 3 * MIB).collect(), vec![4]),
    ] {
        for &threads in &thread_counts {
            for channel in [Channel::Color, Channel::Normal] {
                let mut batches = BTreeSet::new();
                for &room in &rooms {
                    let (result, refused) = compare(scene, threads, channel, Some(room), |_, _| {});
                    match refused {
                        Some(at) => {
                            assert_eq!(result, Err(CoreError::SourceBudgetExceeded));
                            batches.insert(at / scene.batch);
                        }
                        None => assert!(result.is_ok()),
                    }
                }
                let last = (scene.count() - 1) / scene.batch;
                assert_eq!(
                    batches,
                    if scene.batch < scene.count() {
                        (0..=last).collect::<BTreeSet<_>>()
                    } else {
                        BTreeSet::from([0])
                    },
                    "{scene:?} {channel:?}: 超える位置が全部の束に届く"
                );
            }
        }
    }
}

#[test]
fn an_empty_list_changes_nothing() {
    let (mut d, id) = SMALL.canvas();
    let before = state(&d, id);
    assert_eq!(together(&mut d, id, Channel::Normal, &[]), Ok(0));
    assert_same_state(&state(&d, id), &before, "空の並び");
}
