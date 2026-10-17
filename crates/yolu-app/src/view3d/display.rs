//! 3D ビューの表示の設定（Unity 版の `PreviewSceneSettings` と表示の切り替え）: 何を見せるか（マテリアル・中立・チャンネルだけ）、
//! 主な光（向き・強さ・色）と環境光、環境（空・スタジオ。明るさ・回転・背景とそのぼかし）、トーンマッピングと露出、仕上げ（アンチエイリアス・ブルーム）。
//! 画面だけの状態で、文書・.ylp には入らない（仕上げだけは、機材と好みの設定なので設定のファイルに保存する。`PostFx`）。既定の光は、モデルの前（+Z）の
//! 斜め上から当たる向き（Unity 版の既定の向きを Z で折り返したもの）、環境光は灰色 0.5。
//!
//! 切り替えは `Op`（`Action::View3d`）。メニュー（`entries`）とパネル（`panels::view3d`）が出す。

use yolu_core::glam::Vec3;
use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::Channel;

use super::brdf::Curve;
use super::environment::SkyColors;
use super::{ViewId, ViewState};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::{Action, AppState};
use crate::ui::menu::Entry;

/// `Display::key_bits` の長さ。
pub const KEY_BITS: usize = 24;

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
            EnvKind::None => lang.t("view3d-none"),
            EnvKind::Sky => lang.t("view3d-sky"),
            EnvKind::Studio => lang.t("view3d-studio"),
        }
    }
}

/// アンチエイリアス（MSAA）に選べるサンプル数。1 は切。
pub const SAMPLE_CHOICES: [u32; 4] = [1, 2, 4, 8];
/// アンチエイリアスの既定のサンプル数。
pub const DEFAULT_SAMPLES: u32 = 4;
/// ブルームの強さの範囲と既定（0 は何も足さない）。
pub const BLOOM_STRENGTH_MAX: f32 = 2.0;
pub const DEFAULT_BLOOM_STRENGTH: f32 = 0.6;
/// ブルームのしきい値の範囲と既定。光と露出を掛けたあとのリニアの明るさ（1 が白）で、これを超えた分がにじむ。
/// 標準のマテリアルの発光は 8 bit（最大 1）なので、既定は 1 より少し下げて、白に近い発光が光って見えるようにする。
pub const BLOOM_THRESHOLD_MAX: f32 = 4.0;
pub const DEFAULT_BLOOM_THRESHOLD: f32 = 0.8;

/// 3D の絵の仕上げ（アンチエイリアスとブルーム）。GPU と好みで決まる設定なので、文書でなく設定のファイルに保存する。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostFx {
    /// アンチエイリアスのサンプル数（`SAMPLE_CHOICES` のどれか。1 は切）。選んだ値で、機材が対応しない数・描き先のメモリの上限を超える数は、
    /// 描くときに対応する最大の数へ下げる（保存した値は変えない）。
    pub antialias: u32,
    /// ブルームを使うか。
    pub bloom: bool,
    pub bloom_strength: f32,
    pub bloom_threshold: f32,
}

impl Default for PostFx {
    fn default() -> Self {
        PostFx {
            antialias: DEFAULT_SAMPLES,
            bloom: false,
            bloom_strength: DEFAULT_BLOOM_STRENGTH,
            bloom_threshold: DEFAULT_BLOOM_THRESHOLD,
        }
    }
}

/// 機材が対応する数（`supported`。昇順）のうち、選んだ数 `want` 以下でいちばん大きいもの（無ければ 1）。
pub fn clamp_samples(want: u32, supported: &[u32]) -> u32 {
    supported
        .iter()
        .copied()
        .filter(|n| *n <= want)
        .max()
        .unwrap_or(1)
        .max(1)
}

