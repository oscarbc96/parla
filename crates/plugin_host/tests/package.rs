//! `.parlaplugin` unpacker. Packages are built here with `tar`; hostile ones write
//! raw header bytes so names the `tar` builder refuses still reach the
//! unpacker.

use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;
use plugin_host::package::{Package, PackageError, PackageSource, limits, pack};
use sha2::{Digest, Sha256};

const MANIFEST: &str = "id = \"hello-plugin\"\nname = \"Hello\"\nversion = \"0.1.0\"\napi = \"0.1\"\nlicense = \"MIT\"\n";
const WASM: &[u8] = b"\0asm\x0d\0\x01\0";

/// One archive entry: name bytes as written, type flag, data.
struct Entry {
    name: Vec<u8>,
    kind: tar::EntryType,
    data: Vec<u8>,
    link: Option<Vec<u8>>,
    /// Size written in the header when it differs from `data.len()`.
    size: Option<u64>,
}

fn file(name: &str, data: &[u8]) -> Entry {
    Entry {
        name: name.as_bytes().to_vec(),
        kind: tar::EntryType::Regular,
        data: data.to_vec(),
        link: None,
        size: None,
    }
}

fn dir(name: &str) -> Entry {
    Entry {
        name: name.as_bytes().to_vec(),
        kind: tar::EntryType::Directory,
        data: Vec::new(),
        link: None,
        size: None,
    }
}

fn valid_entries() -> Vec<Entry> {
    vec![
        file("plugin.toml", MANIFEST.as_bytes()),
        file("plugin.wasm", WASM),
        file("icon.svg", b"<svg/>"),
        file("LICENSE", b"MIT"),
        file("README.md", b"# Hello"),
        dir("assets/"),
        file("assets/logo.png", b"png"),
        file("i18n/en.ftl", b"plugin-name = Hello\n"),
    ]
}

/// A gzip tar of `entries`, headers written byte by byte.
fn archive(entries: &[Entry]) -> Vec<u8> {
    let mut tar = Vec::new();
    for e in entries {
        let mut h = tar::Header::new_gnu();
        {
            let old = h.as_old_mut();
            assert!(e.name.len() <= old.name.len(), "test name too long");
            old.name[..e.name.len()].copy_from_slice(&e.name);
            if let Some(link) = &e.link {
                old.linkname[..link.len()].copy_from_slice(link);
            }
        }
        h.set_entry_type(e.kind);
        h.set_mode(0o644);
        h.set_size(e.size.unwrap_or(e.data.len() as u64));
        h.set_cksum();
        tar.extend_from_slice(h.as_bytes());
        tar.extend_from_slice(&e.data);
        let pad = (512 - e.data.len() % 512) % 512;
        tar.extend(std::iter::repeat_n(0u8, pad));
    }
    tar.extend(std::iter::repeat_n(0u8, 1024));
    gzip(&tar)
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::best());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn registry(bytes: &[u8]) -> PackageSource {
    PackageSource::Registry { sha256: sha(bytes) }
}

fn open(bytes: &[u8]) -> Result<plugin_host::package::Staged, PackageError> {
    let staging = tempfile::tempdir().unwrap();
    let r = Package::open_bytes(bytes, registry(bytes), staging.path());
    assert_staging_clean_on_error(&r, staging.path());
    r
}

fn assert_staging_clean_on_error<T>(r: &Result<T, PackageError>, staging: &Path) {
    if r.is_err() {
        let left: Vec<_> = std::fs::read_dir(staging).unwrap().collect();
        assert!(left.is_empty(), "staging left behind: {left:?}");
    }
}

#[track_caller]
fn rejected(entries: Vec<Entry>) -> PackageError {
    match open(&archive(&entries)) {
        Ok(s) => panic!("expected a rejection, staged {:?}", s.manifest.id),
        Err(e) => e,
    }
}

fn with(extra: Entry) -> Vec<Entry> {
    let mut v = valid_entries();
    v.push(extra);
    v
}

