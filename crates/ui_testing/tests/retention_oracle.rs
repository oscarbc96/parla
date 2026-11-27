//! GPUI Fast retains views between frames. A view that reads state outside
//! entities and globals and does not `cx.notify()` would paint stale content.
//! The oracle renders every scenario for a few frames with retention on and off
//! and requires identical images.
// Windows has no headless renderer at this rev; `harness = false` still needs a `main`
// there, so the target compiles to a no-op.

#[cfg(not(windows))]
use ui_testing::stage::{self, StageOptions};
#[cfg(not(windows))]
use ui_testing::{UiHarness, Variant, scenario};

#[cfg(not(windows))]
const FRAMES: usize = 3;

/// Flags that take a value: the argument after one is that flag's value, not
/// a scenario name (`--test-threads 2` must not ask for a scenario "2").
#[cfg(not(windows))]
const VALUE_FLAGS: &[&str] = &[
    "--test-threads",
    "--format",
    "--logfile",
    "--skip",
    "--color",
    "-Z",
];

#[cfg(not(windows))]
fn frames(v: Variant, name: &str, retention: bool) -> Vec<image::RgbaImage> {
    let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
    h.set_view_retention(retention);
    let s = scenario::by_name(name).unwrap_or_else(|| panic!("no scenario {name}"));
    stage::capture_frames(&mut h, s, StageOptions::default(), FRAMES).unwrap()
}

#[cfg(windows)]
fn main() {}

/// The scenario names `args` asks for: every argument that is not a flag and
/// not the value of a flag that takes one.
#[cfg(not(windows))]
fn asked_names(args: &[String]) -> Vec<&str> {
    let mut asked = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg.starts_with('-') {
            if VALUE_FLAGS.contains(&arg.as_str()) {
                args.next();
            }
            continue;
        }
        asked.push(arg.as_str());
    }
    asked
}

/// A `harness = false` binary speaks nextest's protocol itself: `--list`
/// names one test per scenario (none for `--ignored`), and a scenario runs
/// when asked by name (all of them with no name, for `cargo test`). Without
/// the listing nextest sees no tests and never runs the oracle; one test per
/// scenario lets nextest spread them over its workers and stay inside its
/// per-test timeout.
#[cfg(not(windows))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            for s in scenario::all() {
                println!("{}: test", s.name);
            }
        }
        return;
    }
    let asked = asked_names(&args);
    let unknown: Vec<&str> = asked
        .iter()
        .copied()
        .filter(|name| scenario::by_name(name).is_none())
        .collect();
    if !unknown.is_empty() {
        for name in unknown {
            eprintln!("no scenario named {name}");
        }
        std::process::exit(1);
    }
    let mut stale = Vec::new();
    let mut skipped = Vec::new();
    for s in scenario::all() {
        if asked.is_empty() || asked.contains(&s.name) {
            stale.extend(retention_differences(s.name, &mut skipped));
        }
    }
    if !skipped.is_empty() {
        skipped.sort_unstable();
        skipped.dedup();
        eprintln!(
            "skipped {} non-deterministic scenario(s): {}",
            skipped.len(),
            skipped.join(", ")
        );
    }
    if !stale.is_empty() {
        eprintln!("retained frames differ: {stale:?}");
        std::process::exit(1);
    }
}

/// Frames of `name` that differ between retention on and off, in every variant.
#[cfg(not(windows))]
fn retention_differences(name: &'static str, skipped: &mut Vec<&'static str>) -> Vec<String> {
    let mut stale = Vec::new();
    for v in Variant::defaults() {
        let on = frames(v, name, true);
        let off = frames(v, name, false);
        // A scenario that does not draw the same twice with retention on
        // (images that arrive on their own) says nothing about retention.
        let again = frames(v, name, true);
        if on.iter().zip(&again).any(|(a, b)| a.as_raw() != b.as_raw()) {
            eprintln!("{name}: not deterministic with retention on, skipped");
            skipped.push(name);
            continue;
        }
        for (i, (a, b)) in on.iter().zip(&off).enumerate() {
            if a.as_raw() != b.as_raw() {
                stale.push(format!("{name} frame {i}"));
            }
        }
    }
    stale
}
