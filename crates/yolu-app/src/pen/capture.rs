//! ペンの押しを OS が奪う場面と、ペンでウィンドウを動かす掴み（OS に依らない決め方。Windows のメッセージの受け方は `win_ink`）。
//!
//! Windows Ink では、触れているペンのポインタはウィンドウに暗黙に捕まえられていて、離しの `WM_POINTERUP` も同じウィンドウへ届く。ところが OS が移動・大きさ変更の輪へ入る、
//! ほかのウィンドウが捕まえるなどでポインタを失うと、`WM_POINTERCAPTURECHANGED` だけが届いて、以後そのポインタの知らせは来ない（`WM_POINTERUP` と対にならない）。
//! 離しが来ないと、受け口の「触れている」記録・押しの持ち主・ビューの `PenPress`・egui のポインタの押しが全部残り、次の押しを受けなくなる。
//! ここでは、そのような合図（`Loss`）を受けたときに、触れていた押しを離したことにする決め方（`seize`）を持つ。
//!
//! 帯をペンや指で引いてウィンドウを動かすときは、OS の移動の輪（`ViewportCommand::StartDrag`）に渡さず、アプリがウィンドウの位置を追わせる（`PenWindowMove`）。
//! winit の `drag_window` は、始めたときに「動かしている最中」の印を立て、`WM_EXITSIZEMOVE` でだけ戻す（印が立っている間は、次の `drag_window` が何もしない）。
//! 輪はマウスのボタンの押しで始めるもので、ペンや指の押しで始めても `WM_EXITSIZEMOVE` が来る保証が無いので、ペンや指の押しでは輪に頼らない。
//! マウスの引きで輪に渡したあと、輪が始まらないときの印は、見張り（`DragWatch`）が一定の時間のあとに戻す。
//! ペンの位置から目標の位置を決めるのは純関数にして試験できる。

use std::time::{Duration, Instant};

use super::wintab::{PressOwner, Touch};
use super::PenSample;

/// 押しを奪われた合図。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loss {
    /// `WM_POINTERCAPTURECHANGED`: そのポインタ（番号）の捕まえを失った。`kept` はこのウィンドウが捕まえ直した（`lParam` が自分）。
    Pointer { id: u32, kept: bool },
    /// `WM_CAPTURECHANGED`: マウスの捕まえが替わった。`elsewhere` はほかのウィンドウが捕まえた（捕まえを手放しただけ（普通のマウスの離し）・自分が捕まえ直したときは false）。
    Mouse { elsewhere: bool },
    /// `WM_EXITSIZEMOVE`: 移動・大きさ変更の輪が終わった（輪の中では、ペンの離しがこのウィンドウに届かないことがある）。
    ExitMove,
}

impl Loss {
    /// 記録に出す名前（メッセージの名前）。
    pub fn message(self) -> &'static str {
        match self {
            Loss::Pointer { .. } => "WM_POINTERCAPTURECHANGED",
            Loss::Mouse { .. } => "WM_CAPTURECHANGED",
            Loss::ExitMove => "WM_EXITSIZEMOVE",
        }
    }

    /// 触れているペンの点 `touching` の押しを、この合図が奪ったか。別のポインタの捕まえの変更・自分が捕まえ直した合図・普通のマウスの離しは奪わない。
    pub fn takes(self, touching: &PenSample) -> bool {
        self.takes_pointer(touching.pointer_id)
    }

    /// ポインタの番号 `pointer` のペンの押しを、この合図が奪ったか。
    pub fn takes_pointer(self, pointer: u32) -> bool {
        match self {
            Loss::Pointer { id, kept } => !kept && id == pointer,
            Loss::Mouse { elsewhere } => elsewhere,
            Loss::ExitMove => true,
        }
    }
}

/// Windows Ink のペンが触れていて、`loss` がその押しを奪ったなら、離しの点（その位置で触れていない点。記録は忘れる）を返し、押しの持ち主が Windows Ink なら手放す。
/// 奪われていない（触れていない・別のポインタ・捕まえ直した）なら何も変えない。WinTab の押しは、ドライバーのパケットがウィンドウのメッセージと別に来るので対象にしない
/// （`touch` は Windows Ink の点だけを覚えているので、WinTab の押しの間は触れていない）。
pub fn seize(touch: &mut Touch, owner: &mut PressOwner, loss: Loss) -> Option<PenSample> {
    let touching = touch.touching()?;
    if !loss.takes(&touching) {
        return None;
    }
    owner.ink_up();
    touch.release()
}

