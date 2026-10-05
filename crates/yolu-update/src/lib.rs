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
pub const TARGETS: [&str; 3] = [WINDOWS_ARCHIVE, LINUX_ARCHIVE, WINDOWS_INSTALLER];
/// アーカイブ（zip・tar.gz）の対象か。`build`・`bundle` に渡せるのはこれだけ。
pub fn is_archive_target(target: &str) -> bool {
    target == WINDOWS_ARCHIVE || target == LINUX_ARCHIVE
}

/// 更新情報に、この環境の対象の配布物が無いときの `Error` の文（署名・形式は通っている。`Error::is_missing_target` が見分ける）。
const MISSING_TARGET: &str = "対象の配布物がありません";

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
    let extension = match target {
        WINDOWS_ARCHIVE => "zip",
        LINUX_ARCHIVE => "tar.gz",
        WINDOWS_INSTALLER => "exe",
        _ => return Err(fail("未対応の配布ターゲットです")),
    };
    Ok(format!("yolupainter-{version}-{target}.{extension}"))
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