#[test]
fn valid_package_stages() {
    let bytes = archive(&valid_entries());
    let staging = tempfile::tempdir().unwrap();
    let path = staging.path().join("hello-plugin-0.1.0.parlaplugin");
    std::fs::write(&path, &bytes).unwrap();
    let staged = Package::open(&path, registry(&bytes), staging.path()).unwrap();
    assert_eq!(staged.manifest.id.as_str(), "hello-plugin");
    assert_eq!(staged.sha256, sha(&bytes));
    let d = staged.dir.path();
    assert!(d.starts_with(staging.path()));
    assert_eq!(std::fs::read(d.join("plugin.wasm")).unwrap(), WASM);
    assert_eq!(std::fs::read(d.join("assets/logo.png")).unwrap(), b"png");
    assert_eq!(
        std::fs::read(d.join("i18n/en.ftl")).unwrap(),
        b"plugin-name = Hello\n"
    );
}

#[test]
fn file_source_needs_no_hash() {
    let bytes = archive(&valid_entries());
    let staging = tempfile::tempdir().unwrap();
    let staged = Package::open_bytes(&bytes, PackageSource::File, staging.path()).unwrap();
    assert_eq!(staged.sha256, sha(&bytes));
}

#[test]
fn hash_mismatch_is_rejected_before_unpack() {
    let bytes = archive(&valid_entries());
    let staging = tempfile::tempdir().unwrap();
    let r = Package::open_bytes(
        &bytes,
        PackageSource::Registry { sha256: [7; 32] },
        staging.path(),
    );
    assert!(matches!(r, Err(PackageError::HashMismatch)), "{r:?}");
    assert_staging_clean_on_error(&r, staging.path());
}

#[test]
fn package_with_dotdot_is_rejected() {
    for name in [
        "../evil.png",
        "assets/../../evil",
        "assets/../plugin.toml",
        "./plugin.toml",
    ] {
        let e = rejected(with(file(name, b"x")));
        assert!(matches!(e, PackageError::BadPath { .. }), "{name}: {e:?}");
    }
}

#[test]
fn absolute_and_odd_paths_are_rejected() {
    for name in [
        "/etc/passwd",
        "/plugin.toml",
        "assets\\..\\evil.png",
        "C:/evil.png",
        "assets//x.png",
        "assets/.hidden.png",
        "assets/x.png.",
        "assets/a b.png",
    ] {
        let e = rejected(with(file(name, b"x")));
        assert!(matches!(e, PackageError::BadPath { .. }), "{name}: {e:?}");
    }
}

#[test]
fn non_ascii_names_are_rejected() {
    // NFC and NFD spellings of the same name must not both land on disk.
    for name in ["assets/caf\u{e9}.png", "assets/cafe\u{301}.png"] {
        let e = rejected(with(file(name, b"x")));
        assert!(matches!(e, PackageError::BadPath { .. }), "{name}: {e:?}");
    }
}

#[test]
fn windows_reserved_names_are_rejected() {
    for name in [
        "assets/con.png",
        "assets/NUL.svg",
        "assets/Com1.png",
        "assets/lpt9.webp",
    ] {
        let e = rejected(with(file(name, b"x")));
        assert!(matches!(e, PackageError::BadPath { .. }), "{name}: {e:?}");
    }
}

#[test]
fn symlink_is_rejected() {
    let mut e = file("assets/link.png", b"");
    e.kind = tar::EntryType::Symlink;
    e.link = Some(b"/etc/passwd".to_vec());
    let err = rejected(with(e));
    assert!(matches!(err, PackageError::EntryType { .. }), "{err:?}");
}

#[test]
fn hard_link_and_devices_are_rejected() {
    for kind in [
        tar::EntryType::Link,
        tar::EntryType::Char,
        tar::EntryType::Block,
        tar::EntryType::Fifo,
        tar::EntryType::GNUSparse,
    ] {
        let mut e = file("assets/x.png", b"");
        e.kind = kind;
        e.link = Some(b"plugin.toml".to_vec());
        let err = rejected(with(e));
        // `tar` itself refuses a sparse header it cannot parse.
        let sparse = kind == tar::EntryType::GNUSparse && matches!(err, PackageError::Malformed(_));
        assert!(
            sparse || matches!(err, PackageError::EntryType { .. }),
            "{kind:?}: {err:?}"
        );
    }
}

#[test]
fn duplicate_entry_is_rejected() {
    let e = rejected(with(file("assets/logo.png", b"again")));
    assert!(matches!(e, PackageError::Duplicate { .. }), "{e:?}");
}

