//! 署名付き更新情報の検証。HTTP は呼び出し側が実装し、ファイルの置換は行わない。
use ed25519_dalek::{Signature, VerifyingKey};
pub use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const RELEASE_BASE: &str = "https://github.com/YozoraKurage/YoluPainter/releases/download";
/// アプリが更新情報を取る場所。GitHub の「最新の Release」（下書き・プレリリースを除く最新）の資産へ飛ぶ。
/// ファイル名に schema の番号（`UPDATER_SCHEMA`）を含める。schema を上げた更新情報は別の名前に置き、
/// 旧い schema を読むアプリが取る更新情報を旧形式のまま残すため。
pub const UPDATER_URL: &str =
    "https://github.com/YozoraKurage/YoluPainter/releases/latest/download/updater-v1.json";
/// 更新情報の本文の schema。変えるときは `UPDATER_FILE` と `UPDATER_URL` の番号も上げ、旧い名前のファイルも残して配る。
pub const UPDATER_SCHEMA: u32 = 1;
/// Release に置く更新情報のファイル名（`UPDATER_URL` の末尾）。
pub const UPDATER_FILE: &str = "updater-v1.json";
/// 試験版の更新情報を置く固定のタグ。GitHub の prerelease の Release で、「最新の Release」（`UPDATER_URL` の行き先）にならない。
/// 公開した試験版の署名つきの `UPDATER_FILE` を、原本のまま上書きで置く（`cargo xtask beta-channel`）。
pub const BETA_CHANNEL_TAG: &str = "updater-beta";
/// 設定「試験版を使う」を入れたアプリが、stable に加えて取る場所（固定のタグの資産。認証なし・API の回数の上限なし）。
/// `UPDATER_SCHEMA` を上げたときは、`UPDATER_URL` と一緒にファイル名の番号も上げる。
pub const BETA_UPDATER_URL: &str =
    "https://github.com/YozoraKurage/YoluPainter/releases/download/updater-beta/updater-v1.json";
pub const MAX_METADATA: usize = 1024 * 1024;
pub const MAX_ASSET: u64 = 2 * 1024 * 1024 * 1024;
/// 更新情報に載せられる配布物の数の上限。対象を足しても旧版のクライアントが読めるよう、
/// 既知の対象の数ではなく固定の値にする。
pub const MAX_ASSETS: usize = 32;
/// 配布物の種類の鍵。三つ組みの対象のアーカイブ（zip・tar.gz）のほかに、Windows のインストーラーを別の鍵で載せる。
/// 知らない鍵の配布物は旧いクライアントが読み飛ばすので、種類を足しても schema は上げない。
pub const WINDOWS_ARCHIVE: &str = "x86_64-pc-windows-msvc";
pub const LINUX_ARCHIVE: &str = "x86_64-unknown-linux-gnu";
/// Windows のインストーラー（NSIS の setup.exe）。インストール済みの Windows のアプリが自分を更新するときの配布物。
pub const WINDOWS_INSTALLER: &str = "x86_64-pc-windows-msvc-setup";
/// macOS の試作の配布物（Intel と Apple Silicon の両方に入る universal の .app を入れた zip）の鍵。Rust のターゲットではなく、
/// 2 つのターゲット（`MACOS_TRIPLES`）でビルドして 1 つにまとめた物の名前。署名は ad-hoc だけで、アプリは自分では入れ替えない
/// （新しい版を知らせて、その版のリリースのページを開くだけ）。
pub const MACOS_ARCHIVE: &str = "universal-apple-darwin";
/// `MACOS_ARCHIVE` を作る 2 つの Rust のターゲット。
pub const MACOS_TRIPLES: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
/// `MACOS_ARCHIVE` の配布物名に入る呼び名（「試作」と分かる形）。
const MACOS_NAME: &str = "macos-universal-experimental";
pub const TARGETS: [&str; 4] = [
    WINDOWS_ARCHIVE,
    LINUX_ARCHIVE,
    WINDOWS_INSTALLER,
    MACOS_ARCHIVE,
];
/// アーカイブ（zip・tar.gz）の対象か。`build`・`bundle` に渡せるのはこれだけ。
pub fn is_archive_target(target: &str) -> bool {
    target == WINDOWS_ARCHIVE || target == LINUX_ARCHIVE || target == MACOS_ARCHIVE
}

/// 更新情報に、この環境の対象の配布物が無いときの `Error` の文（署名・形式は通っている。`Error::is_missing_target` が見分ける）。
const MISSING_TARGET: &str = "対象の配布物がありません";