/// 奪われた合図を受けた記録の 1 行（`YOLU_PEN_LOG`）。`touching` はそのとき触れていたペンの点、`lifted` は受け口へ離しを補ったか、`egui_up` は egui の
/// ポインタの押しの離し（`WM_LBUTTONUP`）を post したか。
pub fn log_loss(loss: Loss, touching: Option<&PenSample>, lifted: bool, egui_up: bool) -> String {
    let mut parts = vec![format!("loss msg={}", loss.message())];
    match loss {
        Loss::Pointer { id, kept } => parts.push(format!("id={id} kept={kept}")),
        Loss::Mouse { elsewhere } => parts.push(format!("elsewhere={elsewhere}")),
        Loss::ExitMove => {}
    }
    parts.push(format!(
        "touching={}",
        touching.map_or_else(
            || "none".to_string(),
            |s| format!("id={} pos=({:.1},{:.1})", s.pointer_id, s.pos[0], s.pos[1]),
        )
    ));
    parts.push(format!("lifted={lifted} egui-up={egui_up}"));
    parts.join(" ")
}

// ───────── 奪って補った番号 ─────────

/// 奪って離しを補ったポインタの番号。次の `WM_POINTERDOWN`（触れた最初のメッセージ）まで、その番号の触れた点を使わない: 奪われたあとにも同じ番号の点が
/// 来続けると、補った離しのあとで押しが新しく始まり、ストロークが 2 つに割れる。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TakenIds {
    ids: Vec<u32>,
}

impl TakenIds {
    /// 奪われた番号（これ以上は覚えない。古い物から忘れる）。
    const MAX: usize = 8;

    /// 番号 `id` の押しを奪って、離しを補った。
    pub fn take(&mut self, id: u32) {
        if self.ids.contains(&id) {
            return;
        }
        if self.ids.len() >= Self::MAX {
            self.ids.remove(0);
        }
        self.ids.push(id);
    }

    /// 点 `s` を受け口へ入れてよいか。`down` は触れた最初のメッセージ（新しい押し）。奪われた番号の、`down` でない触れた点は入れない。
    /// 新しい押しと、触れていない点（離し・浮いた点）は、その番号の記録を消して入れる。
    pub fn admit(&mut self, down: bool, s: &PenSample) -> bool {
        let Some(at) = self.ids.iter().position(|&id| id == s.pointer_id) else {
            return true;
        };
        if down || !s.contact {
            self.ids.remove(at);
            return true;
        }
        false
    }
}

// ───────── 輪の見張り ─────────

/// `StartDrag` を OS の移動の輪に渡してから、`WM_ENTERSIZEMOVE` が来るのを待つ時間。
pub const START_WAIT: Duration = Duration::from_secs(1);

/// OS の移動の輪に `StartDrag` を渡したあと、輪が始まったかの見張り。輪が始まらない（始める時にマウスのボタンがもう離れていたなど）と `WM_EXITSIZEMOVE` が来ず、
/// winit の「動かしている最中」の印が残って、以後の `StartDrag` が全部捨てられる。輪が生きている間（`WM_ENTERSIZEMOVE` から `WM_EXITSIZEMOVE` まで）は何も言わないので、
/// 見張りが `WM_EXITSIZEMOVE` を補っても、生きている輪に二重には送らない。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DragWatch {
    handed: Option<Instant>,
    in_loop: bool,
}

impl DragWatch {
    /// `StartDrag` を OS に渡した。
    pub fn handed(&mut self, now: Instant) {
        self.handed = Some(now);
    }

    /// 輪が始まった（`WM_ENTERSIZEMOVE`）。
    pub fn entered(&mut self) {
        self.in_loop = true;
        self.handed = None;
    }

    /// 輪が終わった（`WM_EXITSIZEMOVE`）。
    pub fn exited(&mut self) {
        self.in_loop = false;
        self.handed = None;
    }

    /// 渡して `START_WAIT` たっても輪が始まっていないなら true（1 度だけ。見張りを戻す）。このとき呼んだ側が、自分のウィンドウへ `WM_EXITSIZEMOVE` を送る。
    pub fn stuck(&mut self, now: Instant) -> bool {
        let late = !self.in_loop
            && self
                .handed
                .and_then(|at| now.checked_duration_since(at))
                .is_some_and(|waited| waited >= START_WAIT);
        if late {
            self.handed = None;
        }
        late
    }