/// 設定のパネルの 3 つの面。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsTab {
    /// 光・環境・トーンマッピング。
    #[default]
    Display,
    /// アンチエイリアスとブルーム。
    Quality,
    /// 視点（回転・ズームの中心）。
    Navigation,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 3] = [
        SettingsTab::Display,
        SettingsTab::Quality,
        SettingsTab::Navigation,
    ];

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            SettingsTab::Display => lang.t("view3d-display"),
            SettingsTab::Quality => lang.t("view3d-quality"),
            SettingsTab::Navigation => lang.t("view3d-navigation"),
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
    /// 仕上げ（アンチエイリアス・ブルーム。設定のファイルへ保存する）。
    pub post: PostFx,
    /// 設定のパネルを開いているか（画面の状態。描き方には効かない）。
    pub settings_open: bool,
    /// 設定のパネルで開いている面（画面の状態）。
    pub tab: SettingsTab,
    /// 仕上げのスライダーをドラッグしている最中か（画面の状態。離すまで設定のファイルへ書かない）。
    pub post_dragging: bool,
    /// 最後に描いたサンプル数（画面の状態。選びを機材・メモリの上限で下げたもの。まだ描いていなければ 0）。
    pub drawn_samples: u32,
    /// この機材が描き先に使えるサンプル数（`SAMPLE_CHOICES` の並びの順に 1 ビットずつ。描き手がつながったときに入れる。1 は常に使える）。
    pub supported_samples: u8,
}

/// 主な光の既定の向き（光の来る向き）。モデルの前（+Z）の斜め上から: Unity 版の既定 (−0.3, 0.65, −0.7) を Z で折り返した (−0.3, 0.65, +0.7)
/// で、高さ（約 40°）と横へのずれ（約 23°）は前のまま、来る側だけが背中側から正面側になる。
/// モデルは +Z を向くのが Unity の約束（アバターも同じ）で、前から見たときに逆光にならない。
/// 保存された設定には光の向きを含めない（画面だけの状態）ので、変わるのは新しく開いたときの既定だけ。
pub const DEFAULT_LIGHT: Vec3 = Vec3::new(-0.3, 0.65, 0.7);

impl Default for Display {
    fn default() -> Self {
        let d = DEFAULT_LIGHT.normalize();
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
            post: PostFx::default(),
            settings_open: false,
            tab: SettingsTab::Display,
            post_dragging: false,
            drawn_samples: 0,
            supported_samples: 1,
        }
    }
}

impl Display {
    /// 機材が使えるサンプル数を入れる（`SAMPLE_CHOICES` に無い数は無視。1 は常に使える）。
    pub fn set_supported_samples(&mut self, counts: &[u32]) {
        self.supported_samples = SAMPLE_CHOICES
            .iter()
            .enumerate()
            .filter(|(_, n)| **n == 1 || counts.contains(n))
            .fold(0, |mask, (i, _)| mask | 1 << i);
    }

    /// 機材が使えるサンプル数（昇順。1 を含む）。
    pub fn supported_sample_counts(&self) -> Vec<u32> {
        SAMPLE_CHOICES
            .iter()
            .enumerate()
            .filter(|(i, _)| self.supported_samples >> i & 1 == 1)
            .map(|(_, n)| *n)
            .collect()
    }

    /// 画面に出すアンチエイリアスの選び: 選んだ数を、機材が使える最大の数へ下げたもの。
    pub fn shown_samples(&self) -> u32 {
        clamp_samples(self.post.antialias, &self.supported_sample_counts())
    }

    /// 光の来る向き（単位ベクトル）。
    pub fn light_direction(&self) -> Vec3 {
        let (y, p) = (self.light_yaw.to_radians(), self.light_pitch.to_radians());
        Vec3::new(y.sin() * p.cos(), p.sin(), y.cos() * p.cos())
    }

    /// マテリアル表示の主な光（リニアの放射輝度）。Unity のビルトインのレンダーパイプラインのディレクショナルライトの `_LightColor0` と
    /// 同じ値: 色 × 強さを、ライトの強さをリニアで掛けない既定の設定（`GraphicsSettings.lightsUseLinearIntensity` が偽）どおり
    /// GammaToLinearSpace でリニアへ（1 を超える値は pow 2.2）。白・強さ 1 で 1 になり、Unity の場面の白・強さ 1 のライトと同じ明るさ。
    pub fn direct_light(&self) -> [f32; 3] {
        self.light_color
            .map(|c| super::brdf::unity_gamma_to_linear(c * self.light_intensity))
    }

