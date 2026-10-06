//! マテリアルで塗る（Substance Painter のように、1 回のストロークで複数のチャンネル）: 塗るチャンネルの組と、チャンネルごとの値
//! （Color は描画色、Emission は色、Roughness・Metallic・Height は 0〜1 の値、Normal は傾き）。オンなら、ブラシのストロークも
//! バケツ・ポリゴン塗りつぶしも組の全部を 1 回の Undo で塗る（core の `begin_material_brush_stroke`・`fill_material`・
//! `begin_material_triangle_fill`。層で無効のチャンネルは有効にする）。オフ（既定）なら描くチャンネル 1 つを描画色で塗る。
//! マスクの編集中は、どちらでもマスクだけを塗る。値は画面の状態で、文書には入らない（Unity 版の BrushState と同じ持ち方）。
//!
//! 組の最後の 1 つは外せない（何も塗らないストロークにしない）。初めてオンにしたときは、描くチャンネル 1 つの組で始める。

use yolu_core::material::ChannelPaint;

use crate::engine::{Channel, CoreError, Rgba8};
use crate::lang::Lang;
use crate::state::{to_byte, AppState, Rgba};

/// 組に選べるチャンネル（標準の 6 つ。番号の順）。
pub const CHANNELS: [Channel; 6] = Channel::ALL;

fn slot(channel: Channel) -> Option<usize> {
    CHANNELS.iter().position(|c| *c == channel)
}

/// マテリアルで塗る設定。
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialPaint {
    pub enabled: bool,
    /// 塗るチャンネルの組（`CHANNELS` の並び。全部 false はまだ選んでいない）。
    pub channels: [bool; 6],
    pub emission: [f32; 3],
    pub roughness: f32,
    pub metallic: f32,
    pub height: f32,
    /// Normal の値: 傾き（−1〜1。Z は長さが 1 になるように決める）。0, 0 は平ら。
    pub normal: [f32; 2],
}

impl Default for MaterialPaint {
    fn default() -> Self {
        MaterialPaint {
            enabled: false,
            channels: [false; 6],
            emission: [0.0; 3],
            roughness: 0.5,
            metallic: 0.0,
            height: 0.5,
            normal: [0.0; 2],
        }
    }
}

impl MaterialPaint {
    pub fn includes(&self, channel: Channel) -> bool {
        slot(channel).is_some_and(|i| self.channels[i])
    }

    /// 組のチャンネル（番号の順）。
    pub fn included(&self) -> Vec<Channel> {
        CHANNELS
            .iter()
            .copied()
            .filter(|c| self.includes(*c))
            .collect()
    }

    /// オン・オフ。初めてオンにしたとき（組が空）は、描くチャンネル 1 つの組で始める（標準でなければ Color）。
    pub fn set_enabled(&mut self, on: bool, current: Channel) {
        self.enabled = on;
        if on && self.channels.iter().all(|c| !c) {
            let start = slot(current).unwrap_or(0);
            self.channels[start] = true;
        }
    }

    /// 組にチャンネルを足す・外す。最後の 1 つは外さない（false を返す）。
    pub fn set_channel(&mut self, channel: Channel, on: bool) -> bool {
        let Some(i) = slot(channel) else {
            return false;
        };
        let mut next = self.channels;
        next[i] = on;
        if next.iter().all(|c| !c) {
            return false;
        }
        self.channels = next;
        true
    }

    /// データのチャンネル（Roughness・Metallic・Height）の値。
    pub fn scalar(&self, channel: Channel) -> Option<f32> {
        match channel {
            Channel::Roughness => Some(self.roughness),
            Channel::Metallic => Some(self.metallic),
            Channel::Height => Some(self.height),
            _ => None,
        }
    }

    pub fn set_scalar(&mut self, channel: Channel, value: f32) {
        let v = value.clamp(0.0, 1.0);
        match channel {
            Channel::Roughness => self.roughness = v,
            Channel::Metallic => self.metallic = v,
            Channel::Height => self.height = v,
            _ => {}
        }
    }

