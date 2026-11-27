//! Golden images on disk: lookup, update mode, failure output and the run report.
//!
//! Layout: `baselines/<os>/<suite>/<name>@<scale>x-<light|dark>.png` under this crate.
//! Output: `target/visual/<os>/` (failure triples, `report.md`, `report.html`, optional `shots/`).

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use image::RgbaImage;

use crate::compare::{Comparison, Tolerance, compare};
use crate::variant::{Appearance, Os, Variant};

/// Where baselines and output live and which modes are on.
#[derive(Debug, Clone)]
pub struct Config {
    pub os: Os,
    pub baseline_root: PathBuf,
    /// `target/visual`; per-OS output goes below it.
    pub out_root: PathBuf,
    /// `PARLA_VISUAL_UPDATE=1`: write the baseline and pass.
    pub update: bool,
    /// `CI` is set: a missing baseline is never created.
    pub ci: bool,
    /// `PARLA_VISUAL_SHOTS=1`: also write every rendered image.
    pub shots: bool,
}

impl Config {
    /// Configuration from the environment of the running test.
    pub fn from_env() -> Self {
        let flag =
            |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty() && v != "0" && v != "false");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map_or_else(|| manifest.join("../../target"), PathBuf::from);
        Self {
            os: Os::current(),
            baseline_root: manifest.join("baselines"),
            out_root: target.join("visual"),
            update: flag("PARLA_VISUAL_UPDATE"),
            ci: flag("CI"),
            shots: flag("PARLA_VISUAL_SHOTS"),
        }
    }

    fn os_out(&self) -> PathBuf {
        self.out_root.join(os_name(self.os))
    }
}

/// Lower-case OS directory name.
pub fn os_name(os: Os) -> &'static str {
    match os {
        Os::Macos => "macos",
        Os::Windows => "windows",
        Os::Linux => "linux",
    }
}

fn theme_name(a: Appearance) -> &'static str {
    match a {
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    }
}

/// `<name>@<scale>x-<light|dark>`
pub fn stem(name: &str, variant: &Variant) -> String {
    format!(
        "{name}@{}x-{}",
        variant.scale,
        theme_name(variant.appearance)
    )
}

/// Baseline path relative to the baseline root: `<os>/<suite>/<stem>.png`.
pub fn baseline_rel_path(os: Os, suite: &str, name: &str, variant: &Variant) -> PathBuf {
    Path::new(os_name(os))
        .join(suite)
        .join(format!("{}.png", stem(name, variant)))
}

/// Encode as PNG, then optimise (oxipng level 4, strip all).
pub fn encode_optimised(img: &RgbaImage) -> anyhow::Result<Vec<u8>> {
    let raw = encode_raw(img)?;
    let mut opts = oxipng::Options::from_preset(4);
    opts.strip = oxipng::StripChunks::All;
    Ok(oxipng::optimize_from_memory(&raw, &opts)?)
}

/// Plain PNG encode.
pub fn encode_raw(img: &RgbaImage) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Updated,
    Fail,
    Missing,
    SizeMismatch,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Updated => "updated",
            Status::Fail => "FAIL",
            Status::Missing => "MISSING",
            Status::SizeMismatch => "SIZE",
        }
    }
    fn failed(self) -> bool {
        matches!(self, Status::Fail | Status::Missing | Status::SizeMismatch)
    }
}

struct Record {
    suite: String,
    name: String,
    scale: String,
    theme: &'static str,
    status: Status,
    ratio: f64,
    /// Report-relative `expected`, `actual`, `diff` links (empty when not written).
    links: [String; 3],
}

impl Record {
    fn to_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.suite,
            self.name,
            self.scale,
            self.theme,
            self.status.label(),
            self.ratio,
            self.links[0],
            self.links[1],
            self.links[2]
        )
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Write beside the target and rename, so concurrent test processes reading the
    // records never see a half-written file.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let tmp = path.with_file_name(tmp_name);
    fs::write(&tmp, bytes)?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// Compare `actual` with its baseline under `cfg`; `Err` carries the failure message.