#[test]
fn case_insensitive_collision_is_rejected() {
    let e = rejected(with(file("assets/LOGO.png", b"x")));
    assert!(matches!(e, PackageError::Duplicate { .. }), "{e:?}");
}

#[test]
fn oversize_wasm_is_rejected_before_unpack() {
    let big = vec![0u8; limits::MAX_WASM_BYTES as usize + 1];
    let mut v = valid_entries();
    v[1] = file("plugin.wasm", &big);
    let e = rejected(v);
    assert!(matches!(e, PackageError::TooLarge { .. }), "{e:?}");
}

#[test]
fn oversize_asset_is_rejected() {
    let big = vec![0u8; limits::MAX_ASSET_BYTES as usize + 1];
    let e = rejected(with(file("assets/big.png", &big)));
    assert!(matches!(e, PackageError::TooLarge { .. }), "{e:?}");
}

#[test]
fn total_uncompressed_over_limit_is_rejected() {
    // Every file is under its own cap; the sum is over the package cap.
    let chunk = vec![0u8; limits::MAX_ASSET_BYTES as usize];
    let mut v = valid_entries();
    let n = limits::MAX_PACKAGE_BYTES as usize / chunk.len() + 1;
    for i in 0..n {
        v.push(file(&format!("assets/a{i}.png"), &chunk));
    }
    let e = rejected(v);
    assert!(matches!(e, PackageError::TooLarge { .. }), "{e:?}");
}

#[test]
fn compressed_file_over_limit_is_rejected() {
    let bytes = vec![1u8; limits::MAX_PACKAGE_BYTES as usize + 1];
    let staging = tempfile::tempdir().unwrap();
    let path = staging.path().join("big.parlaplugin");
    std::fs::write(&path, &bytes).unwrap();
    let r = Package::open(&path, PackageSource::File, staging.path());
    assert!(matches!(r, Err(PackageError::TooLarge { .. })), "{r:?}");
}

#[test]
fn gzip_bomb_in_header_data_is_rejected() {
    // A GNU long-name entry claiming 16 MB: the tar reader would buffer it
    // whole. The decompressed stream cap stops it.
    let mut e = file("././@LongLink", &vec![b'a'; 16 << 20]);
    e.kind = tar::EntryType::GNULongName;
    let e = rejected(with(e));
    assert!(matches!(e, PackageError::TooLarge { .. }), "{e:?}");
}

#[test]
fn directory_with_data_is_rejected() {
    let mut d = dir("themes/");
    d.data = vec![0; 1024];
    let e = rejected(with(d));
    assert!(matches!(e, PackageError::EntryType { .. }), "{e:?}");
}

#[test]
fn too_many_files_is_rejected() {
    let mut v = valid_entries();
    for i in 0..limits::MAX_FILES {
        v.push(file(&format!("assets/f{i}.png"), b"x"));
    }
    let e = rejected(v);
    assert!(matches!(e, PackageError::TooManyFiles), "{e:?}");
}

#[test]
fn unknown_top_level_is_rejected() {
    for name in [
        "evil.sh",
        "readme.md",
        "src/lib.rs",
        "assets/x.exe",
        "themes/x.toml",
    ] {
        let e = rejected(with(file(name, b"x")));
        assert!(
            matches!(e, PackageError::UnknownEntry { .. }),
            "{name}: {e:?}"
        );
    }
}

#[test]
fn missing_license_for_registry_is_rejected() {
    let v: Vec<Entry> = valid_entries()
        .into_iter()
        .filter(|e| e.name != b"LICENSE")
        .collect();
    let bytes = archive(&v);
    let e = open(&bytes).unwrap_err();
    assert!(matches!(e, PackageError::Missing("LICENSE")), "{e:?}");
    // A file install does not need one.
    let staging = tempfile::tempdir().unwrap();
    Package::open_bytes(&bytes, PackageSource::File, staging.path()).unwrap();
}

#[test]
fn missing_icon_for_registry_is_rejected() {
    let v: Vec<Entry> = valid_entries()
        .into_iter()
        .filter(|e| e.name != b"icon.svg")
        .collect();
    let e = open(&archive(&v)).unwrap_err();
    assert!(matches!(e, PackageError::Missing("icon.svg")), "{e:?}");
}