    /// 輪が生きているか（`WM_ENTERSIZEMOVE` を受けて、`WM_EXITSIZEMOVE` をまだ受けていない）。
    pub fn in_loop(&self) -> bool {
        self.in_loop
    }
}

// ───────── ペンでウィンドウを動かす ─────────

/// 触れているポインタ（ペン・指）の直近の点（画面の位置つき）。ウィンドウを動かし始める根拠。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LastPen {
    pub pointer: u32,
    pub contact: bool,
    /// 画面の位置（仮想スクリーンの物理の画素）。
    pub screen: [f64; 2],
}

impl LastPen {
    /// 触れていれば、掴みの出どころ（ポインタの番号と画面の位置）。古さは問わない: 帯を引き始めた押しがペン・指のものだと分かっているとき（egui の押しが Touch から来た）だけ
    /// 呼ぶので、止めたペンから更新が来なくても、最後の位置から掴む。
    pub fn source(&self) -> Option<(u32, [f64; 2])> {
        self.contact.then_some((self.pointer, self.screen))
    }
}

/// ウィンドウの大きさが `from` から `to` に変わったとき（別の拡大率の画面へまたいだ）、掴んだ点のウィンドウの左上からの差 `offset` を、幅の比で直す。
/// 差はウィンドウの中に収める（大きさが分からない・0 以下なら、そのまま）。
pub fn rescale_offset(offset: [f64; 2], from: [i32; 2], to: [i32; 2]) -> [f64; 2] {
    if from[0] <= 0 || to[0] <= 0 {
        return offset;
    }
    let ratio = f64::from(to[0]) / f64::from(from[0]);
    let inside = |value: f64, extent: i32| value.clamp(0.0, f64::from((extent - 1).max(0)));
    [
        inside(offset[0] * ratio, to[0]),
        inside(offset[1] * ratio, to[1]),
    ]
}

/// ペンで動かしているウィンドウの掴み。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowGrab {
    pointer: u32,
    /// 掴んだ点の、ウィンドウの左上からの差（物理の画素。`size` のときの値）。
    offset: [f64; 2],
    /// `offset` を決めたときのウィンドウの大きさ。
    size: [i32; 2],
}

impl WindowGrab {
    /// ペンが `pen` にあるときのウィンドウの左上。ウィンドウの大きさが変わっていたら、差を取り直す。目標は掴んだ点の差だけで決める
    /// （前の目標からの差を足さないので、丸めの誤差が溜まらない）。
    pub fn target(&mut self, pen: [f64; 2], size: [i32; 2]) -> [i32; 2] {
        // 大きさが読めない（0 以下）ときは、差を取り直さない
        if size[0] > 0 && size[1] > 0 && size != self.size {
            self.offset = rescale_offset(self.offset, self.size, size);
            self.size = size;
        }
        [
            (pen[0] - self.offset[0]).round() as i32,
            (pen[1] - self.offset[1]).round() as i32,
        ]
    }
}

/// ペンの 1 点を受けた結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Follow {
    /// 掴んでいない・別のペンの点。
    Idle,
    /// ウィンドウをこの左上へ動かす。
    To([i32; 2]),
    /// ペンが離れて、掴みが終わった。
    Ended,
}

/// ウィンドウごとの、ペンでウィンドウを動かす掴み（高々 1 つ）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PenWindowMove {
    grab: Option<WindowGrab>,
}

impl PenWindowMove {
    /// ペン `pointer` が画面の `pen` にあり、ウィンドウの左上が `window`・大きさが `size` のときから掴む（前の掴みは捨てる）。
    pub fn begin(&mut self, pointer: u32, pen: [f64; 2], window: [i32; 2], size: [i32; 2]) {
        self.grab = Some(WindowGrab {
            pointer,
            offset: [pen[0] - f64::from(window[0]), pen[1] - f64::from(window[1])],
            size,
        });
    }

    pub fn is_active(&self) -> bool {
        self.grab.is_some()
    }

    /// ペンの 1 点（画面の位置 `pen`、ウィンドウの今の大きさ `size`）。掴んでいるペンの接触の点ならウィンドウの目標、離しの点なら掴みを終える。
    pub fn follow(&mut self, pointer: u32, contact: bool, pen: [f64; 2], size: [i32; 2]) -> Follow {
        let Some(grab) = self.grab.as_mut() else {
            return Follow::Idle;
        };
        if grab.pointer != pointer {
            return Follow::Idle;
        }
        if contact {
            Follow::To(grab.target(pen, size))
        } else {
            self.grab = None;
            Follow::Ended
        }
    }