pub fn check(
    cfg: &Config,
    suite: &str,
    name: &str,
    variant: &Variant,
    actual: &RgbaImage,
    tol: Tolerance,
) -> Result<(), String> {
    let stem = stem(name, variant);
    let base = cfg
        .baseline_root
        .join(baseline_rel_path(cfg.os, suite, name, variant));
    let os_out = cfg.os_out();
    let rel = |kind: &str| format!("{suite}/{stem}.{kind}.png");
    let mut links: [String; 3] = Default::default();

    let io =
        |r: anyhow::Result<()>| r.map_err(|e| format!("visual io error for {suite}/{stem}: {e}"));

    if cfg.shots {
        let shot = os_out.join("shots").join(suite).join(format!("{stem}.png"));
        io(encode_raw(actual).and_then(|b| write_file(&shot, &b)))?;
    }

    let mut message = None;
    let (status, ratio) = if cfg.update {
        io(encode_optimised(actual).and_then(|b| write_file(&base, &b)))?;
        (Status::Updated, 0.0)
    } else if !base.exists() {
        message = Some(format!(
            "missing baseline {}: run mise run test:visual:update{}",
            base.display(),
            if cfg.ci { " (not created in CI)" } else { "" }
        ));
        (Status::Missing, 1.0)
    } else {
        let expected = image::open(&base)
            .map_err(|e| format!("cannot read {}: {e}", base.display()))?
            .to_rgba8();
        match compare(actual, &expected, tol) {
            Err(e) => {
                write_triple(
                    &os_out,
                    suite,
                    &stem,
                    actual,
                    Some(&expected),
                    None,
                    &mut links,
                )
                .map_err(|e| e.to_string())?;
                message = Some(format!("{e} for {}", base.display()));
                (Status::SizeMismatch, 1.0)
            }
            Ok(c) if c.within(tol) => (Status::Pass, c.ratio()),
            Ok(c) => {
                write_triple(
                    &os_out,
                    suite,
                    &stem,
                    actual,
                    Some(&expected),
                    Some(&c),
                    &mut links,
                )
                .map_err(|e| e.to_string())?;
                message = Some(format!(
                    "{suite}/{stem}: {} of {} pixels differ ({:.4}%, allowed {:.4}%)\n  expected: {}\n  actual:   {}\n  diff:     {}",
                    c.differing,
                    c.total,
                    c.ratio() * 100.0,
                    tol.max_ratio * 100.0,
                    os_out.join(rel("expected")).display(),
                    os_out.join(rel("actual")).display(),
                    os_out.join(rel("diff")).display(),
                ));
                (Status::Fail, c.ratio())
            }
        }
    };
    if cfg.shots {
        links[1] = format!("shots/{suite}/{stem}.png");
    }
    let rec = Record {
        suite: suite.into(),
        name: name.into(),
        scale: variant.scale.to_string(),
        theme: theme_name(variant.appearance),
        status,
        ratio,
        links,
    };
    io(write_record(&os_out, &rec).and_then(|()| write_reports(&os_out)))?;
    match message {
        Some(m) if status.failed() => Err(m),
        _ => Ok(()),
    }
}

fn write_triple(
    os_out: &Path,
    suite: &str,
    stem: &str,
    actual: &RgbaImage,
    expected: Option<&RgbaImage>,
    cmp: Option<&Comparison>,
    links: &mut [String; 3],
) -> anyhow::Result<()> {
    let dir = os_out.join(suite);
    let put = |kind: &str, img: &RgbaImage| {
        write_file(&dir.join(format!("{stem}.{kind}.png")), &encode_raw(img)?)
    };
    put("actual", actual)?;
    links[1] = format!("{suite}/{stem}.actual.png");
    if let Some(e) = expected {
        put("expected", e)?;
        links[0] = format!("{suite}/{stem}.expected.png");
    }
    if let Some(c) = cmp {
        put("diff", &c.diff)?;
        links[2] = format!("{suite}/{stem}.diff.png");
    }
    Ok(())
}

/// Fields in one record line (see `Record::to_line`).
const RECORD_FIELDS: usize = 9;

fn write_record(os_out: &Path, rec: &Record) -> anyhow::Result<()> {
    let path = os_out
        .join("records")
        .join(&rec.suite)
        .join(format!("{}@{}x-{}.tsv", rec.name, rec.scale, rec.theme));
    write_file(&path, rec.to_line().as_bytes())
}

