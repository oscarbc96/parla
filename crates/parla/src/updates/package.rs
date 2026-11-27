//! The update package of a release and its signature check (pure, no UI).

use std::io::Read as _;
use std::path::Path;

use super::ReleaseAsset;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: url::Url,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdatePackage {
    pub version: semver::Version,
    pub tag: String,
    pub dmg: Asset,
    pub signature: Asset,
}

pub const DOWNLOAD_PREFIX: &str = "https://github.com/oscarbc96/parla/releases/download/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyError {
    BadKey,
    BadSignature,
    WrongTrustedComment,
    Invalid,
}

pub fn package_for(
    version: &semver::Version,
    tag: &str,
    assets: &[ReleaseAsset],
) -> Option<UpdatePackage> {
    let dmg_name = format!("Parla-{version}-macos-universal.dmg");
    let sig_name = format!("{dmg_name}.minisig");
    let prefix = format!("{DOWNLOAD_PREFIX}{tag}/");
    let pick = |name: &str| -> Option<Asset> {
        let a = assets.iter().find(|a| a.name == name)?;
        if !a.browser_download_url.starts_with(&prefix) {
            return None;
        }
        let url = url::Url::parse(&a.browser_download_url).ok()?;
        Some(Asset {
            name: a.name.clone(),
            url,
            size: a.size,
        })
    };
    Some(UpdatePackage {
        version: version.clone(),
        tag: tag.to_string(),
        dmg: pick(&dmg_name)?,
        signature: pick(&sig_name)?,
    })
}

/// The release signing public key (base64 line of `minisign.pub`), baked in
/// at build time from `PARLA_UPDATE_PUBLIC_KEY`; `None` in builds without one.
pub fn update_public_key() -> Option<&'static str> {
    option_env!("PARLA_UPDATE_PUBLIC_KEY")
        .map(str::trim)
        .filter(|k| !k.is_empty())
}

/// The trusted comment the release signs into each signature.
pub fn trusted_comment(version: &semver::Version, dmg_name: &str) -> String {
    format!("parla {version} {dmg_name}")
}

fn check(
    pkg: &UpdatePackage,
    minisig: &str,
    public_key: &str,
    feed: impl FnOnce(&mut minisign_verify::StreamVerifier<'_>) -> std::io::Result<()>,
) -> Result<(), VerifyError> {
    let key = minisign_verify::PublicKey::from_base64(public_key.trim())
        .map_err(|_| VerifyError::BadKey)?;
    let sig = minisign_verify::Signature::decode(minisig).map_err(|_| VerifyError::Invalid)?;
    let mut stream = key.verify_stream(&sig).map_err(|e| match e {
        minisign_verify::Error::UnexpectedKeyId => VerifyError::BadSignature,
        _ => VerifyError::Invalid,
    })?;
    feed(&mut stream).map_err(|_| VerifyError::Invalid)?;
    stream.finalize().map_err(|_| VerifyError::BadSignature)?;
    if sig.trusted_comment() != trusted_comment(&pkg.version, &pkg.dmg.name) {
        return Err(VerifyError::WrongTrustedComment);
    }
    Ok(())
}

/// Verifies `dmg_bytes` against the minisign signature text and key, and that
/// the signed trusted comment names this package.
pub fn verify(
    pkg: &UpdatePackage,
    dmg_bytes: &[u8],
    minisig: &str,
    public_key: &str,
) -> Result<(), VerifyError> {
    check(pkg, minisig, public_key, |s| {
        s.update(dmg_bytes);
        Ok(())
    })
}

/// Like [`verify`], streaming the file from disk in chunks.
pub fn verify_file(
    pkg: &UpdatePackage,
    dmg_path: &Path,
    minisig: &str,
    public_key: &str,
) -> Result<(), VerifyError> {
    check(pkg, minisig, public_key, |s| {
        let mut f = std::fs::File::open(dmg_path)?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            s.update(&buf[..n]);
        }
    })
}
