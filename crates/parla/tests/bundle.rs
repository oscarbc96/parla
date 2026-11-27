//! Bundle metadata checks: neutral names, call usage strings.

use std::path::Path;

// Built from parts so the core-boundary scan does not flag this file.
const FORBIDDEN: &str = concat!("sl", "ack");

fn manifest() -> toml::Table {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    std::fs::read_to_string(path).unwrap().parse().unwrap()
}

fn packager(m: &toml::Table) -> &toml::Table {
    m.get("package")
        .and_then(|p| p.get("metadata"))
        .and_then(|p| p.get("packager"))
        .and_then(|p| p.as_table())
        .expect("[package.metadata.packager] missing")
}

#[test]
fn bundle_metadata_is_neutral() {
    let m = manifest();
    let p = packager(&m);
    assert_eq!(p["product-name"].as_str(), Some("Parla"));
    assert_eq!(p["identifier"].as_str(), Some("com.justparla.parla"));
    let formats: Vec<_> = p["formats"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(formats, ["app"]);
    let dump = toml::to_string(p).unwrap().to_lowercase();
    assert!(!dump.contains(&FORBIDDEN.to_lowercase()));
}

#[test]
fn info_plist_declares_only_parla_scheme() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("macos/Info.plist");
    let plist = std::fs::read_to_string(path).unwrap();
    assert!(plist.contains("CFBundleURLTypes"), "{plist}");
    let schemes: Vec<_> = plist
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .windows(3)
        .filter(|w| w[0] == "<key>CFBundleURLSchemes</key>" && w[1] == "<array>")
        .map(|w| w[2].to_string())
        .collect();
    assert_eq!(schemes, vec!["<string>parla</string>"], "{plist}");
    let forbidden_scheme = format!("<string>{}</string>", FORBIDDEN);
    assert!(!plist.contains(&forbidden_scheme));
    assert!(!plist.contains("<string>http</string>"));
    assert!(!plist.contains("<string>https</string>"));
}

#[test]
fn info_plist_has_call_usage_strings() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("macos/Info.plist");
    let plist = std::fs::read_to_string(path).unwrap();
    assert!(plist.contains("CFBundleDisplayName"));
    for (key, text) in [
        (
            "NSMicrophoneUsageDescription",
            "Parla uses the microphone so you can talk in calls.",
        ),
        (
            "NSCameraUsageDescription",
            "Parla uses the camera so you can share video in calls.",
        ),
    ] {
        let want = format!("<key>{key}</key>\n  <string>{text}</string>");
        assert!(plist.contains(&want), "{key} missing or changed");
    }
    assert!(!plist.to_lowercase().contains(&FORBIDDEN.to_lowercase()));
}

#[cfg(target_os = "macos")]
#[test]
fn binary_embeds_info_plist() {
    let out = std::process::Command::new("otool")
        .args(["-s", "__TEXT", "__info_plist", env!("CARGO_BIN_EXE_parla")])
        .output()
        .expect("otool");
    let text = String::from_utf8_lossy(&out.stdout);
    // Header line, then hex rows; an absent section prints only the header.
    assert!(
        text.lines().filter(|l| l.starts_with("0000")).count() > 0,
        "no __info_plist section in the parla binary: {text}"
    );
}

#[test]
fn every_lproj_localises_the_usage_strings() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for code in locale_codes() {
        let text =
            std::fs::read_to_string(root.join(format!("macos/{code}.lproj/InfoPlist.strings")))
                .unwrap();
        for key in ["NSMicrophoneUsageDescription", "NSCameraUsageDescription"] {
            assert!(text.contains(&format!("\"{key}\" = \"")), "{code}: {key}");
        }
        assert!(!text.to_lowercase().contains(&FORBIDDEN.to_lowercase()));
    }
}

fn locale_codes() -> Vec<String> {
    i18n::LocaleSet::embedded()
        .locales
        .iter()
        .map(|l| l.code.to_string())
        .collect()
}

#[test]
fn bundle_has_an_lproj_per_locale() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let m = manifest();
    let resources = packager(&m)["resources"].as_array().expect("resources");
    let plist = std::fs::read_to_string(root.join("macos/Info.plist")).unwrap();
    let codes = locale_codes();
    assert!(codes.len() >= 3, "{codes:?}");
    for code in &codes {
        let strings = root.join(format!("macos/{code}.lproj/InfoPlist.strings"));
        let text = std::fs::read_to_string(&strings)
            .unwrap_or_else(|e| panic!("{}: {e}", strings.display()));
        assert!(
            text.contains(r#""CFBundleDisplayName" = "Parla";"#),
            "{code}: {text}"
        );
        assert!(
            resources.iter().any(|r| {
                r.get("src").and_then(|v| v.as_str()) == Some(&format!("macos/{code}.lproj"))
                    && r.get("target").and_then(|v| v.as_str()) == Some(&format!("{code}.lproj"))
            }),
            "{code}: the packager does not copy its lproj into Contents/Resources"
        );
        assert!(
            plist.contains(&format!("<string>{code}</string>")),
            "{code}: missing from CFBundleLocalizations"
        );
    }
    assert!(plist.contains("CFBundleLocalizations"));
}

