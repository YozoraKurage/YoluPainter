//! 3D ビューの表示の設定（Unity 版の `PreviewSceneSettings` と表示の切り替え）: 何を見せるか（マテリアル・中立・チャンネルだけ）、
//! 主な光（向き・強さ・色）と環境光、環境（空・スタジオ。明るさ・回転・背景とそのぼかし）、トーンマッピングと露出。画面だけの状態で、文書・.ylp には
//! 入らない。既定は Unity 版の照明（光の来る向き (−0.3, 0.65, −0.7)、環境光は灰色 0.5）。
//!
//! 切り替えは `Op`（`Action::View3d`）。メニュー（`entries`）とパネル（`panels::view3d`）が出す。

use yolu_core::glam::Vec3;
use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::Channel;

use super::brdf::Curve;
use super::environment::SkyColors;
use crate::lang::Lang;
use crate::state::{Action, AppState};
use crate::ui::menu::Entry;

/// 何を見せるか。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shading {
    /// 金属の流儀の PBR（Unity の Standard と同じ BRDF）。法線マップ・Roughness・Metallic・Emission も見える。
    Material,
    /// 中立（Unity 版のプレビューの簡単な明暗: 0.35 + 0.65 × saturate(n·L)）。
    Neutral,
    /// 1 つのチャンネルだけを、光・環境・トーンマッピングなしでそのまま（標準の 6 つ）。
    Channel(Channel),
    /// 今のテクスチャセットの焼いたメッシュマップ 1 枚だけを、光・環境・トーンマッピングなしでそのまま。
    MeshMap(MeshMapKind),
}

/// 環境の元。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum EnvKind {
    /// 使わない（一様な環境光）。
    None,
    /// 内蔵の空（Unity 版の既定の色の勾配）。
    #[default]
    Sky,
    /// 内蔵のスタジオ（面光源つき。金属の映り込みと回転が分かる）。
    Studio,
}

impl EnvKind {
    pub const ALL: [EnvKind; 3] = [EnvKind::None, EnvKind::Sky, EnvKind::Studio];

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            EnvKind::None => lang.pick("なし", "None"),
            EnvKind::Sky => lang.pick("空", "Sky"),
            EnvKind::Studio => lang.pick("スタジオ", "Studio"),
        }
    }
}

/// 表示の設定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    pub shading: Shading,
    /// 主な光の来る向き（方位 yaw と高さ pitch、度。Unity 版と同じ: (sin yaw cos pitch, sin pitch, cos yaw cos pitch)）。
    pub light_yaw: f32,
    pub light_pitch: f32,
    /// 主な光の強さ（1 が既定）と色。
    pub light_intensity: f32,
    pub light_color: [f32; 3],
    /// 環境光（灰色 0.5 が既定。中立の表示では 0.7 倍が影の側の明るさ、マテリアルの表示では 0.4 倍をリニアにしたもの）。
    pub ambient: [f32; 3],
    pub env: EnvKind,
    pub sky: SkyColors,
    /// 環境の明るさ（拡散と映り込みと背景に掛ける。1 が既定）。
    pub env_intensity: f32,
    /// 環境の向き（上の軸のまわりの度）。
    pub env_rotation: f32,
    /// 背景に環境を映すか（false なら背景の色）と、そのぼかし（0〜1。0 は元の解像度）。
    pub env_background: bool,
    pub env_blur: f32,
    pub tone_map: Curve,
    /// 露出（EV。+1 で 2 倍の明るさ）。
    pub exposure: f32,
    /// 主な光からモデル自身への影（光なしの表示では使わない）と、その柔らかさ（0〜1。0 は影のマップの 1 画素ぶん、1 はモデルの大きさの約 2.5%）。
    pub shadows: bool,
    pub shadow_softness: f32,
    /// 設定のパネルを開いているか（画面の状態。描き方には効かない）。
    pub settings_open: bool,
}

impl Default for Display {
    fn default() -> Self {
        let d = Vec3::new(-0.3, 0.65, -0.7).normalize();
        Display {
            shading: Shading::Material,
            light_yaw: d.x.atan2(d.z).to_degrees(),
            light_pitch: d.y.asin().to_degrees(),
            light_intensity: 1.0,
            light_color: [1.0; 3],
            ambient: [0.5; 3],
            env: EnvKind::Sky,
            sky: SkyColors::default(),
            env_intensity: 1.0,
            env_rotation: 0.0,
            env_background: false,
            env_blur: 0.3,
            tone_map: Curve::None,
            exposure: 0.0,
            shadows: false,
            shadow_softness: 0.25,
            settings_open: false,
        }
    }
}

impl Display {
    /// 光の来る向き（単位ベクトル）。
    pub fn light_direction(&self) -> Vec3 {
        let (y, p) = (self.light_yaw.to_radians(), self.light_pitch.to_radians());
        Vec3::new(y.sin() * p.cos(), p.sin(), y.cos() * p.cos())
    }

    /// 光なしの表示か（チャンネルだけ・メッシュマップだけ。光・環境・影・トーンマッピングを使わない）。
    pub fn is_unlit(&self) -> bool {
        matches!(self.shading, Shading::Channel(_) | Shading::MeshMap(_))
    }