/// 試験版の版が持つプレリリース識別子の名前（並びは SemVer の辞書順と同じ: alpha < beta < rc）。
pub const BETA_NAMES: [&str; 3] = ["alpha", "beta", "rc"];
/// 試験版の版か。プレリリース識別子が `alpha.N`・`beta.N`・`rc.N`（N は整数）の 1 つだけのもの。
/// この形にすると、版の前後が SemVer のとおりに並ぶ（`rc.2` < `rc.10` < 正式版）。`rc10` のような書き方は
/// 辞書順で `rc10` < `rc2` になり、新しい試験版が古く見えるので、試験版として認めない。
pub fn is_beta_version(version: &Version) -> bool {
    let mut parts = version.pre.as_str().split('.');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(name), Some(number), None) => {
            BETA_NAMES.contains(&name)
                && !number.is_empty()
                && number.bytes().all(|b| b.is_ascii_digit())
        }
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);
impl Error {
    /// 検証は通ったが、更新情報にこの環境の対象の配布物が無い（壊れた・改ざんされた更新情報ではない）。
    pub fn is_missing_target(&self) -> bool {
        self.0 == MISSING_TARGET
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
fn fail(message: &str) -> Error {
    Error(message.into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub target: String,
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub assets: Vec<Asset>,
}
/// 署名検証後の本文の読み取り用。配布物は対象を見てから厳密に読む。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    schema: u32,
    version: String,
    assets: Vec<serde_json::Value>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    /// 署名対象の UTF-8。検証前に解析・正規化しない。
    pub payload: String,
    pub signature: Option<String>,
}

pub fn asset_name(version: &Version, target: &str) -> Result<String, Error> {
    let (stem, extension) = match target {
        WINDOWS_ARCHIVE => (target, "zip"),
        LINUX_ARCHIVE => (target, "tar.gz"),
        WINDOWS_INSTALLER => (target, "exe"),
        MACOS_ARCHIVE => (MACOS_NAME, "zip"),
        _ => return Err(fail("未対応の配布ターゲットです")),
    };
    Ok(format!("yolupainter-{version}-{stem}.{extension}"))
}
pub fn asset_url(version: &Version, name: &str) -> String {
    format!("{RELEASE_BASE}/v{version}/{name}")
}
/// その版の Release のページ（アプリを自動で入れ替えない環境で、利用者に開いてもらう）。
pub fn release_page(version: &Version) -> String {
    let releases = RELEASE_BASE
        .strip_suffix("/download")
        .unwrap_or(RELEASE_BASE);
    format!("{releases}/tag/v{version}")
}
pub fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// 実装は HTTPS の証明書を検証し、タイムアウトを設け、読み込み中にも上限を守ること。
/// 認証情報を別ホストへ転送しない。試験にはメモリ上の実装を利用できる。
pub trait Transport {
    fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, Error>;
}

fn parse_hex_key(key: Option<&str>) -> Result<[u8; 32], Error> {
    let key = key.ok_or_else(|| fail("更新用公開鍵が組み込まれていません"))?;
    hex::decode(key)
        .map_err(|_| fail("公開鍵の形式が不正です"))?
        .try_into()
        .map_err(|_| fail("公開鍵の長さが不正です"))
}

/// ビルド時に組み込んだ公開鍵（`YOLUPAINTER_UPDATE_PUBLIC_KEY`）。未設定・空・形式が不正なら Err。
/// アプリは、これが Ok のビルドだけに更新の項目を出す。弱い鍵は `UpdateClient::with_public_key` が断る。
pub fn embedded_public_key() -> Result<[u8; 32], Error> {
    parse_hex_key(option_env!("YOLUPAINTER_UPDATE_PUBLIC_KEY"))
}

fn verifying_key(key: [u8; 32]) -> Result<VerifyingKey, Error> {
    let key = VerifyingKey::from_bytes(&key).map_err(|_| fail("公開鍵が不正です"))?;
    if key.is_weak() {
        return Err(fail("弱い公開鍵は使えません"));
    }
    Ok(key)
}

/// 公開鍵として使えるか（曲線の上の点で、弱い鍵でない）。配布のビルドが、組み込む前に確かめる。
pub fn check_public_key(key: [u8; 32]) -> Result<(), Error> {
    verifying_key(key).map(|_| ())
}

pub struct UpdateClient<T> {
    transport: T,
    key: VerifyingKey,
}
impl<T: Transport> UpdateClient<T> {
    /// 公開鍵は配布時に YOLUPAINTER_UPDATE_PUBLIC_KEY（32 バイトの hex）で組み込む。
    pub fn embedded(transport: T) -> Result<Self, Error> {
        Self::from_hex_key(transport, option_env!("YOLUPAINTER_UPDATE_PUBLIC_KEY"))
    }
    fn from_hex_key(transport: T, key: Option<&str>) -> Result<Self, Error> {
        Self::with_public_key(transport, parse_hex_key(key)?)
    }
    /// 呼び出し側が信頼した公開鍵だけを渡す。更新情報から公開鍵を取得しない。
    pub fn with_public_key(transport: T, key: [u8; 32]) -> Result<Self, Error> {
        Ok(Self {
            transport,
            key: verifying_key(key)?,
        })
    }
    pub fn check(
        &self,
        url: &str,
        current: &Version,
        target: &str,
        allow_prerelease: bool,
    ) -> Result<Option<AvailableUpdate>, Error> {
        if !url.starts_with("https://") {
            return Err(fail("HTTPS が必要です"));
        }
        let bytes = self.transport.get(url, MAX_METADATA)?;
        if bytes.len() > MAX_METADATA {
            return Err(fail("更新情報が大きすぎます"));
        }
        let envelope: Envelope =
            serde_json::from_slice(&bytes).map_err(|_| fail("更新情報の形式が不正です"))?;
        let signature = hex::decode(envelope.signature.ok_or_else(|| fail("署名がありません"))?)
            .map_err(|_| fail("署名の形式が不正です"))?;
        let signature =
            Signature::from_slice(&signature).map_err(|_| fail("署名の長さが不正です"))?;
        self.key
            .verify_strict(envelope.payload.as_bytes(), &signature)
            .map_err(|_| fail("署名が一致しません"))?;
        let manifest: RawManifest =
            serde_json::from_str(&envelope.payload).map_err(|_| fail("本文の形式が不正です"))?;
        let version = Version::parse(&manifest.version).map_err(|_| fail("版の形式が不正です"))?;
        if manifest.schema != UPDATER_SCHEMA
            || manifest.assets.is_empty()
            || manifest.assets.len() > MAX_ASSETS
        {
            return Err(fail("更新情報の版または配布物の数が不正です"));
        }
        // 署名済みでも、このクライアントが知らない対象の配布物は読み飛ばす。
        // 対象を足した版の更新情報を、すでに配った版が拒否しないため。
        // 知っている対象の配布物は、形・名前・URL・大きさ・SHA-256 まで厳密に確かめる。
        let mut seen = std::collections::HashSet::new();
        let mut assets = Vec::new();
        for raw in manifest.assets {
            let target = raw
                .get("target")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| fail("配布物の指定が不正です"))?;
            if !TARGETS.contains(&target) {
                continue;
            }
            let asset: Asset =
                serde_json::from_value(raw).map_err(|_| fail("配布物の指定が不正です"))?;
            let name = asset_name(&version, &asset.target)?;
            if !seen.insert(asset.target.clone())
                || asset.name != name
                || asset.url != asset_url(&version, &name)
                || asset.size == 0
                || asset.size > MAX_ASSET
                || asset.sha256.len() != 64
                || !asset
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(fail("配布物の指定が不正です"));
            }
            assets.push(asset);
        }
        // build metadata は版の前後に影響させない。
        if version.cmp_precedence(current).is_le() || (!allow_prerelease && !version.pre.is_empty())
        {
            return Ok(None);
        }
        let asset = assets
            .into_iter()
            .find(|a| a.target == target)
            .ok_or_else(|| fail(MISSING_TARGET))?;
        Ok(Some(AvailableUpdate { version, asset }))
    }
    /// stable の更新情報（`stable_url`）と、あれば試験版の置き場（`beta_url`）の両方を見て、今の版より新しい方を返す。
    /// どちらも署名・形式・版・対象を `check` と同じ厳しさで確かめる（同じ鍵）。stable が試験版の版を載せていても受けない。
    ///
    /// - 試験版の置き場が引けない・検証を通らない・対象が無いときは、無かったことにして stable の結果だけで答える
    ///   （まだ試験版を 1 つも出していない間も含む。試験版が使えないことで stable の更新まで止めない）。
    /// - 逆に stable が引けないときは、試験版が新しい版を見つけたときだけそれを返す。「新しい版なし」とは答えない
    ///   （引けなかった stable に、もっと新しい版があるかもしれないため）。
    /// - 同じ版が両方にあれば stable を返す。版を下げる更新は返さない（`check` が今の版以下を断る）。
    pub fn check_channels(
        &self,
        stable_url: &str,
        beta_url: Option<&str>,
        current: &Version,
        target: &str,
    ) -> Result<Option<AvailableUpdate>, Error> {
        let (stable, beta) = self.check_each(stable_url, beta_url, current, target);
        merge_channels(stable, beta)
    }
    /// `check_channels` の前半: 両方の置き場を別々に確かめる。1 つ目は stable の結果、2 つ目は試験版の置き場が見つけた新しい版
    /// （`beta_url` が無い・引けない・検証を通らない・対象が無い・今の版より新しくない、はどれも `None`）。
    /// 束ねるのは `merge_channels`。結果を受け取る側が、どちらを数えるかをあとで決めたいとき（確かめの途中で設定が変わる）に使う。
    pub fn check_each(
        &self,
        stable_url: &str,
        beta_url: Option<&str>,
        current: &Version,
        target: &str,
    ) -> (
        Result<Option<AvailableUpdate>, Error>,
        Option<AvailableUpdate>,
    ) {
        let stable = self.check(stable_url, current, target, false);
        let beta = beta_url.and_then(|url| self.check(url, current, target, true).ok().flatten());
        (stable, beta)
    }
    pub fn download(&self, approved: ApprovedDownload) -> Result<VerifiedDownload, Error> {
        let update = approved.0;
        let bytes = self
            .transport
            .get(&update.asset.url, update.asset.size as usize)?;
        if bytes.len() as u64 != update.asset.size || sha256(&bytes) != update.asset.sha256 {
            return Err(fail("配布物の大きさまたは SHA-256 が一致しません"));
        }
        Ok(VerifiedDownload { update, bytes })
    }
}