/// Rebuild `report.md` and `report.html` from every record written so far.
fn write_reports(os_out: &Path) -> anyhow::Result<()> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut stack = vec![os_out.join("records")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let p = entry?.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "tsv") {
                // A record that is unreadable or short is skipped: the next report rebuild
                // sees it complete.
                let Ok(text) = fs::read_to_string(&p) else {
                    continue;
                };
                let row: Vec<String> = text.split('\t').map(str::to_owned).collect();
                if row.len() >= RECORD_FIELDS {
                    rows.push(row);
                }
            }
        }
    }
    let is_fail = |r: &Vec<String>| matches!(r[4].as_str(), "FAIL" | "MISSING" | "SIZE");
    rows.sort_by(|a, b| {
        (!is_fail(a), &a[0], &a[1], &a[2], &a[3]).cmp(&(!is_fail(b), &b[0], &b[1], &b[2], &b[3]))
    });
    let fails = rows.iter().filter(|r| is_fail(r)).count();

    let mut md = format!(
        "# Visual report\n\n{} images, {} failing\n\n| suite | name | scale | theme | status | ratio | expected | actual | diff |\n|---|---|---|---|---|---|---|---|---|\n",
        rows.len(),
        fails
    );
    let link = |l: &str| {
        if l.is_empty() {
            String::new()
        } else {
            format!("[png]({l})")
        }
    };
    for r in &rows {
        let ratio: f64 = r[5].parse().unwrap_or(0.0);
        let _ = writeln!(
            md,
            "| {} | {} | {}x | {} | {} | {:.4}% | {} | {} | {} |",
            r[0],
            r[1],
            r[2],
            r[3],
            r[4],
            ratio * 100.0,
            link(&r[6]),
            link(&r[7]),
            link(&r[8])
        );
    }
    write_file(&os_out.join("report.md"), md.as_bytes())?;

    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('"', "&quot;")
    };
    let mut html = String::from(
        "<!doctype html><meta charset=\"utf-8\"><title>Visual report</title><style>body{font:14px sans-serif;margin:16px}figure{display:inline-block;margin:4px}img{max-width:30vw;border:1px solid #888}.bad h2{color:#c00}</style>",
    );
    let _ = write!(
        html,
        "<h1>Visual report</h1><p>{} images, {fails} failing</p>",
        rows.len()
    );
    for r in &rows {
        let ratio: f64 = r[5].parse().unwrap_or(0.0);
        let _ = write!(
            html,
            "<section class=\"{}\"><h2>{} / {} @{}x {} : {} ({:.4}%)</h2>",
            if is_fail(r) { "bad" } else { "ok" },
            esc(&r[0]),
            esc(&r[1]),
            esc(&r[2]),
            esc(&r[3]),
            esc(&r[4]),
            ratio * 100.0
        );
        for (i, label) in ["expected", "actual", "diff"].iter().enumerate() {
            if !r[6 + i].is_empty() {
                let _ = write!(
                    html,
                    "<figure><figcaption>{label}</figcaption><img src=\"{}\"></figure>",
                    esc(&r[6 + i])
                );
            }
        }
        html.push_str("</section>");
    }
    write_file(&os_out.join("report.html"), html.as_bytes())?;
    Ok(())
}