    /// トーンマッピングか露出を当てるか（当てないときは 8 bit の描き先へ直に描く）。
    pub fn uses_tone_map(&self) -> bool {
        !self.is_unlit() && (self.tone_map != Curve::None || self.exposure.abs() > 1e-4)
    }

    /// 影を使うか（光なしの表示では使わない）。
    pub fn uses_shadows(&self) -> bool {
        self.shadows && !self.is_unlit()
    }

    /// 背景に環境を映すか。
    pub fn shows_background(&self) -> bool {
        !self.is_unlit() && self.env != EnvKind::None && self.env_background
    }

    /// 描き直しの鍵（描き方に効く値だけ。`settings_open` は含めない）。
    pub fn key_bits(&self) -> [u32; 20] {
        let (mode, channel) = match self.shading {
            Shading::Material => (0, 0),
            Shading::Neutral => (1, 0),
            Shading::Channel(c) => (2, c.index() as u32),
            Shading::MeshMap(k) => (3, k as u32),
        };
        [
            mode,
            channel,
            self.light_yaw.to_bits(),
            self.light_pitch.to_bits(),
            self.light_intensity.to_bits(),
            self.light_color[0].to_bits(),
            self.light_color[1].to_bits(),
            self.light_color[2].to_bits(),
            self.ambient[0].to_bits(),
            self.ambient[1].to_bits(),
            self.ambient[2].to_bits(),
            self.env as u32,
            self.env_intensity.to_bits(),
            self.env_rotation.to_bits(),
            u32::from(self.env_background),
            self.env_blur.to_bits(),
            self.tone_map as u32,
            self.exposure.to_bits(),
            u32::from(self.shadows),
            self.shadow_softness.to_bits(),
        ]
    }

    /// 値を当てる。範囲の外・NaN は端へ（壊れた値で描かない）。
    pub fn apply(&mut self, op: Op) {
        let finite = |v: f32, lo: f32, hi: f32, fallback: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                fallback
            }
        };
        match op {
            Op::Shading(s) => self.shading = s,
            Op::Env(k) => self.env = k,
            Op::EnvIntensity(v) => self.env_intensity = finite(v, 0.0, 8.0, 1.0),
            Op::EnvRotation(v) => self.env_rotation = finite(v, -180.0, 180.0, 0.0),
            Op::EnvBackground(b) => self.env_background = b,
            Op::EnvBlur(v) => self.env_blur = finite(v, 0.0, 1.0, 0.3),
            Op::Tone(c) => self.tone_map = c,
            Op::Exposure(v) => self.exposure = finite(v, -6.0, 6.0, 0.0),
            Op::Shadows(b) => self.shadows = b,
            Op::ShadowSoftness(v) => self.shadow_softness = finite(v, 0.0, 1.0, 0.25),
            Op::LightYaw(v) => self.light_yaw = finite(v, -180.0, 180.0, 0.0),
            Op::LightPitch(v) => self.light_pitch = finite(v, -89.0, 89.0, 0.0),
            Op::LightIntensity(v) => self.light_intensity = finite(v, 0.0, 4.0, 1.0),
            Op::ResetLighting => {
                let d = Display::default();
                self.light_yaw = d.light_yaw;
                self.light_pitch = d.light_pitch;
                self.light_intensity = d.light_intensity;
                self.light_color = d.light_color;
                self.ambient = d.ambient;
                self.env = d.env;
                self.env_intensity = d.env_intensity;
                self.env_rotation = d.env_rotation;
                self.env_background = d.env_background;
                self.env_blur = d.env_blur;
                self.tone_map = d.tone_map;
                self.exposure = d.exposure;
                self.shadows = d.shadows;
                self.shadow_softness = d.shadow_softness;
            }
            Op::ToggleSettings => self.settings_open = !self.settings_open,
            Op::CloseSettings => self.settings_open = false,
        }
    }
}

/// 表示の切り替え（`Action::View3d`）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Shading(Shading),
    Env(EnvKind),
    EnvIntensity(f32),
    EnvRotation(f32),
    EnvBackground(bool),
    EnvBlur(f32),
    Tone(Curve),
    Exposure(f32),
    Shadows(bool),
    ShadowSoftness(f32),
    LightYaw(f32),
    LightPitch(f32),
    LightIntensity(f32),
    /// 光・環境・トーンマッピングを既定へ（何を見せるかは変えない）。
    ResetLighting,
    ToggleSettings,
    CloseSettings,
}

/// トーンマッピングの曲線の名前。
pub fn tone_label(lang: Lang, curve: Curve) -> &'static str {
    match curve {
        Curve::None => lang.pick("なし", "None"),
        Curve::Neutral => lang.pick("ニュートラル", "Neutral"),
        Curve::Aces => "ACES",
    }
}