/// stable の結果と、試験版の置き場が見つけた新しい版から、勧める版を決める（`check_channels` の規則。`E` は呼び出し側の失敗の型）。
/// stable が引けないときは、試験版が見つかったときだけそれを返し、見つからなければ stable の失敗のまま（「新しい版なし」とは答えない）。
pub fn merge_channels<E>(
    stable: Result<Option<AvailableUpdate>, E>,
    beta: Option<AvailableUpdate>,
) -> Result<Option<AvailableUpdate>, E> {
    match (stable, beta) {
        (Ok(stable), beta) => Ok(newer(stable, beta)),
        (Err(_), Some(beta)) => Ok(Some(beta)),
        (Err(error), None) => Err(error),
    }
}

/// 2 つの結果のうち版が新しい方（同じ版なら先の方）。
fn newer(
    first: Option<AvailableUpdate>,
    second: Option<AvailableUpdate>,
) -> Option<AvailableUpdate> {
    match (first, second) {
        (Some(a), Some(b)) => Some(if b.version.cmp_precedence(&a.version).is_gt() {
            b
        } else {
            a
        }),
        (a, b) => a.or(b),
    }
}

#[derive(Debug, Clone)]
pub struct AvailableUpdate {
    version: Version,
    asset: Asset,
}
impl AvailableUpdate {
    pub fn version(&self) -> &Version {
        &self.version
    }
    pub fn asset(&self) -> &Asset {
        &self.asset
    }
    /// UI でこの版のダウンロードを利用者が承認した後だけ呼ぶ。
    pub fn approve_download(self) -> ApprovedDownload {
        ApprovedDownload(self)
    }
}
pub struct ApprovedDownload(AvailableUpdate);
pub struct VerifiedDownload {
    update: AvailableUpdate,
    bytes: Vec<u8>,
}
impl VerifiedDownload {
    pub fn version(&self) -> &Version {
        self.update.version()
    }
    pub fn asset(&self) -> &Asset {
        self.update.asset()
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::cell::RefCell;
    const URL: &str = "https://example.invalid/updater.json";
    struct Fake {
        metadata: Vec<u8>,
        file: Vec<u8>,
        calls: RefCell<Vec<String>>,
        limits: RefCell<Vec<usize>>,
    }
    impl Transport for Fake {
        fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, Error> {
            self.calls.borrow_mut().push(url.into());
            self.limits.borrow_mut().push(max_bytes);
            Ok(if url == URL {
                self.metadata.clone()
            } else {
                self.file.clone()
            })
        }
    }
    fn manifest() -> Manifest {
        let version = Version::parse("1.2.0").unwrap();
        let name = asset_name(&version, TARGETS[0]).unwrap();
        Manifest {
            schema: 1,
            version: version.to_string(),
            assets: vec![Asset {
                target: TARGETS[0].into(),
                url: asset_url(&version, &name),
                name,
                sha256: sha256(b"archive"),
                size: 7,
            }],
        }
    }
    fn client(m: Manifest, alter: impl FnOnce(&mut Envelope)) -> UpdateClient<Fake> {
        client_payload(serde_json::to_string(&m).unwrap(), alter)
    }
    fn client_payload(payload: String, alter: impl FnOnce(&mut Envelope)) -> UpdateClient<Fake> {
        // 試験だけの固定鍵。本番鍵は生成しない。
        let key = SigningKey::from_bytes(&[42; 32]);
        let mut envelope = Envelope {
            signature: Some(hex::encode(key.sign(payload.as_bytes()).to_bytes())),
            payload,
        };
        alter(&mut envelope);
        UpdateClient::with_public_key(
            Fake {
                metadata: serde_json::to_vec(&envelope).unwrap(),
                file: b"archive".to_vec(),
                calls: RefCell::new(vec![]),
                limits: RefCell::new(vec![]),
            },
            key.verifying_key().to_bytes(),
        )
        .unwrap()
    }
    fn check(c: &UpdateClient<Fake>) -> Result<Option<AvailableUpdate>, Error> {
        c.check(URL, &Version::parse("1.0.0").unwrap(), TARGETS[0], false)
    }
    #[test]
    fn approval_precedes_download_and_hash_verification() {
        let c = client(manifest(), |_| {});
        let update = check(&c).unwrap().unwrap();
        assert_eq!(c.transport.calls.borrow().len(), 1);
        let verified = c.download(update.approve_download()).unwrap();
        assert_eq!(verified.bytes(), b"archive");
        assert_eq!(verified.version().to_string(), "1.2.0");
        assert_eq!(c.transport.calls.borrow().len(), 2);
    }
    #[test]
    fn installer_is_its_own_asset_kind_and_the_app_picks_it_by_key() {
        let version = Version::parse("1.2.0").unwrap();
        assert_eq!(
            asset_name(&version, WINDOWS_INSTALLER).unwrap(),
            "yolupainter-1.2.0-x86_64-pc-windows-msvc-setup.exe"
        );
        assert!(is_archive_target(WINDOWS_ARCHIVE) && is_archive_target(LINUX_ARCHIVE));
        assert!(!is_archive_target(WINDOWS_INSTALLER));
        let mut m = manifest();
        let name = asset_name(&version, WINDOWS_INSTALLER).unwrap();
        m.assets.push(Asset {
            target: WINDOWS_INSTALLER.into(),
            url: asset_url(&version, &name),
            name,
            sha256: sha256(b"installer"),
            size: 9,
        });
        let c = client(m, |_| {});
        let current = Version::parse("1.0.0").unwrap();
        let zip = c
            .check(URL, &current, WINDOWS_ARCHIVE, false)
            .unwrap()
            .unwrap();
        assert!(zip.asset().name.ends_with(".zip"));
        let setup = c
            .check(URL, &current, WINDOWS_INSTALLER, false)
            .unwrap()
            .unwrap();
        assert!(setup.asset().name.ends_with("-setup.exe"));
        // 載っていない種類を求められたら、別の種類で代用せず断る。
        assert!(c.check(URL, &current, LINUX_ARCHIVE, false).is_err());
    }
    #[test]
    fn installer_asset_fields_are_checked_like_any_other() {
        let version = Version::parse("1.2.0").unwrap();
        let name = asset_name(&version, WINDOWS_INSTALLER).unwrap();
        let good = Asset {
            target: WINDOWS_INSTALLER.into(),
            url: asset_url(&version, &name),
            name,
            sha256: sha256(b"installer"),
            size: 9,
        };
        for n in 0..3 {
            let mut m = manifest();
            let mut a = good.clone();
            match n {
                0 => a.name = "yolupainter-1.2.0-x86_64-pc-windows-msvc.zip".into(),
                1 => a.url = "https://example.invalid/setup.exe".into(),
                _ => a.size = 0,
            }
            m.assets.push(a);
            assert!(check(&client(m, |_| {})).is_err(), "{n}");
        }
    }
    #[test]
    fn macos_is_a_zip_named_experimental_and_found_by_its_own_key() {
        let version = Version::parse("1.2.0").unwrap();
        // 鍵は Rust のターゲットではなく、2 つのターゲットをまとめた物の名前。配布物の名前は「試作」と分かる形
        assert_eq!(
            asset_name(&version, MACOS_ARCHIVE).unwrap(),
            "yolupainter-1.2.0-macos-universal-experimental.zip"
        );
        assert!(is_archive_target(MACOS_ARCHIVE) && TARGETS.contains(&MACOS_ARCHIVE));
        assert!(MACOS_TRIPLES.iter().all(|t| t.ends_with("-apple-darwin")));
        assert!(MACOS_TRIPLES.iter().all(|t| !is_archive_target(t)));
        // mac の配布物を載せた更新情報を、Windows・Linux の確かめは今までどおり読む。mac は自分の鍵で見つける
        let mut m = manifest();
        let name = asset_name(&version, MACOS_ARCHIVE).unwrap();
        m.assets.push(Asset {
            target: MACOS_ARCHIVE.into(),
            url: asset_url(&version, &name),
            name: name.clone(),
            sha256: sha256(b"mac"),
            size: 3,
        });
        let c = client(m, |_| {});
        let current = Version::parse("1.0.0").unwrap();
        assert_eq!(
            check(&c).unwrap().unwrap().asset().name,
            asset_name(&version, WINDOWS_ARCHIVE).unwrap()
        );
        let update = c
            .check(URL, &current, MACOS_ARCHIVE, false)
            .unwrap()
            .unwrap();
        assert_eq!(update.asset().name, name);
        // mac の配布物を載せない版は、署名は正しいまま「対象の配布物がありません」
        let error = client(manifest(), |_| {})
            .check(URL, &current, MACOS_ARCHIVE, false)
            .unwrap_err();
        assert!(error.is_missing_target());
        // 載せた mac の配布物の名前が違えば、ほかの対象と同じく断る
        let mut m = manifest();
        m.assets.push(Asset {
            target: MACOS_ARCHIVE.into(),
            url: asset_url(&version, "yolupainter-1.2.0-universal-apple-darwin.zip"),
            name: "yolupainter-1.2.0-universal-apple-darwin.zip".into(),
            sha256: sha256(b"mac"),
            size: 3,
        });
        assert!(check(&client(m, |_| {})).is_err());
    }
    #[test]
    fn update_locations_follow_the_release_base_and_the_schema() {
        let releases = RELEASE_BASE.strip_suffix("/download").unwrap();
        assert_eq!(
            UPDATER_URL,
            format!("{releases}/latest/download/{UPDATER_FILE}")
        );
        assert_eq!(UPDATER_FILE, format!("updater-v{UPDATER_SCHEMA}.json"));
        assert_eq!(
            release_page(&Version::parse("1.2.0-rc.1").unwrap()),
            format!("{releases}/tag/v1.2.0-rc.1")
        );
    }
    #[test]
    fn unsigned_rejected() {
        assert!(check(&client(manifest(), |e| e.signature = None)).is_err());
    }
    #[test]
    fn tampered_payload_rejected() {
        assert!(check(&client(manifest(), |e| e.payload.push(' '))).is_err());
    }
    #[test]
    fn malformed_signature_rejected() {
        assert!(check(&client(manifest(), |e| e.signature = Some("00".into()))).is_err());
    }
    #[test]
    fn wrong_key_rejected() {
        let mut c = client(manifest(), |_| {});
        c.key = SigningKey::from_bytes(&[43; 32]).verifying_key();
        assert!(check(&c).is_err());
    }
    #[test]
    fn same_and_older_versions_not_offered() {
        let c = client(manifest(), |_| {});
        for v in ["1.2.0", "2.0.0", "1.2.0+build"] {
            assert!(c
                .check(URL, &Version::parse(v).unwrap(), TARGETS[0], false)
                .unwrap()
                .is_none());
        }
    }
    #[test]
    fn prerelease_requires_opt_in() {
        let mut m = manifest();
        m.version = "1.3.0-rc.1".into();
        let v = Version::parse(&m.version).unwrap();
        m.assets[0].name = asset_name(&v, TARGETS[0]).unwrap();
        m.assets[0].url = asset_url(&v, &m.assets[0].name);
        let c = client(m, |_| {});
        assert!(check(&c).unwrap().is_none());
        assert!(c
            .check(URL, &Version::parse("1.0.0").unwrap(), TARGETS[0], true)
            .unwrap()
            .is_some());
    }
    #[test]
    fn invalid_signed_asset_fields_rejected() {
        for n in 0..7 {
            let mut m = manifest();
            match n {
                0 => m.assets[0].url = "https://example.invalid/malicious".into(),
                1 => m.assets[0].name = "../bad.zip".into(),
                2 => m.assets[0].size = 0,
                3 => m.assets[0].size = MAX_ASSET + 1,
                4 => m.assets[0].sha256 = "g".repeat(64),
                5 => m.assets.push(m.assets[0].clone()),
                _ => m.schema = 2,
            }
            assert!(check(&client(m, |_| {})).is_err());
        }
    }
    #[test]
    fn missing_target_rejected() {
        let error = client(manifest(), |_| {})
            .check(URL, &Version::new(1, 0, 0), TARGETS[1], false)
            .unwrap_err();
        // 検証は通ったが対象が無いだけなので、壊れた更新情報とは見分けられる
        assert!(error.is_missing_target(), "{error}");
        // 署名が合わない・形式が不正・対象がある場合は、「対象が無い」ではない
        let tampered = client(manifest(), |e| e.payload.push(' '))
            .check(URL, &Version::new(1, 0, 0), TARGETS[0], false)
            .unwrap_err();
        assert!(!tampered.is_missing_target(), "{tampered}");
        let malformed = client_payload("{".into(), |_| {})
            .check(URL, &Version::new(1, 0, 0), TARGETS[0], false)
            .unwrap_err();
        assert!(!malformed.is_missing_target(), "{malformed}");
    }
    /// 署名つきの正しい更新情報の末尾に空白を足して大きさだけを変える。
    /// serde_json は末尾の空白を許すので、落とすのは大きさの検査だけになる。
    fn padded_to(c: &mut UpdateClient<Fake>, length: usize) {
        assert!(c.transport.metadata.len() <= length);
        c.transport.metadata.resize(length, b' ');
    }
    #[test]
    fn oversized_metadata_rejected_only_by_size_check() {
        let mut c = client(manifest(), |_| {});
        padded_to(&mut c, MAX_METADATA);
        assert!(check(&c).unwrap().is_some());
        padded_to(&mut c, MAX_METADATA + 1);
        assert_eq!(check(&c).err().unwrap(), fail("更新情報が大きすぎます"));
    }
    #[test]
    fn transport_is_given_the_size_budget() {
        let c = client(manifest(), |_| {});
        let update = check(&c).unwrap().unwrap();
        c.download(update.approve_download()).unwrap();
        assert_eq!(*c.transport.limits.borrow(), vec![MAX_METADATA, 7]);
    }
    #[test]
    fn download_longer_than_signed_size_rejected() {
        let mut c = client(manifest(), |_| {});
        c.transport.file = b"archive!".to_vec();
        assert!(c
            .download(check(&c).unwrap().unwrap().approve_download())
            .is_err());
    }
    fn payload_with_assets(extra: Vec<serde_json::Value>) -> String {
        let mut value = serde_json::to_value(manifest()).unwrap();
        value["assets"].as_array_mut().unwrap().extend(extra);
        value.to_string()
    }
    #[test]
    fn signed_assets_for_unknown_targets_are_skipped() {
        // 後から足した対象は、形も知らない。旧版のクライアントは読み飛ばして自分の対象を探す。
        let c = client_payload(
            payload_with_assets(vec![
                serde_json::json!({
                    "target": "aarch64-apple-darwin", "name": "x.dmg", "notarized": true,
                    "url": "https://example.invalid/x.dmg", "size": "unknown",
                }),
                serde_json::json!({"target": "aarch64-apple-darwin"}),
            ]),
            |_| {},
        );
        let update = check(&c).unwrap().unwrap();
        assert_eq!(update.asset().target, TARGETS[0]);
        // 知らない対象しか持たない対象は従来どおり「対象の配布物がありません」。
        assert!(c
            .check(URL, &Version::new(1, 0, 0), TARGETS[1], false)
            .is_err());
    }
    #[test]
    fn unknown_target_assets_do_not_hide_bad_known_assets() {
        let mut bad = serde_json::to_value(&manifest().assets[0]).unwrap();
        bad["sha256"] = "g".repeat(64).into();
        let c = client_payload(payload_with_assets(vec![bad]), |_| {});
        assert!(check(&c).is_err());
        let mut extra_field = serde_json::to_value(&manifest().assets[0]).unwrap();
        extra_field["target"] = TARGETS[1].into();
        extra_field["extra"] = true.into();
        assert!(check(&client_payload(
            payload_with_assets(vec![extra_field]),
            |_| {}
        ))
        .is_err());
    }
    #[test]
    fn asset_without_target_or_too_many_assets_rejected() {
        let without_target = serde_json::json!({"name": "x"});
        assert!(check(&client_payload(
            payload_with_assets(vec![without_target]),
            |_| {}
        ))
        .is_err());
        let many = (0..MAX_ASSETS)
            .map(|n| serde_json::json!({"target": format!("unknown-{n}")}))
            .collect();
        assert!(check(&client_payload(payload_with_assets(many), |_| {})).is_err());
        // 上限ちょうどは通る。
        let fits = (0..MAX_ASSETS - 1)
            .map(|n| serde_json::json!({"target": format!("unknown-{n}")}))
            .collect();
        assert!(check(&client_payload(payload_with_assets(fits), |_| {}))
            .unwrap()
            .is_some());
    }
    #[test]
    fn unknown_manifest_fields_or_schema_rejected() {
        let mut value = serde_json::to_value(manifest()).unwrap();
        value["added_later"] = true.into();
        assert!(check(&client_payload(value.to_string(), |_| {})).is_err());
        let mut value = serde_json::to_value(manifest()).unwrap();
        value["schema"] = 2.into();
        assert!(check(&client_payload(value.to_string(), |_| {})).is_err());
    }
    #[test]
    fn embedded_key_must_be_configured_and_valid() {
        let fake = || Fake {
            metadata: vec![],
            file: vec![],
            calls: RefCell::new(vec![]),
            limits: RefCell::new(vec![]),
        };
        let unset = UpdateClient::from_hex_key(fake(), None).err().unwrap();
        assert_eq!(unset, fail("更新用公開鍵が組み込まれていません"));
        for bad in ["", "zz", &"ab".repeat(31), &"ab".repeat(33)] {
            assert!(UpdateClient::from_hex_key(fake(), Some(bad)).is_err());
        }
        let good = hex::encode(SigningKey::from_bytes(&[42; 32]).verifying_key().to_bytes());
        assert!(UpdateClient::from_hex_key(fake(), Some(&good)).is_ok());
        // この試験の組みでは公開鍵を組み込んでいないので、未設定はエラーになる。
        if option_env!("YOLUPAINTER_UPDATE_PUBLIC_KEY").is_none() {
            assert!(UpdateClient::embedded(fake()).is_err());
            assert_eq!(
                embedded_public_key().unwrap_err(),
                fail("更新用公開鍵が組み込まれていません")
            );
        }
        assert_eq!(parse_hex_key(Some(&good)).unwrap().len(), 32);
        assert!(parse_hex_key(Some("")).is_err());
    }
    #[test]
    fn weak_or_invalid_public_keys_rejected() {
        let fake = || Fake {
            metadata: vec![],
            file: vec![],
            calls: RefCell::new(vec![]),
            limits: RefCell::new(vec![]),
        };
        // 単位元（y = 1）は、どの署名も通してしまう弱い鍵。
        let mut identity = [0u8; 32];
        identity[0] = 1;
        let weak = UpdateClient::with_public_key(fake(), identity)
            .err()
            .unwrap();
        assert_eq!(weak, fail("弱い公開鍵は使えません"));
        // y = 2 は曲線の上の点にならない。
        let mut off_curve = [0u8; 32];
        off_curve[0] = 2;
        let invalid = UpdateClient::with_public_key(fake(), off_curve)
            .err()
            .unwrap();
        assert_eq!(invalid, fail("公開鍵が不正です"));
    }
    #[test]
    fn corrupt_or_truncated_download_rejected() {
        for data in [b"damaged".to_vec(), vec![]] {
            let mut c = client(manifest(), |_| {});
            c.transport.file = data;
            assert!(c
                .download(check(&c).unwrap().unwrap().approve_download())
                .is_err());
        }
    }
    // ───────── 試験版の置き場と、stable との新しい方の選び ─────────

    const STABLE: &str = "https://example.invalid/stable/updater-v1.json";
    const BETA: &str = "https://example.invalid/beta/updater-v1.json";

    /// 置き場ごとに答えを決められる偽の口（`None` は通信できない）。取った URL を記録する。
    struct Routes {
        files: Vec<(&'static str, Option<Vec<u8>>)>,
        calls: RefCell<Vec<String>>,
    }
    impl Transport for Routes {
        fn get(&self, url: &str, _max_bytes: usize) -> Result<Vec<u8>, Error> {
            self.calls.borrow_mut().push(url.into());
            match self.files.iter().find(|(known, _)| *known == url) {
                Some((_, Some(bytes))) => Ok(bytes.clone()),
                _ => Err(fail("通信できません")),
            }
        }
    }
    /// その版の、使い捨ての鍵（`seed`）で署名した更新情報（最初の対象の配布物だけを載せる）。
    fn signed(version: &str, seed: u8) -> Vec<u8> {
        signed_for(version, seed, TARGETS[0])
    }
    /// 載せる配布物の対象を選べる `signed`。
    fn signed_for(version: &str, seed: u8, target: &str) -> Vec<u8> {
        let v = Version::parse(version).unwrap();
        let name = asset_name(&v, target).unwrap();
        let payload = serde_json::to_string(&Manifest {
            schema: 1,
            version: version.into(),
            assets: vec![Asset {
                target: target.into(),
                url: asset_url(&v, &name),
                name,
                sha256: sha256(b"archive"),
                size: 7,
            }],
        })
        .unwrap();
        let key = SigningKey::from_bytes(&[seed; 32]);
        serde_json::to_vec(&Envelope {
            signature: Some(hex::encode(key.sign(payload.as_bytes()).to_bytes())),
            payload,
        })
        .unwrap()
    }
    fn channels(stable: Option<Vec<u8>>, beta: Option<Vec<u8>>) -> UpdateClient<Routes> {
        UpdateClient::with_public_key(
            Routes {
                files: vec![(STABLE, stable), (BETA, beta)],
                calls: RefCell::new(vec![]),
            },
            SigningKey::from_bytes(&[42; 32]).verifying_key().to_bytes(),
        )
        .unwrap()
    }
    /// 今の版 `current` で stable と試験版の両方を見た結果の版（無ければ None）。
    fn offered(c: &UpdateClient<Routes>, current: &str, beta: bool) -> Option<String> {
        c.check_channels(
            STABLE,
            beta.then_some(BETA),
            &Version::parse(current).unwrap(),
            TARGETS[0],
        )
        .unwrap()
        .map(|u| u.version().to_string())
    }
    #[test]
    fn beta_locations_are_a_fixed_tag_asset_that_is_not_the_latest_release() {
        let releases = RELEASE_BASE.strip_suffix("/download").unwrap();
        assert_eq!(
            BETA_UPDATER_URL,
            format!("{releases}/download/{BETA_CHANNEL_TAG}/{UPDATER_FILE}")
        );
        // stable の道は「最新の Release」、試験版の道は固定のタグ。同じ道にならない。
        assert!(UPDATER_URL.contains("/releases/latest/download/"));
        assert!(!BETA_UPDATER_URL.contains("/latest/"));
        // タグの名前は、配布物のタグ `v<版>` と衝突しない。
        assert!(!BETA_CHANNEL_TAG.starts_with('v'));
    }
    #[test]
    fn beta_versions_are_alpha_beta_or_rc_with_one_number() {
        let beta = |v: &str| is_beta_version(&Version::parse(v).unwrap());
        for yes in [
            "1.2.0-alpha.1",
            "1.2.0-beta.3",
            "1.2.0-rc.1",
            "1.2.0-rc.10",
            "0.4.0-rc.0+build5",
        ] {
            assert!(beta(yes), "{yes}");
        }
        // 正式版・数の無い識別子・つなげた書き方（rc10 は辞書順で rc2 より前になる）・知らない名前・段の多いもの
        for no in [
            "1.2.0",
            "1.2.0+build",
            "1.2.0-rc",
            "1.2.0-rc1",
            "1.2.0-rc.1.2",
            "1.2.0-preview.1",
            "1.2.0-RC.1",
            "1.2.0-rc.x",
            "1.2.0-rc.1a",
            "1.2.0-0",
            "1.2.0-1.rc",
        ] {
            assert!(!beta(no), "{no}");
        }
    }
    #[test]
    fn beta_versions_sort_between_the_releases_they_lead_to() {
        let order = [
            "1.1.0",
            "1.2.0-alpha.1",
            "1.2.0-beta.1",
            "1.2.0-beta.2",
            "1.2.0-rc.1",
            "1.2.0-rc.2",
            "1.2.0-rc.10",
            "1.2.0",
            "1.2.1-rc.1",
            "1.2.1",
        ];
        for pair in order.windows(2) {
            let (a, b) = (
                Version::parse(pair[0]).unwrap(),
                Version::parse(pair[1]).unwrap(),
            );
            assert!(a.cmp_precedence(&b).is_lt(), "{} < {}", pair[0], pair[1]);
        }
    }
    #[test]
    fn without_the_beta_setting_only_stable_is_asked_and_a_beta_in_it_is_ignored() {
        let c = channels(Some(signed("1.2.0", 42)), Some(signed("1.3.0-rc.1", 42)));
        assert_eq!(offered(&c, "1.0.0", false).as_deref(), Some("1.2.0"));
        assert_eq!(*c.transport.calls.borrow(), [STABLE]);
        // stable の場所に試験版の版が載っていても受けない（今の版が試験版でも）。
        let c = channels(Some(signed("1.3.0-rc.2", 42)), None);
        assert_eq!(offered(&c, "1.0.0", false), None);
        assert_eq!(offered(&c, "1.3.0-rc.1", false), None);
    }
    #[test]
    fn with_the_beta_setting_the_newer_of_stable_and_the_beta_is_offered() {
        let both =
            |stable: &str, beta: &str| channels(Some(signed(stable, 42)), Some(signed(beta, 42)));
        // 試験版が新しい
        let c = both("1.2.0", "1.3.0-rc.1");
        assert_eq!(offered(&c, "1.0.0", true).as_deref(), Some("1.3.0-rc.1"));
        assert_eq!(*c.transport.calls.borrow(), [STABLE, BETA]);
        // 同じ版の正式版が出たあとは、試験版の置き場に前の試験版が残っていても stable が新しい
        assert_eq!(
            offered(&both("1.3.0", "1.3.0-rc.2"), "1.0.0", true).as_deref(),
            Some("1.3.0")
        );
        assert_eq!(
            offered(&both("1.3.0", "1.3.0-rc.2"), "1.3.0-rc.1", true).as_deref(),
            Some("1.3.0")
        );
        // 試験版の次の試験版
        assert_eq!(
            offered(&both("1.2.0", "1.3.0-rc.2"), "1.3.0-rc.1", true).as_deref(),
            Some("1.3.0-rc.2")
        );
        // 同じ版は両方にあっても、提案は 1 つ（stable を先に）
        let same = both("1.3.0", "1.3.0");
        assert_eq!(offered(&same, "1.0.0", true).as_deref(), Some("1.3.0"));
        // 新しい版が無ければ何も返さない
        assert_eq!(
            offered(&both("1.2.0", "1.3.0-rc.1"), "1.3.0-rc.1", true),
            None
        );
        assert_eq!(offered(&both("1.2.0", "1.3.0-rc.1"), "1.3.0", true), None);
    }
    #[test]
    fn the_beta_setting_never_offers_an_older_version_and_stable_returns_when_it_is_newer() {
        let c = channels(Some(signed("1.2.0", 42)), Some(signed("1.3.0-rc.1", 42)));
        // 試験版（1.3.0-rc.1）を入れている人が設定を切る: stable（1.2.0）は古いので提案しない
        assert_eq!(offered(&c, "1.3.0-rc.1", false), None);
        // 次の stable が今の版より新しくなったとき、切っていても stable へ戻れる（入でも同じ）
        let next = channels(Some(signed("1.3.0", 42)), Some(signed("1.3.0-rc.1", 42)));
        assert_eq!(
            offered(&next, "1.3.0-rc.1", false).as_deref(),
            Some("1.3.0")
        );
        assert_eq!(offered(&next, "1.3.0-rc.1", true).as_deref(), Some("1.3.0"));
        // 古い試験版を置き場に残していても、新しい版の人を下げない
        let old = channels(Some(signed("1.2.0", 42)), Some(signed("1.2.0-rc.1", 42)));
        assert_eq!(offered(&old, "1.2.0", true), None);
        assert_eq!(offered(&old, "1.3.0-beta.1", true), None);
    }
    #[test]
    fn an_unusable_beta_place_never_hides_stable() {
        let stable = || Some(signed("1.2.0", 42));
        // 引けない（まだ 1 つも出していない場合を含む）・別の鍵の署名・形式が壊れている・今の対象の配布物が無い
        let other_target = signed_for("1.4.0-rc.1", 42, TARGETS[1]);
        for beta in [
            None,
            Some(signed("1.4.0-rc.1", 43)),
            Some(b"not json".to_vec()),
            Some(other_target),
        ] {
            let c = channels(stable(), beta);
            assert_eq!(offered(&c, "1.0.0", true).as_deref(), Some("1.2.0"));
            assert_eq!(offered(&c, "1.2.0", true), None);
        }
        // 署名を壊した試験版は、新しくても提案しない
        let mut tampered = serde_json::from_slice::<Envelope>(&signed("1.4.0-rc.1", 42)).unwrap();
        tampered.payload.push(' ');
        let c = channels(stable(), Some(serde_json::to_vec(&tampered).unwrap()));
        assert_eq!(offered(&c, "1.0.0", true).as_deref(), Some("1.2.0"));
    }
    #[test]
    fn a_missing_stable_still_lets_a_newer_beta_through_but_is_never_called_up_to_date() {
        let beta = Some(signed("1.3.0-rc.1", 42));
        // 試験版が新しければ、それを返す
        assert_eq!(
            offered(&channels(None, beta.clone()), "1.0.0", true).as_deref(),
            Some("1.3.0-rc.1")
        );
        // 試験版も新しくなければ、「新しい版なし」とは答えず失敗にする（引けなかった stable に新しい版があるかもしれない）
        let c = channels(None, beta);
        assert!(c
            .check_channels(
                STABLE,
                Some(BETA),
                &Version::parse("1.3.0-rc.1").unwrap(),
                TARGETS[0]
            )
            .is_err());
        // 両方引けないときも失敗
        let c = channels(None, None);
        assert!(c
            .check_channels(
                STABLE,
                Some(BETA),
                &Version::parse("1.0.0").unwrap(),
                TARGETS[0]
            )
            .is_err());
        // 設定が切なら、stable だけを見て、同じ失敗
        assert!(c
            .check_channels(STABLE, None, &Version::parse("1.0.0").unwrap(), TARGETS[0])
            .is_err());
        assert_eq!(*c.transport.calls.borrow(), [STABLE, BETA, STABLE]);
    }
    #[test]
    fn each_place_is_reported_apart_so_the_caller_can_drop_the_beta_later() {
        let version = |u: Option<AvailableUpdate>| u.map(|u| u.version().to_string());
        let current = Version::parse("1.0.0").unwrap();
        let c = channels(Some(signed("1.2.0", 42)), Some(signed("1.3.0-rc.1", 42)));
        let (stable, beta) = c.check_each(STABLE, Some(BETA), &current, TARGETS[0]);
        assert_eq!(version(stable.unwrap()).as_deref(), Some("1.2.0"));
        assert_eq!(version(beta.clone()).as_deref(), Some("1.3.0-rc.1"));
        // 試験版を数えるなら新しい試験版、数えないなら stable
        let stable = || c.check(STABLE, &current, TARGETS[0], false);
        assert_eq!(
            version(merge_channels(stable(), beta.clone()).unwrap()).as_deref(),
            Some("1.3.0-rc.1")
        );
        assert_eq!(
            version(merge_channels(stable(), None).unwrap()).as_deref(),
            Some("1.2.0")
        );
        // stable が引けないとき: 試験版があればそれ、無ければ stable の失敗のまま（型は呼び出し側のもの）
        let failed: Result<Option<AvailableUpdate>, &str> = Err("stable");
        assert_eq!(
            version(merge_channels(failed.clone(), beta).unwrap()).as_deref(),
            Some("1.3.0-rc.1")
        );
        assert_eq!(merge_channels(failed, None).unwrap_err(), "stable");
        // 引けない試験版・置き場を見ない呼び出しは、どちらも None
        let c = channels(Some(signed("1.2.0", 42)), None);
        let (_, beta) = c.check_each(STABLE, Some(BETA), &current, TARGETS[0]);
        assert!(beta.is_none());
        let (_, beta) = c.check_each(STABLE, None, &current, TARGETS[0]);
        assert!(beta.is_none());
        assert_eq!(*c.transport.calls.borrow(), [STABLE, BETA, STABLE]);
    }
    #[test]
    fn channels_only_talk_https() {
        let c = channels(Some(signed("1.2.0", 42)), Some(signed("1.3.0-rc.1", 42)));
        let current = Version::parse("1.0.0").unwrap();
        // 試験版の置き場が https でなければ、stable の結果で答え、その置き場へは取りに行かない
        let insecure = c.check_channels(
            STABLE,
            Some("http://example.invalid/beta"),
            &current,
            TARGETS[0],
        );
        assert_eq!(insecure.unwrap().unwrap().version().to_string(), "1.2.0");
        assert_eq!(*c.transport.calls.borrow(), [STABLE]);
    }
    #[test]
    fn insecure_metadata_transport_rejected() {
        let c = client(manifest(), |_| {});
        assert!(c
            .check(
                "http://example.invalid",
                &Version::new(1, 0, 0),
                TARGETS[0],
                false
            )
            .is_err());
        assert!(c.transport.calls.borrow().is_empty());
    }
}