#[test]
fn missing_manifest_is_rejected() {
    let v: Vec<Entry> = valid_entries().into_iter().skip(1).collect();
    let e = open(&archive(&v)).unwrap_err();
    assert!(matches!(e, PackageError::Missing("plugin.toml")), "{e:?}");
}

#[test]
fn code_plugin_without_wasm_is_rejected() {
    let v: Vec<Entry> = valid_entries()
        .into_iter()
        .filter(|e| e.name != b"plugin.wasm")
        .collect();
    let e = open(&archive(&v)).unwrap_err();
    assert!(matches!(e, PackageError::Missing("plugin.wasm")), "{e:?}");
}

#[test]
fn wasm_without_magic_is_rejected() {
    let mut v = valid_entries();
    v[1] = file("plugin.wasm", b"#!/bin/sh\n");
    let e = rejected(v);
    assert!(matches!(e, PackageError::NotWasm), "{e:?}");
}

#[test]
fn data_plugin_with_wasm_is_rejected() {
    let toml = "id = \"some-themes\"\nname = \"T\"\nversion = \"1.0.0\"\napi = \"0.1\"\n[themes]\nfiles = [\"themes/t.json\"]\n";
    let mut v = valid_entries();
    v[0] = file("plugin.toml", toml.as_bytes());
    v.push(file("themes/t.json", b"{}"));
    let e = rejected(v);
    assert!(
        matches!(e, PackageError::Unexpected("plugin.wasm")),
        "{e:?}"
    );
}

#[test]
fn declared_theme_file_must_exist() {
    let toml = "id = \"some-themes\"\nname = \"T\"\nversion = \"1.0.0\"\napi = \"0.1\"\n[themes]\nfiles = [\"themes/t.json\"]\n";
    let v = vec![
        file("plugin.toml", toml.as_bytes()),
        file("icon.svg", b"<svg/>"),
        file("LICENSE", b"MIT"),
    ];
    let e = open(&archive(&v)).unwrap_err();
    assert!(matches!(e, PackageError::MissingDeclared { .. }), "{e:?}");
}

#[test]
fn i18n_without_en_is_rejected() {
    let mut v = valid_entries();
    v.retain(|e| e.name != b"i18n/en.ftl");
    v.push(file("i18n/es.ftl", b"plugin-name = Hola\n"));
    let e = open(&archive(&v)).unwrap_err();
    assert!(matches!(e, PackageError::Missing("i18n/en.ftl")), "{e:?}");
}

#[test]
fn dev_section_is_rejected_in_a_package() {
    let toml = format!("{MANIFEST}[dev]\nwasm = \"target/x.wasm\"\n");
    let mut v = valid_entries();
    v[0] = file("plugin.toml", toml.as_bytes());
    let e = rejected(v);
    assert!(matches!(e, PackageError::Manifest(_)), "{e:?}");
}

#[test]
fn not_a_gzip_tar_is_rejected() {
    for bytes in [
        b"PK\x03\x04zip".to_vec(),
        gzip(b"not a tar header at all"),
        Vec::new(),
    ] {
        let e = open(&bytes).unwrap_err();
        assert!(
            matches!(e, PackageError::Malformed(_) | PackageError::Missing(_)),
            "{e:?}"
        );
    }
}

#[test]
fn header_size_beyond_data_is_rejected() {
    let mut e = file("assets/short.png", b"abc");
    e.size = Some(4096);
    let mut v = valid_entries();
    v.push(e);
    let err = open(&archive(&v)).unwrap_err();
    assert!(matches!(err, PackageError::Malformed(_)), "{err:?}");
}

#[test]
fn staging_removed_on_error() {
    // Several files are written before the bad one is reached.
    let staging = tempfile::tempdir().unwrap();
    let bytes = archive(&with(file("zz/evil", b"x")));
    let r = Package::open_bytes(&bytes, PackageSource::File, staging.path());
    assert!(r.is_err());
    assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 0);
}

#[test]
fn staged_dir_removed_on_drop() {
    let staging = tempfile::tempdir().unwrap();
    let bytes = archive(&valid_entries());
    let staged = Package::open_bytes(&bytes, PackageSource::File, staging.path()).unwrap();
    drop(staged);
    assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 0);
}