    /// 傾きを決める（長さが 1 を超える傾きは、向きを保って 1 に縮める）。
    pub fn set_normal(&mut self, x: f32, y: f32) {
        let (mut x, mut y) = (x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0));
        let l2 = x * x + y * y;
        if l2 > 1.0 {
            let l = l2.sqrt();
            x /= l;
            y /= l;
        }
        self.normal = [x, y];
    }

    /// チャンネルの値（straight RGBA8）。アルファは描画色のアルファ。Color は描画色、Emission は色、データは灰色、Normal は傾きの法線。
    pub fn value(&self, channel: Channel, color: Rgba) -> Rgba8 {
        let a = color[3];
        let rgba =
            |r: f32, g: f32, b: f32| Rgba8::new(to_byte(r), to_byte(g), to_byte(b), to_byte(a));
        match channel {
            Channel::Color => rgba(color[0], color[1], color[2]),
            Channel::Emission => rgba(self.emission[0], self.emission[1], self.emission[2]),
            Channel::Normal => {
                let (x, y) = (self.normal[0], self.normal[1]);
                let z = (1.0 - (x * x + y * y)).max(0.0).sqrt();
                rgba(x * 0.5 + 0.5, y * 0.5 + 0.5, z * 0.5 + 0.5)
            }
            other => {
                let v = self.scalar(other).unwrap_or(0.0);
                rgba(v, v, v)
            }
        }
    }

    /// 組のチャンネルとその値（番号の順。組が空なら空）。
    pub fn paints(&self, color: Rgba) -> Vec<ChannelPaint> {
        self.included()
            .into_iter()
            .map(|c| ChannelPaint::new(c, self.value(c, color)))
            .collect()
    }
}

/// マテリアルの操作（画面の状態だけ。文書は変えない）。値のスライダーは画面が `AppState::mat` を直に変える。
#[derive(Clone, Debug, PartialEq)]
pub enum MatAction {
    Enabled(bool),
    /// 組にチャンネルを足す・外す。
    Channel(Channel, bool),
    NormalFlat,
    /// 描画色を Emission の値にする。
    EmissionFromPaint,
    Emission([f32; 3]),
}

/// 1 つのチャンネルを描画色で塗るときの値（描画色の RGBA をそのまま。種類によらない）。ブラシのストロークがチャンネルへ渡す値
/// （core は色の変化を外すだけで、スカラー・Normal のチャンネルでも描画色の RGB のまま描く）と、Unity 版のバケツ・ポリゴン塗りつぶし
/// （`BrushColor32()`）に合わせる。明るさの灰色にすると、同じ画面の同じ設定でも、ブラシとバケツで値が違ってしまう。
pub fn single_value(color: Rgba) -> Rgba8 {
    let b = to_byte;
    Rgba8::new(b(color[0]), b(color[1]), b(color[2]), b(color[3]))
}

/// 効いているロックの名前（「すべて」が付いていればそれだけ。複数なら「、」でつなぐ）。名前の表は `layerops::lock_name` の 1 つだけで、
/// レイヤーの欄の錠の印・プロパティ・ロックの付け外しの状態の文と、断りの文が同じ言い方になる。
pub fn lock_names(lang: Lang, lock: yolu_core::LayerLocks) -> String {
    use yolu_core::LayerLocks as L;
    if lock.contains(L::ALL) {
        return crate::layerops::lock_name(lang, L::ALL).into();
    }
    crate::layerops::lock_names(lang, lock).join(lang.pick("、", ", "))
}

/// 断られた理由の短い文（core のエラーを、画面の言語で名前と理由だけにする）。
pub fn refusal_text(lang: Lang, error: &CoreError) -> String {
    match error {
        CoreError::LayerLocked {
            layer,
            holder,
            lock,
        } => format!(
            "{}: {}",
            if layer == holder {
                lang.pick("レイヤーがロックされています", "The layer is locked")
            } else {
                lang.pick("親グループがロックされています", "A parent group is locked")
            },
            lock_names(lang, *lock)
        ),
        CoreError::Cancelled => lang.pick("取り消しました", "Cancelled").to_owned(),
        CoreError::StrokeActive | CoreError::NoActiveStroke => lang
            .pick("描いている間はできません", "Not while drawing")
            .to_owned(),
        CoreError::LayerNotFound => lang
            .pick("レイヤーがありません", "No such layer")
            .to_owned(),
        CoreError::ChannelNotFound => lang
            .pick("チャンネルがありません", "No such channel")
            .to_owned(),
        CoreError::SourceBudgetExceeded | CoreError::StrokeBudgetExceeded => lang
            .pick("メモリの予算を超えます", "Over the memory budget")
            .to_owned(),
        CoreError::WorkingBudgetExceeded => lang
            .pick("作業のメモリを超えます", "Over the working memory")
            .to_owned(),
        other => lang.core_error(other),
    }
}