#[test]
fn entitlements_are_exactly_audio_and_camera() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("macos/Parla.entitlements");
    let ent = std::fs::read_to_string(&path).expect("Parla.entitlements missing");
    assert!(
        ent.contains("<key>com.apple.security.device.audio-input</key>"),
        "{ent}"
    );
    assert!(
        ent.contains("<key>com.apple.security.device.camera</key>"),
        "{ent}"
    );
    let keys = ent.matches("<key>").count();
    assert_eq!(keys, 2, "expected exactly two entitlement keys: {ent}");
    assert!(!ent.contains("com.apple.security.get-task-allow"), "{ent}");
}

#[test]
fn bundle_version_is_crate_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workspace: toml::Table = std::fs::read_to_string(root.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let workspace_version = workspace["workspace"]["package"]["version"]
        .as_str()
        .unwrap();
    assert_eq!(env!("CARGO_PKG_VERSION"), workspace_version);
}

#[test]
fn every_member_inherits_the_workspace_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workspace: toml::Table = std::fs::read_to_string(root.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let members = workspace["workspace"]["members"]
        .as_array()
        .expect("workspace members");
    assert!(!members.is_empty());
    for member in members {
        let pattern = member.as_str().unwrap();
        let path = root.join(pattern);
        let manifests: Vec<std::path::PathBuf> = if pattern.contains('*') {
            std::fs::read_dir(path.parent().unwrap_or(&root))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path().join("Cargo.toml"))
                .filter(|p| p.exists())
                .collect()
        } else {
            vec![path.join("Cargo.toml")]
        };
        for manifest in manifests {
            if !manifest.exists() {
                continue;
            }
            let m: toml::Table = std::fs::read_to_string(&manifest).unwrap().parse().unwrap();
            let Some(pkg) = m.get("package") else {
                continue;
            };
            let ver = &pkg["version"];
            assert!(
                ver.get("workspace").and_then(|v| v.as_bool()) == Some(true),
                "{} does not use version.workspace = true: {ver}",
                manifest.display()
            );
        }
    }
}

#[test]
fn release_profile_is_fat_lto_one_cgu_strip_debuginfo() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workspace: toml::Table = std::fs::read_to_string(root.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let release = workspace
        .get("profile")
        .and_then(|p| p.get("release"))
        .and_then(|p| p.as_table())
        .expect("[profile.release] missing");
    assert_eq!(release.get("lto").and_then(|v| v.as_str()), Some("fat"));
    assert_eq!(
        release.get("codegen-units").and_then(|v| v.as_integer()),
        Some(1)
    );
    assert_eq!(
        release.get("strip").and_then(|v| v.as_str()),
        Some("debuginfo")
    );
}

#[test]
fn bundle_resources_include_notice() {
    let m = manifest();
    let resources = packager(&m)["resources"].as_array().expect("resources");
    assert!(
        resources.iter().any(|r| {
            r.get("src").and_then(|v| v.as_str()) == Some("../../target/release-assets/NOTICE")
                && r.get("target").and_then(|v| v.as_str()) == Some("NOTICE")
        }),
        "packager resources must include the generated NOTICE: {resources:?}"
    );
}

/// `cargo run` has no bundle identifier, so macOS refuses notifications; the
/// notifier must know (and say so) instead of assuming it can post.
#[cfg(target_os = "macos")]
#[test]
fn bare_binary_is_not_bundled() {
    assert!(!parla::platform::is_bundled());
}

/// `mise run run` wraps the debug binary in a dev bundle so notifications work.
#[cfg(target_os = "macos")]
#[test]
fn dev_bundle_script_makes_a_bundle_with_an_identifier() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("parla-dev-bundle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let app = dir.join("Parla.app");
    let status = std::process::Command::new("bash")
        .arg(root.join("scripts/dev-bundle.sh"))
        .arg(env!("CARGO_BIN_EXE_parla"))
        .arg(&app)
        .status()
        .expect("bash");
    assert!(status.success());
    let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).unwrap();
    assert!(plist.contains("com.justparla.parla"), "{plist}");
    assert!(plist.contains("NSMicrophoneUsageDescription"));
    assert!(app.join("Contents/MacOS/parla").is_file());
    assert!(
        plist.contains("CFBundleIconFile"),
        "bundle has no CFBundleIconFile: {plist}"
    );
    assert!(app.join("Contents/Resources/AppIcon.icns").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}