    /// 光なしの表示か（チャンネルだけ・メッシュマップだけ。光・環境・影・トーンマッピングを使わない）。
    pub fn is_unlit(&self) -> bool {
        matches!(self.shading, Shading::Channel(_) | Shading::MeshMap(_))
    }

    /// トーンマッピングか露出を当てるか。
    pub fn uses_tone_map(&self) -> bool {
        !self.is_unlit() && (self.tone_map != Curve::None || self.exposure.abs() > 1e-4)
    }

    /// ブルームを足すか（光なしの表示・強さ 0 では足さない）。
    pub fn uses_bloom(&self) -> bool {
        !self.is_unlit() && self.post.bloom && self.post.bloom_strength > 0.0
    }

    /// HDR の描き先へ描いて、仕上げ（ブルーム・露出・トーンマッピング）を通すか（通さないときは 8 bit の描き先へ直に描く）。
    pub fn uses_hdr_path(&self) -> bool {
        self.uses_tone_map() || self.uses_bloom()
    }

    /// 影を使うか（光なしの表示では使わない）。
    pub fn uses_shadows(&self) -> bool {
        self.shadows && !self.is_unlit()
    }

    /// 背景に環境を映すか。
    pub fn shows_background(&self) -> bool {
        !self.is_unlit() && self.env != EnvKind::None && self.env_background
    }

    /// 描き直しの鍵（描き方に効く値だけ。パネルの状態は含めない）。
    pub fn key_bits(&self) -> [u32; KEY_BITS] {
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
            self.post.antialias,
            u32::from(self.post.bloom),
            self.post.bloom_strength.to_bits(),
            self.post.bloom_threshold.to_bits(),
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
            Op::Shading(s) | Op::ShadingOf(_, s) => self.shading = s,
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
            // 選べる数でなければ、それ以下でいちばん大きい選べる数（0 は 1 = 切）
            Op::Antialias(n) => {
                self.post.antialias = SAMPLE_CHOICES
                    .into_iter()
                    .filter(|c| *c <= n)
                    .max()
                    .unwrap_or(1);
            }
            Op::Bloom(b) => self.post.bloom = b,
            Op::BloomStrength(v) => {
                self.post.bloom_strength =
                    finite(v, 0.0, BLOOM_STRENGTH_MAX, DEFAULT_BLOOM_STRENGTH)
            }
            Op::BloomThreshold(v) => {
                self.post.bloom_threshold =
                    finite(v, 0.0, BLOOM_THRESHOLD_MAX, DEFAULT_BLOOM_THRESHOLD)
            }
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
                // ブルームは見た目の一部なので戻す。アンチエイリアスは画質の選びなので戻さない
                self.post.bloom = d.post.bloom;
                self.post.bloom_strength = d.post.bloom_strength;
                self.post.bloom_threshold = d.post.bloom_threshold;
            }
            Op::ToggleSettings => {
                self.settings_open = !self.settings_open;
                self.post_dragging = false;
            }
            Op::CloseSettings => {
                self.settings_open = false;
                self.post_dragging = false;
            }
        }
    }
}

/// 表示の切り替え（`Action::View3d`）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// 今のビューの表示の種類。
    Shading(Shading),
    /// このビューの表示の種類（ビューの左上のドロップダウンのメニュー。メニューを開いたビューへ当てる）。
    ShadingOf(super::ViewId, Shading),
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
    /// アンチエイリアスのサンプル数（`SAMPLE_CHOICES`。1 は切）。
    Antialias(u32),
    Bloom(bool),
    BloomStrength(f32),
    BloomThreshold(f32),
    /// 光・環境・トーンマッピング・ブルームを既定へ（何を見せるか・アンチエイリアスは変えない）。
    ResetLighting,
    ToggleSettings,
    CloseSettings,
}

/// トーンマッピングの曲線の名前。
pub fn tone_label(lang: Lang, curve: Curve) -> &'static str {
    match curve {
        Curve::None => lang.t("view3d-none"),
        Curve::Neutral => lang.t("view3d-neutral"),
        Curve::Aces => "ACES",
    }
}