impl AppState {
    /// 次に塗る（バケツ・ポリゴン塗りつぶし）チャンネルとその値。マテリアルがオフなら描くチャンネル 1 つを描画色で。
    pub fn paint_channels(&self) -> Vec<ChannelPaint> {
        let color = self.color.main;
        if self.mat.enabled {
            let paints = self.mat.paints(color);
            if !paints.is_empty() {
                return paints;
            }
        }
        vec![ChannelPaint::new(
            self.m2.paint_channel,
            single_value(color),
        )]
    }

    /// マテリアルで塗る設定を使うストロークか（マスクに描くあいだは使わない）。
    pub fn paints_material(&self) -> bool {
        self.mat.enabled && !self.m2.edit_mask
    }

    /// マテリアルの操作を当てる（描いている間は断る）。
    pub fn mat_apply(&mut self, action: MatAction) {
        let lang = self.lang;
        if self.is_stroking() {
            self.message = lang
                .pick("描いている間はできません。", "Not while drawing.")
                .into();
            return;
        }
        match action {
            MatAction::Enabled(on) => {
                let current = self.m2.paint_channel;
                self.mat.set_enabled(on, current);
            }
            MatAction::Channel(channel, on) => {
                if !self.mat.set_channel(channel, on) {
                    self.message = lang
                        .pick(
                            "マテリアルは 1 つ以上のチャンネルを塗ります",
                            "A material paints at least one channel",
                        )
                        .into();
                }
            }
            MatAction::NormalFlat => self.mat.normal = [0.0; 2],
            MatAction::EmissionFromPaint => {
                let c = self.color.main;
                self.mat.emission = [c[0], c[1], c[2]];
            }
            MatAction::Emission(rgb) => self.mat.emission = rgb.map(|v| v.clamp(0.0, 1.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_enable_starts_from_the_paint_channel_and_the_last_channel_stays() {
        let mut m = MaterialPaint::default();
        m.set_enabled(true, Channel::Roughness);
        assert_eq!(m.included(), vec![Channel::Roughness]);
        assert!(
            !m.set_channel(Channel::Roughness, false),
            "最後の 1 つは外せない"
        );
        assert!(m.set_channel(Channel::Color, true));
        assert!(m.set_channel(Channel::Roughness, false));
        assert_eq!(m.included(), vec![Channel::Color]);
        // 組が残っていれば、オフ → オンで組は変わらない
        m.set_enabled(false, Channel::Metallic);
        m.set_enabled(true, Channel::Metallic);
        assert_eq!(m.included(), vec![Channel::Color]);
        // 標準でないチャンネルから始めるときは Color
        let mut n = MaterialPaint::default();
        n.set_enabled(true, Channel::from_index(9).unwrap());
        assert_eq!(n.included(), vec![Channel::Color]);
    }

    #[test]
    fn values_match_the_single_channel_bytes() {
        let mut m = MaterialPaint::default();
        let color = [1.0, 0.5, 0.0, 0.5];
        assert_eq!(m.value(Channel::Color, color), Rgba8::new(255, 128, 0, 128));
        m.set_scalar(Channel::Roughness, 0.25);
        assert_eq!(
            m.value(Channel::Roughness, color),
            Rgba8::new(64, 64, 64, 128),
            "アルファは描画色のもの"
        );
        m.emission = [0.0, 1.0, 0.0];
        assert_eq!(
            m.value(Channel::Emission, color),
            Rgba8::new(0, 255, 0, 128)
        );
        // 平らな法線は (128, 128, 255)、傾きは長さ 1 に収める
        assert_eq!(
            m.value(Channel::Normal, [0.0, 0.0, 0.0, 1.0]),
            Rgba8::new(128, 128, 255, 255)
        );
        m.set_normal(1.0, 1.0);
        let l = (m.normal[0] * m.normal[0] + m.normal[1] * m.normal[1]).sqrt();
        assert!((l - 1.0).abs() < 1e-6, "{:?}", m.normal);
        let n = m.value(Channel::Normal, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(n.b, 128, "z = 0");
        // 範囲の外の値は丸める
        m.set_scalar(Channel::Metallic, 2.0);
        assert_eq!(m.metallic, 1.0);
    }

    #[test]
    fn paints_follow_the_channel_order_and_skip_the_rest() {
        let mut m = MaterialPaint::default();
        m.set_enabled(true, Channel::Emission);
        m.set_channel(Channel::Color, true);
        m.set_channel(Channel::Height, true);
        let order: Vec<Channel> = m
            .paints([0.0, 0.0, 0.0, 1.0])
            .iter()
            .map(|p| p.channel)
            .collect();
        assert_eq!(
            order,
            vec![Channel::Color, Channel::Height, Channel::Emission]
        );
    }
}