#[test]
fn pack_then_open_round_trips() {
    let src = tempfile::tempdir().unwrap();
    let d = src.path();
    std::fs::write(d.join("plugin.toml"), MANIFEST).unwrap();
    std::fs::write(d.join("plugin.wasm"), WASM).unwrap();
    std::fs::write(d.join("icon.svg"), "<svg/>").unwrap();
    std::fs::write(d.join("LICENSE"), "MIT").unwrap();
    std::fs::create_dir_all(d.join("icons/a-very-long-icon-pack-name-to-pass-the-old-tar-limit"))
        .unwrap();
    let long = format!(
        "icons/a-very-long-icon-pack-name-to-pass-the-old-tar-limit/{}.svg",
        "n".repeat(60)
    );
    std::fs::write(d.join(&long), "<svg/>").unwrap();
    let bytes = pack(d).unwrap();
    assert_eq!(bytes, pack(d).unwrap(), "pack is deterministic");
    let staging = tempfile::tempdir().unwrap();
    let staged = Package::open_bytes(&bytes, registry(&bytes), staging.path()).unwrap();
    assert_eq!(staged.manifest.id.as_str(), "hello-plugin");
    assert_eq!(
        std::fs::read(staged.dir.path().join("plugin.wasm")).unwrap(),
        WASM
    );
    assert_eq!(
        std::fs::read(staged.dir.path().join(&long)).unwrap(),
        b"<svg/>"
    );
}

#[test]
fn pack_refuses_what_open_would_reject() {
    let src = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("plugin.toml"), MANIFEST).unwrap();
    std::fs::write(src.path().join("notes.txt"), "x").unwrap();
    let e = pack(src.path()).unwrap_err();
    assert!(matches!(e, PackageError::UnknownEntry { .. }), "{e:?}");
}

#[cfg(unix)]
#[test]
fn pack_refuses_symlinks() {
    let src = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("plugin.toml"), MANIFEST).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", src.path().join("LICENSE")).unwrap();
    let e = pack(src.path()).unwrap_err();
    assert!(matches!(e, PackageError::EntryType { .. }), "{e:?}");
}

/// A PAX extended header whose one record applies to the next entry.
fn pax(record: &str) -> Entry {
    let body = format!(" {record}\n");
    // The length prefix counts its own digits.
    let mut len = body.len() + 1;
    while len != body.len() + len.to_string().len() {
        len = body.len() + len.to_string().len();
    }
    let mut e = file("PaxHeaders/x", format!("{len}{body}").as_bytes());
    e.kind = tar::EntryType::XHeader;
    e
}

#[test]
fn pax_path_with_dotdot_is_rejected() {
    let mut v = valid_entries();
    v.push(pax("path=../evil.png"));
    v.push(file("assets/ok.png", b"x"));
    let e = rejected(v);
    assert!(matches!(e, PackageError::BadPath { .. }), "{e:?}");
}

#[test]
fn pax_size_over_cap_is_rejected() {
    let mut v = valid_entries();
    v.push(pax("size=99999999"));
    v.push(file("assets/ok.png", b"x"));
    let e = rejected(v);
    assert!(matches!(e, PackageError::TooLarge { .. }), "{e:?}");
}

#[test]
fn corrupted_packages_never_panic_or_leak() {
    // A cheap stand-in for the fuzz target: corrupt each byte of the
    // decompressed tar in turn.
    let mut tar = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::GzDecoder::new(&archive(&valid_entries())[..]),
        &mut tar,
    )
    .unwrap();
    let staging = tempfile::tempdir().unwrap();
    for i in (0..tar.len()).step_by(7) {
        let mut t = tar.clone();
        t[i] ^= 0xa5;
        let bytes = gzip(&t);
        if let Ok(s) = Package::open_bytes(&bytes, PackageSource::File, staging.path()) {
            assert!(s.dir.path().starts_with(staging.path()));
        }
        assert_eq!(
            std::fs::read_dir(staging.path()).unwrap().count(),
            0,
            "byte {i}"
        );
    }
}

#[test]
fn pax_global_header_is_ignored() {
    let mut g = pax("comment=made by some tar");
    g.kind = tar::EntryType::XGlobalHeader;
    let mut v = vec![g];
    v.extend(valid_entries());
    open(&archive(&v)).unwrap();
}