/// 何を見せるかの名前（見出しのドロップダウンに出す）。
pub fn shading_label(app: &AppState, shading: Shading) -> String {
    match shading {
        Shading::Material => app.lang.t("view3d-material-pbr").to_owned(),
        Shading::Neutral => app.lang.t("view3d-neutral-2").to_owned(),
        Shading::Channel(c) => crate::m2::channel_name(app.lang, &app.session.doc, c),
        Shading::MeshMap(k) => format!(
            "{}: {}",
            app.lang.t("view3d-mesh-map"),
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

/// 表示のドロップダウンの項目（Unity 版と同じ並び: マテリアル・中立・チャンネルだけ・メッシュマップ）。メニューを開いたビュー
/// （`View3dState::shading_menu`）の表示の種類に印を付け、選んだ種類をそのビューへ当てる。チャンネルだけは標準の 6 つの後に、今のセットの
/// ユーザーチャンネル。メッシュマップは、どれかのセットで焼いてあるものだけ選べる。
pub fn entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let view = app.view3d.shading_menu;
    let current = app
        .view3d
        .view_of(view)
        .map_or(app.view3d.view.display.shading, |v| v.display.shading);
    let item = |shading: Shading, label: String| {
        Entry::item(label, Action::View3d(Op::ShadingOf(view, shading))).radio(current == shading)
    };
    let mut v = vec![
        item(Shading::Material, shading_label(app, Shading::Material)),
        item(Shading::Neutral, shading_label(app, Shading::Neutral)),
        Entry::Separator,
        Entry::Heading(lang.t("view3d-channel-only").to_owned()),
    ];
    for c in CHANNELS.into_iter().chain(user_channels(&app.session.doc)) {
        v.push(item(
            Shading::Channel(c),
            shading_label(app, Shading::Channel(c)),
        ));
    }
    v.push(Entry::Separator);
    v.push(Entry::Heading(lang.t("view3d-mesh-map-only").to_owned()));
    for kind in MeshMapKind::ALL {
        v.push(
            item(
                Shading::MeshMap(kind),
                crate::bake::kind_label(lang, kind).to_owned(),
            )
            .enabled(super::views::mesh_map_available(app, kind)),
        );
    }
    v
}

/// 文書のユーザーチャンネル（番号の順）。
pub fn user_channels(doc: &yolu_core::Document) -> impl Iterator<Item = Channel> + '_ {
    (Channel::STANDARD_COUNT..Channel::MAX)
        .filter_map(Channel::from_index)
        .filter(|c| doc.channel_info(*c).is_some())
}

/// タブの見出しに出す、表示の種類の短い名前（マテリアルは「(PBR)」を付けない。メッシュマップは種類の名前だけ）。
pub fn shading_short_label(app: &AppState, shading: Shading) -> String {
    match shading {
        Shading::Material => app.lang.t("view3d-material-short").to_owned(),
        Shading::MeshMap(k) => crate::bake::kind_label(app.lang, k).to_owned(),
        other => shading_label(app, other),
    }
}

/// 設定のパネルの面の切り替えを描き、視点の面を開いているか（開いていれば内容もここで描いた）を返す。`settings_tabs` の、視点だけを見る形。
pub fn navigation_settings(
    ui: &mut egui::Ui,
    app: &mut AppState,
    rows: &mut crate::ui::widgets::Rows,
    view: egui::Rect,
) -> bool {
    settings_tabs(ui, app, rows, view) == SettingsTab::Navigation
}

/// 設定のパネルの面の切り替え（表示・画質・視点）を描き、開いている面を返す。視点の内容はこの側で描く（表示と画質の内容は呼び手が描く）。
pub fn settings_tabs(
    ui: &mut egui::Ui,
    app: &mut AppState,
    rows: &mut crate::ui::widgets::Rows,
    view: egui::Rect,
) -> SettingsTab {
    use super::navigation::{self, OrbitCenter, ZoomCenter};
    use crate::ui::{theme as t, widgets as w};
    let lang = app.lang;
    let r = rows.row(24.0, 4.0);
    for (tab, cell) in SettingsTab::ALL.into_iter().zip(w::Rows::split(r, 3, 4.0)) {
        if w::button(
            ui,
            cell,
            ("view3d.settings.tab", tab as u8),
            tab.label(lang),
            app.view3d.view.display.tab == tab,
            true,
            None,
            None,
        )
        .clicked()
        {
            app.view3d.view.display.tab = tab;
            app.view3d.view.display.post_dragging = false;
        }
    }
    if app.view3d.view.display.tab != SettingsTab::Navigation {
        return app.view3d.view.display.tab;
    }
    let r = rows.row(22.0, 4.0);
    w::text(
        ui.painter(),
        r,
        lang.t("view3d-orbit-center"),
        t::LABEL_BOLD,
        w::Align::Left,
    );
    for center in OrbitCenter::ALL {
        let r = rows.row(26.0, 4.0);
        if w::button(
            ui,
            r,
            ("view3d.orbit", center as u8),
            center.label(lang),
            app.prefs.settings.navigation.orbit == center,
            true,
            None,
            None,
        )
        .clicked()
        {
            app.prefs.settings.navigation.orbit = center;
        }
    }
    rows.space(6.0);
    let r = rows.row(22.0, 4.0);
    w::text(
        ui.painter(),
        r,
        lang.t("view3d-zoom-center"),
        t::LABEL_BOLD,
        w::Align::Left,
    );
    for center in ZoomCenter::ALL {
        let r = rows.row(26.0, 4.0);
        if w::button(
            ui,
            r,
            ("view3d.zoom", center as u8),
            center.label(lang),
            app.prefs.settings.navigation.zoom == center,
            true,
            None,
            None,
        )
        .clicked()
        {
            app.prefs.settings.navigation.zoom = center;
        }
    }
    rows.space(6.0);
    let r = rows.row(26.0, 4.0);
    if w::button(
        ui,
        r,
        "view3d.frame_selected",
        lang.t("view3d-frame-selected"),
        false,
        !app.is_stroking() && navigation::selected_bounds(&app.view3d).is_some(),
        Some(&crate::shortcuts::named_with_keys(
            lang,
            lang.t("view3d-frame-selected-tip"),
            &[crate::shortcuts::key_in("view3d.frame_selected", app.mode)],
        )),
        Some("target"),
    )
    .clicked()
    {
        navigation::frame_selected(app, view);
    }
    SettingsTab::Navigation
}

/// 表示の切り替え（`Action::View3d`）を当てる。ユーザーチャンネルを「チャンネルだけ」で見せるときは、3D に持つユーザーチャンネルの配列に
/// 入るか（`user_channel_room`）を先に確かめ、入らなければ断る（表示は替えない）。
pub fn apply_display(app: &mut AppState, op: Op) {
    let target = match op {
        Op::Shading(s) => Some((app.view3d.active_view(), s)),
        Op::ShadingOf(id, s) => Some((id, s)),
        _ => None,
    };
    if let Some((id, Shading::Channel(c))) = target {
        if !c.is_standard() {
            if let Err(reason) = user_channel_room(app, id, c) {
                let lang = app.lang;
                let what = lang.f(
                    "view3d-cannot-show-channel",
                    &[(
                        "name",
                        &lang.quote(&crate::m2::channel_name(lang, &app.session.doc, c)),
                    )],
                );
                app.refuse(Source::View3d, lang.with_reason(what, reason));
                return;
            }
        }
    }
    app.view3d.apply_display(op);
}

/// ユーザーチャンネル `c`（今のセットの文書の番号）を、ビュー `id` の「チャンネルだけ」で見せられるか: 全部のビューで同時に見せるユーザー
/// チャンネルが [`look_gpu::SHOWN_USERS`] 個まで、今のセットの配列（lilToon のスロットが読むものと合わせて）が
/// [`user_layers::MAX_LAYERS`] 個まで、その配列が元の大きさで 3D のユーザーチャンネルの予算（[`user_layers::USER_BUDGET_BYTES`]）に
/// 入ること。入らなければ理由。
pub fn user_channel_room(app: &AppState, id: ViewId, c: yolu_core::Channel) -> Result<(), String> {
    use super::{look_gpu, user_layers};
    let lang = app.lang;
    let doc = &app.session.doc;
    let Some(info) = doc.channel_info(c) else {
        return Err(lang.t("view3d-the-channel-is-not-in").to_owned());
    };
    // ほかのビューが見ている名前（このビューの今の表示は替わるので数えない）
    let mut names: Vec<String> = Vec::new();
    let mut views: Vec<&ViewState> = app.view3d.views().filter(|v| v.id != id).collect();
    views.sort_by_key(|v| v.id);
    for v in views {
        if let Shading::Channel(u) = v.display.shading {
            if let Some(i) = doc.channel_info(u).filter(|_| !u.is_standard()) {
                if !names.contains(&i.name) {
                    names.push(i.name.clone());
                }
            }
        }
    }
    if !names.contains(&info.name) {
        if names.len() >= look_gpu::SHOWN_USERS {
            return Err(lang.f(
                "view3d-up-to-user-channels-at-once",
                &[("count", &look_gpu::SHOWN_USERS)],
            ));
        }
        names.push(info.name.clone());
    }
    let users = look_gpu::array_users(doc, doc.drawn_look(), &names);
    if !users.contains(&c) {
        return Err(lang.f(
            "view3d-up-to-user-channels-in-3d",
            &[("count", &user_layers::MAX_LAYERS)],
        ));
    }
    if user_layers::full_bytes([doc.width(), doc.height()], users.len())
        > user_layers::USER_BUDGET_BYTES
    {
        return Err(lang.t("view3d-user-channels-exceed-budget").to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_light_comes_from_above_the_front_of_the_model() {
        let d = Display::default();
        let to_light = d.light_direction();
        // 前（+Z）の斜め上: Unity 版の既定の向きを Z で折り返した向き
        let mirrored = Vec3::new(-0.3, 0.65, 0.7).normalize();
        assert!((to_light - mirrored).length() < 1e-5, "{to_light:?}");
        assert!(
            to_light.z > 0.5 && to_light.y > 0.5,
            "前と上から: {to_light:?}"
        );
        // 高さと横へのずれは、前の既定（背中側の斜め上）と同じ
        let old = Vec3::new(-0.3, 0.65, -0.7).normalize();
        assert!((to_light.y - old.y).abs() < 1e-6 && (to_light.x - old.x).abs() < 1e-6);
        // 度の値: 方位 −23°・高さ 40.5° 前後
        assert!(
            (d.light_yaw + 23.2).abs() < 0.2 && (d.light_pitch - 40.5).abs() < 0.2,
            "{} {}",
            d.light_yaw,
            d.light_pitch
        );
    }

    #[test]
    fn a_light_direction_the_user_set_is_not_touched_by_the_new_default() {
        // 値は Op で入れた通りのまま（既定が変わっても、入れた値は上書きされない。既定へ戻すのは ResetLighting だけ）
        let mut d = Display::default();
        d.apply(Op::LightYaw(-156.8));
        d.apply(Op::LightPitch(40.5));
        assert_eq!((d.light_yaw, d.light_pitch), (-156.8, 40.5));
        d.apply(Op::Bloom(true));
        assert_eq!((d.light_yaw, d.light_pitch), (-156.8, 40.5));
        d.apply(Op::ResetLighting);
        assert!(d.light_direction().z > 0.0);
    }

    #[test]
    fn post_defaults_are_four_samples_and_no_bloom() {
        let d = Display::default();
        assert_eq!(d.post.antialias, 4);
        assert!(!d.post.bloom && !d.uses_bloom() && !d.uses_hdr_path());
        assert!(d.post.bloom_strength > 0.0 && d.post.bloom_threshold > 0.0);
    }

    #[test]
    fn bloom_needs_a_lit_view_and_a_strength_and_joins_the_hdr_path() {
        let mut d = Display::default();
        d.apply(Op::Bloom(true));
        assert!(
            d.uses_bloom() && d.uses_hdr_path() && !d.uses_tone_map(),
            "ブルームだけでも HDR の道を通る"
        );
        d.apply(Op::BloomStrength(0.0));
        assert!(
            !d.uses_bloom() && !d.uses_hdr_path(),
            "強さ 0 は何も足さない"
        );
        d.apply(Op::BloomStrength(1.0));
        d.apply(Op::Shading(Shading::Channel(Channel::Emission)));
        assert!(
            !d.uses_bloom(),
            "チャンネルだけの表示は光なし: ブルームを足さない"
        );
        d.apply(Op::Shading(Shading::MeshMap(MeshMapKind::ALL[0])));
        assert!(!d.uses_bloom());
        d.apply(Op::Shading(Shading::Neutral));
        assert!(d.uses_bloom());
    }

    #[test]
    fn antialias_takes_only_the_listed_counts_and_clamps_to_the_device() {
        let mut d = Display::default();
        for (given, want) in [
            (1, 1),
            (2, 2),
            (4, 4),
            (8, 8),
            (0, 1),
            (3, 2),
            (5, 4),
            (7, 4),
            (16, 8),
            (u32::MAX, 8),
        ] {
            d.apply(Op::Antialias(given));
            assert_eq!(d.post.antialias, want, "{given}");
        }
        // 機材が対応しない数は、それ以下で対応する最大の数へ（1 はいつでも）
        assert_eq!(clamp_samples(8, &[1, 2, 4]), 4);
        assert_eq!(clamp_samples(8, &[1, 4]), 4);
        assert_eq!(clamp_samples(2, &[1, 4]), 1);
        assert_eq!(clamp_samples(4, &[1]), 1);
        assert_eq!(clamp_samples(4, &[]), 1);
        assert_eq!(clamp_samples(1, &[1, 2, 4, 8]), 1);
        assert_eq!(clamp_samples(8, &[1, 2, 4, 8]), 8);
    }

    #[test]
    fn bloom_values_are_clamped_and_nan_is_refused() {
        let mut d = Display::default();
        d.apply(Op::BloomStrength(99.0));
        assert_eq!(d.post.bloom_strength, BLOOM_STRENGTH_MAX);
        d.apply(Op::BloomStrength(-1.0));
        assert_eq!(d.post.bloom_strength, 0.0);
        d.apply(Op::BloomStrength(f32::NAN));
        assert_eq!(d.post.bloom_strength, DEFAULT_BLOOM_STRENGTH);
        d.apply(Op::BloomThreshold(99.0));
        assert_eq!(d.post.bloom_threshold, BLOOM_THRESHOLD_MAX);
        // 有限でない値（NaN・無限大）は、端ではなく既定へ（壊れた値で描かない）
        d.apply(Op::BloomThreshold(f32::INFINITY));
        assert_eq!(d.post.bloom_threshold, DEFAULT_BLOOM_THRESHOLD);
        d.apply(Op::BloomThreshold(f32::NAN));
        assert_eq!(d.post.bloom_threshold, DEFAULT_BLOOM_THRESHOLD);
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
        // 仕上げの 4 つの値も絵を変える
        for op in [
            Op::Antialias(2),
            Op::Bloom(true),
            Op::BloomStrength(1.5),
            Op::BloomThreshold(2.0),
        ] {
            let mut e = Display::default();
            e.apply(op);
            assert_ne!(e.key_bits(), key, "{op:?}");
        }
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
        d.apply(Op::Bloom(true));
        d.apply(Op::BloomThreshold(2.5));
        d.apply(Op::Antialias(2));
        d.apply(Op::ResetLighting);
        assert!(
            !d.post.bloom && d.post.bloom_threshold == DEFAULT_BLOOM_THRESHOLD,
            "ブルームは見た目なので戻る"
        );
        assert_eq!(
            d.post.antialias, 2,
            "アンチエイリアスは画質の選びなので戻さない"
        );
        assert_eq!(d.shading, Shading::Neutral);
        assert_eq!(d.env, EnvKind::Sky);
        assert_eq!(d.tone_map, Curve::None);
        assert!((d.light_yaw - Display::default().light_yaw).abs() < 1e-5);
    }
}