    /// 掴みを捨てる（ウィンドウが閉じる）。掴んでいたなら true。
    pub fn cancel(&mut self) -> bool {
        self.grab.take().is_some()
    }

    /// 押しを奪う合図 `loss` が、掴んでいるペンの押しを奪うなら、掴みを捨てる。捨てたなら true。
    pub fn lose(&mut self, loss: Loss) -> bool {
        match self.grab {
            Some(grab) if loss.takes_pointer(grab.pointer) => self.cancel(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Tilt;
    use crate::pen::wintab::{route_ink, InkMessage};

    fn sample(id: u32, contact: bool, x: f32) -> PenSample {
        PenSample {
            pos: [x, 10.0],
            pressure: if contact { 0.6 } else { 0.0 },
            tilt: Tilt::default(),
            rotation: None,
            contact,
            eraser: false,
            barrel: false,
            pointer_id: id,
            time_ms: 0,
        }
    }

    /// Windows Ink の 1 点を、押しの持ち主の通し（WinTab が開いている間の `route_ink`）と触れている記録に通す。
    fn ink(
        touch: &mut Touch,
        owner: &mut PressOwner,
        message: InkMessage,
        s: PenSample,
    ) -> Vec<PenSample> {
        let kept = route_ink(owner, message, vec![s]);
        for k in &kept {
            touch.note(k);
        }
        kept
    }

    #[test]
    fn only_a_lost_capture_of_the_touching_pen_takes_the_press() {
        let touching = sample(7, true, 5.0);
        // そのポインタの捕まえを失った → 奪う
        assert!(Loss::Pointer { id: 7, kept: false }.takes(&touching));
        // 別のポインタ、捕まえ直した（lParam が自分）→ 奪わない
        assert!(!Loss::Pointer { id: 8, kept: false }.takes(&touching));
        assert!(!Loss::Pointer { id: 7, kept: true }.takes(&touching));
        // マウスの捕まえ: ほかのウィンドウが捕まえたときだけ（普通のマウスの離しは奪わない）
        assert!(Loss::Mouse { elsewhere: true }.takes(&touching));
        assert!(!Loss::Mouse { elsewhere: false }.takes(&touching));
        // 輪の終わりは、触れていた押しを奪う
        assert!(Loss::ExitMove.takes(&touching));
    }

    #[test]
    fn a_stolen_press_is_released_and_the_next_press_starts_fresh() {
        let (mut touch, mut owner) = (Touch::default(), PressOwner::default());
        // 押しの始まり（持ち主は Windows Ink）→ 動く
        assert_eq!(
            ink(
                &mut touch,
                &mut owner,
                InkMessage::Down,
                sample(7, true, 5.0)
            )
            .len(),
            1
        );
        assert_eq!(owner.current(), Some(crate::pen::wintab::Owner::Ink));
        assert_eq!(
            ink(
                &mut touch,
                &mut owner,
                InkMessage::Update,
                sample(7, true, 9.0)
            )
            .len(),
            1
        );
        assert!(touch.is_touching());

        // OS が押しを奪った（離しは来ない）→ 最後の位置の離しが補われ、記録と持ち主が空になる
        let lift = seize(&mut touch, &mut owner, Loss::Pointer { id: 7, kept: false })
            .expect("触れていたので離しが補われる");
        assert!(!lift.contact);
        assert_eq!(lift.pressure, 0.0);
        assert_eq!(lift.pos, [9.0, 10.0], "最後の位置で離す");
        assert_eq!(lift.pointer_id, 7);
        assert!(!touch.is_touching());
        assert_eq!(owner.current(), None);

        // 同じ合図がもう一度来ても、離しは 2 度出ない
        assert_eq!(
            seize(&mut touch, &mut owner, Loss::ExitMove),
            None,
            "触れていない"
        );

        // 次の押し（別のポインタの番号）は新しい押しとして始まる（持ち主も取り直せる）
        let next = ink(
            &mut touch,
            &mut owner,
            InkMessage::Down,
            sample(9, true, 30.0),
        );
        assert_eq!(next.len(), 1, "次の押しの始まりは受け口へ入る");
        assert_eq!(owner.current(), Some(crate::pen::wintab::Owner::Ink));
        assert!(touch.is_touching());
        // その押しの普通の離しは、そのまま出る
        let up = ink(
            &mut touch,
            &mut owner,
            InkMessage::Up,
            sample(9, false, 31.0),
        );
        assert_eq!(up.len(), 1);
        assert!(!touch.is_touching());
        assert_eq!(owner.current(), None);
    }

    #[test]
    fn a_signal_that_does_not_take_the_press_changes_nothing() {
        let (mut touch, mut owner) = (Touch::default(), PressOwner::default());
        ink(
            &mut touch,
            &mut owner,
            InkMessage::Down,
            sample(7, true, 5.0),
        );
        for loss in [
            Loss::Pointer { id: 8, kept: false },
            Loss::Pointer { id: 7, kept: true },
            Loss::Mouse { elsewhere: false },
        ] {
            assert_eq!(seize(&mut touch, &mut owner, loss), None, "{loss:?}");
            assert!(touch.is_touching(), "{loss:?}");
            assert_eq!(
                owner.current(),
                Some(crate::pen::wintab::Owner::Ink),
                "{loss:?}"
            );
        }
        // 触れていなければ、奪う合図でも何も出ない
        let (mut idle, mut idle_owner) = (Touch::default(), PressOwner::default());
        assert_eq!(seize(&mut idle, &mut idle_owner, Loss::ExitMove), None);
    }

    #[test]
    fn a_press_owned_by_wintab_is_not_released_by_window_messages() {
        // WinTab が押しの持ち主のあいだ、Windows Ink の点は入らない（触れている記録は空）。ウィンドウのメッセージで WinTab の持ち主を手放さない
        let (mut touch, mut owner) = (Touch::default(), PressOwner::default());
        assert!(owner.admit_tab(true));
        assert_eq!(owner.current(), Some(crate::pen::wintab::Owner::WinTab));
        let kept = ink(
            &mut touch,
            &mut owner,
            InkMessage::Down,
            sample(7, true, 5.0),
        );
        assert!(
            kept.is_empty(),
            "WinTab の押しの間は Windows Ink の点を入れない"
        );
        assert_eq!(seize(&mut touch, &mut owner, Loss::ExitMove), None);
        assert_eq!(owner.current(), Some(crate::pen::wintab::Owner::WinTab));
    }

    #[test]
    fn the_log_line_names_the_message_and_what_was_done() {
        let touching = sample(7, true, 5.0);
        let line = log_loss(
            Loss::Pointer { id: 7, kept: false },
            Some(&touching),
            true,
            true,
        );
        assert_eq!(
            line,
            "loss msg=WM_POINTERCAPTURECHANGED id=7 kept=false touching=id=7 pos=(5.0,10.0) lifted=true egui-up=true"
        );
        let line = log_loss(Loss::ExitMove, None, false, false);
        assert_eq!(
            line,
            "loss msg=WM_EXITSIZEMOVE touching=none lifted=false egui-up=false"
        );
        let line = log_loss(
            Loss::Mouse { elsewhere: true },
            Some(&touching),
            true,
            false,
        );
        assert!(line.starts_with("loss msg=WM_CAPTURECHANGED elsewhere=true"));
    }

    const SIZE: [i32; 2] = [1200, 800];

    #[test]
    fn the_window_follows_the_pen_from_where_it_was_grabbed() {
        let mut moving = PenWindowMove::default();
        assert_eq!(moving.follow(7, true, [100.0, 100.0], SIZE), Follow::Idle);
        moving.begin(7, [500.0, 300.0], [1000, 200], SIZE);
        assert!(moving.is_active());
        // 掴んだ場所からのペンの動きが、そのままウィンドウの動きになる（負の向きも）
        assert_eq!(
            moving.follow(7, true, [510.4, 290.0], SIZE),
            Follow::To([1010, 190])
        );
        assert_eq!(
            moving.follow(7, true, [500.0, 300.0], SIZE),
            Follow::To([1000, 200]),
            "元へ戻せば元の位置"
        );
        // 別のペン（ポインタの番号が違う）の点は使わない
        assert_eq!(moving.follow(8, true, [900.0, 900.0], SIZE), Follow::Idle);
        assert!(moving.is_active());
        // 離しの点で終わる。以後は何もしない
        assert_eq!(moving.follow(7, false, [520.0, 310.0], SIZE), Follow::Ended);
        assert!(!moving.is_active());
        assert_eq!(moving.follow(7, true, [530.0, 320.0], SIZE), Follow::Idle);
    }

    #[test]
    fn rounding_does_not_accumulate_and_a_cancel_ends_the_grab() {
        let mut moving = PenWindowMove::default();
        moving.begin(1, [0.0, 0.0], [0, 0], SIZE);
        // 0.4 画素ずつ 10 回動いても、目標は掴んだ位置からの合計（4 画素）で、毎回の丸めを足し込まない
        let mut last = Follow::Idle;
        for i in 1..=10 {
            last = moving.follow(1, true, [0.4 * i as f64, 0.0], SIZE);
        }
        assert_eq!(last, Follow::To([4, 0]));
        assert!(moving.cancel());
        assert!(!moving.cancel(), "2 度目は掴んでいない");
        assert_eq!(moving.follow(1, true, [50.0, 0.0], SIZE), Follow::Idle);
        // 押しを奪う合図は、掴んでいるペンのものだけが掴みを捨てる
        moving.begin(1, [0.0, 0.0], [0, 0], SIZE);
        assert!(!moving.lose(Loss::Pointer { id: 2, kept: false }));
        assert!(!moving.lose(Loss::Pointer { id: 1, kept: true }));
        assert!(!moving.lose(Loss::Mouse { elsewhere: false }));
        assert!(moving.is_active());
        assert!(moving.lose(Loss::Pointer { id: 1, kept: false }));
        assert!(!moving.is_active());
        moving.begin(1, [0.0, 0.0], [0, 0], SIZE);
        assert!(moving.lose(Loss::ExitMove));
        // 極端な値でも溢れて回り込まない
        moving.begin(1, [0.0, 0.0], [i32::MAX - 1, i32::MIN + 1], SIZE);
        assert_eq!(
            moving.follow(1, true, [1.0e12, -1.0e12], SIZE),
            Follow::To([i32::MAX, i32::MIN])
        );
    }

    /// 掴みの間に別の拡大率の画面へまたいで、ウィンドウの大きさが変わったら、掴んだ点を幅の比で取り直す（150% → 100%、100% → 150%）。
    #[test]
    fn a_dpi_change_in_the_grab_keeps_the_same_spot_of_the_window_under_the_pen() {
        let mut moving = PenWindowMove::default();
        // 150% の画面: 幅 1200 のウィンドウの左上が (3000, 100)。帯の右寄り (3900, 130) を掴む → 差 (900, 30)
        moving.begin(7, [3900.0, 130.0], [3000, 100], [1200, 800]);
        assert_eq!(
            moving.follow(7, true, [3910.0, 140.0], [1200, 800]),
            Follow::To([3010, 110]),
            "大きさが同じなら、掴んだ差のまま"
        );
        // 100% の画面へ入って、ウィンドウの幅が 800 になった: 差は幅の比 (800/1200) で (600, 20) に取り直され、ペンは同じ相対位置の下
        assert_eq!(
            moving.follow(7, true, [1500.0, 140.0], [800, 533]),
            Follow::To([900, 120])
        );
        // 以後は取り直した差のまま
        assert_eq!(
            moving.follow(7, true, [1510.0, 150.0], [800, 533]),
            Follow::To([910, 130])
        );
        // 100% から 150% へ戻る: 差は (600, 20) → (900, 30)
        assert_eq!(
            moving.follow(7, true, [2000.0, 150.0], [1200, 800]),
            Follow::To([1100, 120])
        );
    }

    #[test]
    fn the_rescaled_offset_stays_inside_the_window_and_unknown_sizes_change_nothing() {
        let close = |got: [f64; 2], want: [f64; 2]| {
            assert!(
                (got[0] - want[0]).abs() < 1e-9 && (got[1] - want[1]).abs() < 1e-9,
                "{got:?} != {want:?}"
            );
        };
        // 比で拡大して、ウィンドウの中なら、そのまま比の分
        close(
            rescale_offset([1100.0, 700.0], [1200, 800], [2400, 1600]),
            [2200.0, 1400.0],
        );
        // 縮めたとき、高さの外へ出る差は、高さの中の端まで
        close(
            rescale_offset([900.0, 790.0], [1200, 800], [800, 100]),
            [600.0, 99.0],
        );
        // 負の差は 0 まで
        close(
            rescale_offset([-50.0, -5.0], [1200, 800], [800, 533]),
            [0.0, 0.0],
        );
        // 大きさが分からない（0 以下）ときは、そのまま
        close(
            rescale_offset([10.0, 20.0], [0, 0], [800, 600]),
            [10.0, 20.0],
        );
        close(
            rescale_offset([10.0, 20.0], [800, 600], [0, 0]),
            [10.0, 20.0],
        );
        // 同じ大きさ: そのまま
        close(
            rescale_offset([10.5, 20.5], [800, 600], [800, 600]),
            [10.5, 20.5],
        );
    }

    #[test]
    fn a_grab_starts_from_the_last_touching_point_however_old_it_is() {
        let last = LastPen {
            pointer: 3,
            contact: true,
            screen: [12.0, 34.0],
        };
        assert_eq!(last.source(), Some((3, [12.0, 34.0])));
        // 浮いているだけの点では掴まない
        let hover = LastPen {
            contact: false,
            ..last
        };
        assert_eq!(hover.source(), None);
    }

    #[test]
    fn a_taken_pointer_ignores_its_touching_points_until_the_next_down() {
        let mut taken = TakenIds::default();
        // 奪っていない番号は、そのまま入る
        assert!(taken.admit(false, &sample(7, true, 1.0)));
        taken.take(7);
        // 奪ったあと、同じ番号の触れた点は捨てる（補った離しのあとに押しが新しく始まらない）
        assert!(!taken.admit(false, &sample(7, true, 2.0)));
        assert!(!taken.admit(false, &sample(7, true, 3.0)));
        // ほかの番号は入る
        assert!(taken.admit(false, &sample(8, true, 2.0)));
        // 触れた最初のメッセージで、記録を消して入る。以後の点も入る
        assert!(taken.admit(true, &sample(7, true, 4.0)));
        assert!(taken.admit(false, &sample(7, true, 5.0)));
        // 触れていない点（本物の離し・浮いた点）も、記録を消して入る
        taken.take(7);
        assert!(taken.admit(false, &sample(7, false, 6.0)));
        assert!(taken.admit(false, &sample(7, true, 7.0)));
        // 覚える数には上限があり、古い番号から忘れる
        for id in 100..110 {
            taken.take(id);
        }
        assert!(
            taken.admit(false, &sample(100, true, 0.0)),
            "古い番号は忘れた"
        );
        assert!(!taken.admit(false, &sample(109, true, 0.0)));
    }

    #[test]
    fn a_start_drag_that_never_enters_the_loop_is_reported_once_after_the_wait() {
        let t0 = Instant::now();
        let mut watch = DragWatch::default();
        // 渡していなければ何も言わない
        assert!(!watch.stuck(t0 + START_WAIT * 5));
        // 渡して、待つ時間の前は言わない。過ぎたら 1 度だけ言う
        watch.handed(t0);
        assert!(!watch.stuck(t0 + START_WAIT - Duration::from_millis(1)));
        assert!(watch.stuck(t0 + START_WAIT));
        assert!(!watch.stuck(t0 + START_WAIT * 2), "言ったあとは戻る");
        // 渡し直すと、そこから数え直す
        watch.handed(t0 + START_WAIT * 2);
        assert!(!watch.stuck(t0 + START_WAIT * 2 + START_WAIT / 2));
        assert!(watch.stuck(t0 + START_WAIT * 3));
    }

    #[test]
    fn the_watch_stays_quiet_while_the_loop_is_alive_and_after_it_ended() {
        let t0 = Instant::now();
        let mut watch = DragWatch::default();
        // 輪が始まれば、どれだけ長くても言わない（生きている輪に WM_EXITSIZEMOVE を二重に送らない）
        watch.handed(t0);
        watch.entered();
        assert!(watch.in_loop());
        assert!(!watch.stuck(t0 + START_WAIT * 100));
        // 輪が生きている間に渡し直しても言わない
        watch.handed(t0 + START_WAIT * 100);
        assert!(!watch.stuck(t0 + START_WAIT * 200));
        // 終わったら、渡した記録も消える
        watch.exited();
        assert!(!watch.in_loop());
        assert!(!watch.stuck(t0 + START_WAIT * 300));
        // 終わったあとに渡して始まらなければ、また言う
        watch.handed(t0 + START_WAIT * 300);
        assert!(watch.stuck(t0 + START_WAIT * 301));
    }
}