/// 何を見せるかの名前（見出しのドロップダウンに出す）。
pub fn shading_label(app: &AppState, shading: Shading) -> String {
    match shading {
        Shading::Material => app
            .lang
            .pick("マテリアル (PBR)", "Material (PBR)")
            .to_owned(),
        Shading::Neutral => app.lang.pick("中立", "Neutral").to_owned(),
        Shading::Channel(c) => crate::m2::channel_name(app.lang, &app.doc, c),
        Shading::MeshMap(k) => format!(
            "{}: {}",
            app.lang.pick("メッシュマップ", "Mesh Map"),
            crate::bake::kind_label(app.lang, k)
        ),
    }
}

/// 見せる 6 つのチャンネル（光なしの表示で選べるもの）。
pub const CHANNELS: [Channel; 6] = [
    Channel::Color,
    Channel::Roughness,
    Channel::Metallic,
    Channel::Normal,
    Channel::Emission,
    Channel::Height,
];

/// 表示のドロップダウンの項目（Unity 版と同じ並び: マテリアル・中立・チャンネルだけ・メッシュマップ）。
pub fn entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let current = app.view3d.display.shading;
    let mut v = vec![
        Entry::item(
            shading_label(app, Shading::Material),
            Action::View3d(Op::Shading(Shading::Material)),
        )
        .radio(current == Shading::Material),
        Entry::item(
            shading_label(app, Shading::Neutral),
            Action::View3d(Op::Shading(Shading::Neutral)),
        )
        .radio(current == Shading::Neutral),
        Entry::Separator,
        Entry::Heading(lang.pick("チャンネルだけ", "Channel Only").to_owned()),
    ];
    for c in CHANNELS {
        v.push(
            Entry::item(
                shading_label(app, Shading::Channel(c)),
                Action::View3d(Op::Shading(Shading::Channel(c))),
            )
            .radio(current == Shading::Channel(c)),
        );
    }
    v.push(Entry::Separator);
    // メッシュマップ: 今のテクスチャセットで焼いてあるものだけ選べる
    v.push(Entry::Heading(
        lang.pick("メッシュマップだけ", "Mesh Map Only").to_owned(),
    ));
    let maps = &app.sets.current().mesh_maps;
    for kind in MeshMapKind::ALL {
        v.push(
            Entry::item(
                crate::bake::kind_label(lang, kind),
                Action::View3d(Op::Shading(Shading::MeshMap(kind))),
            )
            .radio(current == Shading::MeshMap(kind))
            .enabled(maps.get(kind).is_some()),
        );
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_light_is_the_unity_preview_direction() {
        let d = Display::default().light_direction();
        let unity = Vec3::new(-0.3, 0.65, -0.7).normalize();
        assert!((d - unity).length() < 1e-5, "{d:?}");
    }

    #[test]
    fn tone_mapping_applies_only_when_asked_and_never_to_channel_views() {
        let mut d = Display::default();
        assert!(!d.uses_tone_map(), "既定は 8 bit の描き先へ直に");
        d.apply(Op::Exposure(1.0));
        assert!(d.uses_tone_map());
        d.apply(Op::Exposure(0.0));
        d.apply(Op::Tone(Curve::Aces));
        assert!(d.uses_tone_map());
        d.apply(Op::Shadows(true));
        assert!(d.uses_shadows());
        d.apply(Op::Shading(Shading::Channel(Channel::Roughness)));
        assert!(!d.uses_tone_map() && d.is_unlit());
        assert!(!d.shows_background(), "光なしの表示は環境を使わない");
        assert!(!d.uses_shadows(), "光なしの表示は影を使わない");
    }

    #[test]
    fn values_are_clamped_and_nan_is_refused() {
        let mut d = Display::default();
        d.apply(Op::Exposure(99.0));
        assert_eq!(d.exposure, 6.0);
        d.apply(Op::LightPitch(f32::NAN));
        assert_eq!(d.light_pitch, 0.0);
        d.apply(Op::EnvBlur(-3.0));
        assert_eq!(d.env_blur, 0.0);
        d.apply(Op::LightIntensity(100.0));
        assert_eq!(d.light_intensity, 4.0);
        d.apply(Op::ShadowSoftness(7.0));
        assert_eq!(d.shadow_softness, 1.0);
    }

    #[test]
    fn key_bits_change_with_the_picture_but_not_with_the_panel() {
        let mut d = Display::default();
        let key = d.key_bits();
        d.apply(Op::ToggleSettings);
        assert_eq!(d.key_bits(), key, "パネルの開閉は絵を変えない");
        d.apply(Op::EnvRotation(30.0));
        assert_ne!(d.key_bits(), key);
        let mut e = Display::default();
        e.apply(Op::Shading(Shading::Neutral));
        assert_ne!(e.key_bits(), key);
    }

    #[test]
    fn reset_restores_lighting_but_keeps_what_is_shown() {
        let mut d = Display::default();
        d.apply(Op::Shading(Shading::Neutral));
        d.apply(Op::Env(EnvKind::Studio));
        d.apply(Op::Tone(Curve::Neutral));
        d.apply(Op::LightYaw(10.0));
        d.apply(Op::ResetLighting);
        assert_eq!(d.shading, Shading::Neutral);
        assert_eq!(d.env, EnvKind::Sky);
        assert_eq!(d.tone_map, Curve::None);
        assert!((d.light_yaw - Display::default().light_yaw).abs() < 1e-5);
    }
}