/// Assert `actual` matches the baseline for `(suite, name, variant)`; panics with the artefact paths otherwise.
pub fn assert_matches(
    suite: &str,
    name: &str,
    variant: &Variant,
    actual: &RgbaImage,
    tol: Tolerance,
) {
    if let Err(msg) = check(&Config::from_env(), suite, name, variant, actual, tol) {
        panic!("{msg}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn cfg(dir: &Path) -> Config {
        Config {
            os: Os::Linux,
            baseline_root: dir.join("baselines"),
            out_root: dir.join("out"),
            update: false,
            ci: false,
            shots: false,
        }
    }

    fn v() -> Variant {
        Variant {
            appearance: Appearance::Dark,
            scale: 2.0,
        }
    }

    fn pattern() -> RgbaImage {
        RgbaImage::from_fn(64, 64, |x, y| {
            Rgba([(x * 4) as u8, (y * 4) as u8, ((x ^ y) * 3) as u8, 255])
        })
    }

    #[test]
    fn baseline_path_format() {
        let p = baseline_rel_path(
            Os::Macos,
            "sidebar",
            "empty",
            &Variant {
                appearance: Appearance::Light,
                scale: 2.0,
            },
        );
        assert_eq!(p, Path::new("macos/sidebar/empty@2x-light.png"));
        let p = baseline_rel_path(
            Os::Linux,
            "s",
            "n",
            &Variant {
                appearance: Appearance::Dark,
                scale: 1.5,
            },
        );
        assert_eq!(p, Path::new("linux/s/n@1.5x-dark.png"));
    }

    #[test]
    fn update_writes_optimised_png_and_passes() {
        let d = tempfile::tempdir().unwrap();
        let mut c = cfg(d.path());
        c.update = true;
        let img = pattern();
        check(&c, "s", "n", &v(), &img, Tolerance::default()).unwrap();
        let path = c.baseline_root.join("linux/s/n@2x-dark.png");
        let written = fs::read(&path).unwrap();
        assert!(written.len() < encode_raw(&img).unwrap().len());
        assert_eq!(image::open(&path).unwrap().to_rgba8(), img);
        c.update = false;
        check(&c, "s", "n", &v(), &img, Tolerance::default()).unwrap();
    }

    #[test]
    fn missing_baseline_fails_in_ci() {
        let d = tempfile::tempdir().unwrap();
        let mut c = cfg(d.path());
        c.ci = true;
        let err = check(&c, "s", "n", &v(), &pattern(), Tolerance::default()).unwrap_err();
        assert!(err.contains("mise run test:visual:update"), "{err}");
        assert!(!c.baseline_root.join("linux/s/n@2x-dark.png").exists());
    }

    #[test]
    fn mismatch_writes_triple_and_names_paths() {
        let d = tempfile::tempdir().unwrap();
        let mut c = cfg(d.path());
        c.update = true;
        check(&c, "s", "n", &v(), &pattern(), Tolerance::default()).unwrap();
        c.update = false;
        let mut other = pattern();
        for x in 0..10 {
            other.put_pixel(x, 0, Rgba([255, 255, 255, 255]));
        }
        let err = check(&c, "s", "n", &v(), &other, Tolerance::default()).unwrap_err();
        for kind in ["actual", "expected", "diff"] {
            let p = c.out_root.join(format!("linux/s/n@2x-dark.{kind}.png"));
            assert!(p.exists(), "{kind}");
            assert!(err.contains(&p.display().to_string()), "{err}");
        }
    }

    #[test]
    fn report_lists_every_image_with_status_and_ratio() {
        let d = tempfile::tempdir().unwrap();
        let mut c = cfg(d.path());
        c.update = true;
        check(&c, "s", "good", &v(), &pattern(), Tolerance::default()).unwrap();
        c.update = false;
        check(&c, "s", "good", &v(), &pattern(), Tolerance::default()).unwrap();
        let mut bad = pattern();
        for x in 0..10 {
            bad.put_pixel(x, 0, Rgba([255, 255, 255, 255]));
        }
        c.update = true;
        check(&c, "s", "bad", &v(), &pattern(), Tolerance::default()).unwrap();
        c.update = false;
        let _ = check(&c, "s", "bad", &v(), &bad, Tolerance::default()).unwrap_err();
        let md = fs::read_to_string(c.out_root.join("linux/report.md")).unwrap();
        assert!(md.contains("| s | good | 2x | dark | pass |"), "{md}");
        assert!(
            md.contains("| s | bad | 2x | dark | FAIL | 0.2441%"),
            "{md}"
        );
        assert!(md.contains("s/bad@2x-dark.diff.png"), "{md}");
        let html = fs::read_to_string(c.out_root.join("linux/report.html")).unwrap();
        assert!(
            html.find("bad").unwrap() < html.find("good").unwrap(),
            "failures first"
        );
    }

    #[test]
    fn report_skips_empty_and_truncated_records() {
        let d = tempfile::tempdir().unwrap();
        let mut c = cfg(d.path());
        c.update = true;
        check(&c, "s", "good", &v(), &pattern(), Tolerance::default()).unwrap();
        c.update = false;
        let os_out = c.out_root.join("linux");
        let dir = os_out.join("records/s");
        fs::write(dir.join("empty@2x-dark.tsv"), "").unwrap();
        fs::write(dir.join("cut@2x-dark.tsv"), "s\tcut\t2").unwrap();
        fs::write(dir.join("half.tmp"), "s\tx").unwrap();
        write_reports(&os_out).unwrap();
        let md = fs::read_to_string(os_out.join("report.md")).unwrap();
        assert!(md.contains("1 images, 0 failing"), "{md}");
        assert!(md.contains("| s | good |"), "{md}");
    }

    #[test]
    fn write_file_is_atomic_and_leaves_no_partial_file() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("a/b.tsv");
        write_file(&path, b"one").unwrap();
        write_file(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("b.tsv")]);
    }

    #[test]
    fn shots_mode_writes_passing_images() {
        let d = tempfile::tempdir().unwrap();
        let mut c = cfg(d.path());
        c.update = true;
        check(&c, "s", "n", &v(), &pattern(), Tolerance::default()).unwrap();
        c.update = false;
        c.shots = true;
        check(&c, "s", "n", &v(), &pattern(), Tolerance::default()).unwrap();
        assert!(c.out_root.join("linux/shots/s/n@2x-dark.png").exists());
    }
}