#[test]
fn too_many_dirs_is_rejected() {
    let mut v = valid_entries();
    for i in 0..limits::MAX_DIRS {
        v.push(dir(&format!("icons/p{i}/")));
    }
    let e = rejected(v);
    assert!(matches!(e, PackageError::TooManyDirs), "{e:?}");
    assert!(e.to_string().contains("directories"), "{e}");
}

fn tar_bytes(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::GzDecoder::new(&archive(entries)[..]),
        &mut out,
    )
    .unwrap();
    out
}

#[test]
fn bad_gzip_trailer_is_rejected() {
    let mut bytes = archive(&valid_entries());
    let n = bytes.len();
    bytes[n - 8] ^= 0xff; // CRC32
    let e = open(&bytes).unwrap_err();
    assert!(matches!(e, PackageError::Malformed(_)), "{e:?}");
    let mut bytes = archive(&valid_entries());
    bytes[n - 1] ^= 0x01; // ISIZE
    let e = open(&bytes).unwrap_err();
    assert!(matches!(e, PackageError::Malformed(_)), "{e:?}");
}

#[test]
fn trailing_data_after_gzip_is_rejected() {
    let mut bytes = archive(&valid_entries());
    bytes.extend_from_slice(b"hidden");
    let e = open(&bytes).unwrap_err();
    assert!(matches!(e, PackageError::Malformed(_)), "{e:?}");
    // A second gzip member is trailing data too.
    let mut bytes = archive(&valid_entries());
    bytes.extend(gzip(b"second member"));
    let e = open(&bytes).unwrap_err();
    assert!(matches!(e, PackageError::Malformed(_)), "{e:?}");
}

#[test]
fn data_after_tar_end_is_rejected() {
    let mut t = tar_bytes(&valid_entries());
    t.extend_from_slice(b"smuggled");
    let e = open(&gzip(&t)).unwrap_err();
    assert!(matches!(e, PackageError::Malformed(_)), "{e:?}");
    // Zero padding after the end is normal.
    let mut t = tar_bytes(&valid_entries());
    t.extend(std::iter::repeat_n(0u8, 8192));
    open(&gzip(&t)).unwrap();
}

#[test]
fn pack_skips_ds_store_and_git() {
    let src = tempfile::tempdir().unwrap();
    let d = src.path();
    std::fs::write(d.join("plugin.toml"), MANIFEST).unwrap();
    std::fs::write(d.join("plugin.wasm"), WASM).unwrap();
    std::fs::write(d.join(".DS_Store"), "x").unwrap();
    std::fs::create_dir_all(d.join("assets")).unwrap();
    std::fs::write(d.join("assets/.DS_Store"), "x").unwrap();
    std::fs::write(d.join("assets/a.png"), "x").unwrap();
    std::fs::create_dir_all(d.join(".git/objects")).unwrap();
    std::fs::write(d.join(".git/objects/evil"), "x").unwrap();
    let bytes = pack(d).unwrap();
    let staging = tempfile::tempdir().unwrap();
    let staged = Package::open_bytes(&bytes, PackageSource::File, staging.path()).unwrap();
    let s = staged.dir.path();
    assert!(!s.join(".DS_Store").exists());
    assert!(!s.join("assets/.DS_Store").exists());
    assert!(!s.join(".git").exists());
    assert!(s.join("assets/a.png").exists());
}

#[test]
fn pack_rejects_other_hidden_files() {
    for name in [".env", ".gitignore"] {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("plugin.toml"), MANIFEST).unwrap();
        std::fs::write(src.path().join(name), "x").unwrap();
        let e = pack(src.path()).unwrap_err();
        assert!(matches!(e, PackageError::BadPath { .. }), "{name}: {e:?}");
    }
}

#[test]
fn theme_outside_themes_folder_is_rejected() {
    let toml = "id = \"some-themes\"\nname = \"T\"\nversion = \"1.0.0\"\napi = \"0.1\"\n[themes]\nfiles = [\"assets/t.json\"]\n";
    let v = vec![
        file("plugin.toml", toml.as_bytes()),
        file("icon.svg", b"<svg/>"),
        file("LICENSE", b"MIT"),
    ];
    let e = open(&archive(&v)).unwrap_err();
    assert!(matches!(e, PackageError::Manifest(_)), "{e:?}");
}
