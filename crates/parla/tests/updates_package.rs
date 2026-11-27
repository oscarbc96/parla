use std::io::Cursor;

use parla::updates::ReleaseAsset;
use parla::updates::package::{
    DOWNLOAD_PREFIX, UpdatePackage, VerifyError, package_for, trusted_comment, update_public_key,
    verify, verify_file,
};

fn version() -> semver::Version {
    semver::Version::parse("0.6.0").unwrap()
}

const TAG: &str = "v0.6.0";
const DMG: &str = "Parla-0.6.0-macos-universal.dmg";
const SIG: &str = "Parla-0.6.0-macos-universal.dmg.minisig";

fn asset(name: &str) -> ReleaseAsset {
    ReleaseAsset {
        name: name.to_string(),
        browser_download_url: format!("{DOWNLOAD_PREFIX}{TAG}/{name}"),
        size: 42,
    }
}

fn assets() -> Vec<ReleaseAsset> {
    vec![
        asset("Parla-0.6.0-linux-x86_64.tar.gz"),
        asset(DMG),
        asset("SHA256SUMS"),
        asset(SIG),
    ]
}

fn package() -> UpdatePackage {
    package_for(&version(), TAG, &assets()).expect("package")
}

#[test]
fn picks_the_dmg_and_signature() {
    let pkg = package();
    assert_eq!(pkg.dmg.name, DMG);
    assert_eq!(pkg.signature.name, SIG);
    assert_eq!(pkg.tag, TAG);
    assert_eq!(pkg.version, version());
    assert_eq!(pkg.dmg.size, 42);
    assert_eq!(
        pkg.dmg.url.as_str(),
        format!("{DOWNLOAD_PREFIX}{TAG}/{DMG}")
    );
}

#[test]
fn ignores_other_assets() {
    let only_others = vec![
        asset("Parla-0.6.0-macos-universal.zip"),
        asset("Parla-0.5.0-macos-universal.dmg"),
        asset("Parla-0.5.0-macos-universal.dmg.minisig"),
    ];
    assert!(package_for(&version(), TAG, &only_others).is_none());
}

#[test]
fn none_when_signature_missing() {
    let list: Vec<_> = assets().into_iter().filter(|a| a.name != SIG).collect();
    assert!(package_for(&version(), TAG, &list).is_none());
}

#[test]
fn rejects_other_host_or_other_tag() {
    let mut list = assets();
    for a in &mut list {
        if a.name == DMG {
            a.browser_download_url = format!("https://evil.example/releases/download/{TAG}/{DMG}");
        }
    }
    assert!(package_for(&version(), TAG, &list).is_none());

    let mut list = assets();
    for a in &mut list {
        if a.name == SIG {
            a.browser_download_url = format!("{DOWNLOAD_PREFIX}v0.5.0/{SIG}");
        }
    }
    assert!(package_for(&version(), TAG, &list).is_none());
}

struct Keys {
    pk: String,
    sk: minisign::SecretKey,
    pkey: minisign::PublicKey,
}

fn keys() -> Keys {
    let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    Keys {
        pk: kp.pk.to_base64(),
        sk: kp.sk,
        pkey: kp.pk,
    }
}

fn sign(k: &Keys, data: &[u8], comment: &str) -> String {
    minisign::sign(Some(&k.pkey), &k.sk, Cursor::new(data), Some(comment), None)
        .unwrap()
        .into_string()
}

const BYTES: &[u8] = b"pretend this is a disk image";

#[test]
fn good_signature_accepted() {
    let k = keys();
    let pkg = package();
    let sig = sign(&k, BYTES, &trusted_comment(&version(), DMG));
    assert_eq!(verify(&pkg, BYTES, &sig, &k.pk), Ok(()));
}

#[test]
fn good_signature_accepted_from_a_file() {
    let k = keys();
    let pkg = package();
    let sig = sign(&k, BYTES, &trusted_comment(&version(), DMG));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(DMG);
    std::fs::write(&path, BYTES).unwrap();
    assert_eq!(verify_file(&pkg, &path, &sig, &k.pk), Ok(()));
    std::fs::write(&path, b"tampered").unwrap();
    assert_eq!(
        verify_file(&pkg, &path, &sig, &k.pk),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn wrong_key_rejected() {
    let k = keys();
    let other = keys();
    let pkg = package();
    let sig = sign(&k, BYTES, &trusted_comment(&version(), DMG));
    assert!(verify(&pkg, BYTES, &sig, &other.pk).is_err());
    assert_eq!(
        verify(&pkg, BYTES, &sig, "not a key"),
        Err(VerifyError::BadKey)
    );
}

#[test]
fn flipped_byte_rejected() {
    let k = keys();
    let pkg = package();
    let sig = sign(&k, BYTES, &trusted_comment(&version(), DMG));
    let mut bad = BYTES.to_vec();
    bad[3] ^= 1;
    assert_eq!(
        verify(&pkg, &bad, &sig, &k.pk),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn trusted_comment_for_another_version_rejected() {
    let k = keys();
    let pkg = package();
    let other = semver::Version::parse("0.5.0").unwrap();
    let sig = sign(&k, BYTES, &trusted_comment(&other, DMG));
    assert_eq!(
        verify(&pkg, BYTES, &sig, &k.pk),
        Err(VerifyError::WrongTrustedComment)
    );
}

#[test]
fn trusted_comment_for_another_file_rejected() {
    let k = keys();
    let pkg = package();
    let sig = sign(
        &k,
        BYTES,
        &trusted_comment(&version(), "Parla-0.6.0-macos-arm64.dmg"),
    );
    assert_eq!(
        verify(&pkg, BYTES, &sig, &k.pk),
        Err(VerifyError::WrongTrustedComment)
    );
}

#[test]
fn edited_trusted_comment_rejected() {
    let k = keys();
    let pkg = package();
    let good = trusted_comment(&version(), DMG);
    let sig = sign(
        &k,
        BYTES,
        &trusted_comment(&semver::Version::parse("0.5.0").unwrap(), DMG),
    );
    let edited = sig.replace(
        &trusted_comment(&semver::Version::parse("0.5.0").unwrap(), DMG),
        &good,
    );
    assert_ne!(edited, sig);
    assert_eq!(
        verify(&pkg, BYTES, &edited, &k.pk),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn trusted_comment_format() {
    assert_eq!(
        trusted_comment(&version(), DMG),
        format!("parla 0.6.0 {DMG}")
    );
}

#[test]
fn public_key_unset_in_tests() {
    assert_eq!(update_public_key(), None);
}
