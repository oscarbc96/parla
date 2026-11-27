use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use crate::{diff, extract, har, output, summary, verify};
use clap::{Parser, Subcommand};
use redact::{IdMap, Redactor, RuleSet};

#[derive(Parser)]
#[command(name = "recon", about = "Redact and inspect web-client captures")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Redact a HAR into per-exchange and per-frame files.
    Redact {
        /// Raw HAR file (never copied to the output).
        input: PathBuf,
        /// Output directory.
        #[arg(long)]
        out: PathBuf,
        /// ID map path (secret; default ~/.cache/parla/recon/idmap.json).
        #[arg(long)]
        map: Option<PathBuf>,
        /// Extra host suffix whose workspace subdomains are rewritten (repeatable).
        #[arg(long = "host-suffix")]
        host_suffix: Vec<String>,
    },
    /// Fail (exit 1) on any leftover secret, real ID, email, file URL or workspace host.
    Check {
        /// Directories or files to scan.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// List methods, events and response keys of a redacted directory.
    Summary { dir: PathBuf },
    /// Draft a docs/providers/<provider>/web-client section from a redacted directory; optionally copy fixtures.
    Extract {
        /// Redacted directory (output of `recon redact`).
        dir: PathBuf,
        /// Area page the draft is for (auth, boot, conversations, messages, ...).
        #[arg(long)]
        area: String,
        /// Copy the grouped exchanges here, re-checking every copy for leaks.
        #[arg(long)]
        fixtures: Option<PathBuf>,
    },
    /// Compare a browser capture with a Parla recording (names only, never values); exit 1 on anything missing.
    Diff {
        /// Redacted browser capture directory.
        browser: PathBuf,
        /// Redacted Parla recording directory (PARLA_RECORD output, redacted).
        parla: PathBuf,
    },
    /// Re-call the catalogue's read-only methods and compare response shapes with the fixtures.
    /// Token and cookie come from stdin (two lines), never argv or env.
    Verify {
        /// Catalogue path (default docs/providers/<provider>/web-client/catalogue.md).
        #[arg(long)]
        catalogue: Option<PathBuf>,
        /// Repository root the catalogue's fixture paths are relative to (default: cwd).
        #[arg(long)]
        root: Option<PathBuf>,
        /// ID map path (secret; default ~/.cache/parla/recon/idmap.json).
        #[arg(long)]
        map: Option<PathBuf>,
        /// Base URL of the owner's workspace.
        #[arg(long)]
        base_url: String,
        /// Requests per minute.
        #[arg(long, default_value_t = 10)]
        rate: u32,
    },
}

/// What a provider supplies to the CLI: its id, redaction rules and the
/// methods its drift check may ever call (everything else is refused).
pub struct Provider {
    pub id: String,
    pub rules: Arc<dyn RuleSet>,
    pub read_only_methods: Vec<String>,
}

fn default_map() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or("HOME is not set; pass --map")?;
    Ok(PathBuf::from(home).join(".cache/parla/recon/idmap.json"))
}

fn redact_cmd(
    rules: Arc<dyn RuleSet>,
    input: &Path,
    out: &Path,
    map: Option<PathBuf>,
    suffixes: &[String],
) -> Result<ExitCode, String> {
    let map_path = match map {
        Some(p) => p,
        None => default_map()?,
    };
    let text = std::fs::read_to_string(input).map_err(|e| format!("read input: {e}"))?;
    let parsed = har::parse(&text).map_err(|e| format!("input is not a HAR: {e}"))?;
    let idmap = IdMap::load(&map_path).map_err(|e| format!("load map: {e}"))?;
    let mut redactor = Redactor::new(idmap, rules);
    for suffix in suffixes {
        redactor = redactor.with_host_suffix(suffix);
    }
    let built = output::build(&parsed, &mut redactor);
    if !built.leaks.is_empty() {
        eprintln!(
            "refusing to write: redacted output still has {} leak(s):",
            built.leaks.len()
        );
        for (file, leak) in &built.leaks {
            eprintln!("  {file}:{leak}");
        }
        return Ok(ExitCode::from(1));
    }
    output::write(out, &built).map_err(|e| format!("write output: {e}"))?;
    redactor
        .map()
        .save(&map_path)
        .map_err(|e| format!("save map: {e}"))?;
    println!(
        "redacted {} exchanges, {} websocket frames -> {} ({} values mapped)",
        built.http,
        built.ws,
        out.display(),
        redactor.map().len()
    );
    Ok(ExitCode::SUCCESS)
}

fn check_cmd(rules: &dyn RuleSet, paths: &[PathBuf]) -> Result<ExitCode, String> {
    let (files, leaks) = summary::check_paths(paths, rules).map_err(|e| format!("scan: {e}"))?;
    if leaks.is_empty() {
        println!("clean ({files} files)");
        return Ok(ExitCode::SUCCESS);
    }
    for (path, leak) in &leaks {
        println!("{}:{leak}", path.display());
    }
    println!("{} leak(s) in {files} files", leaks.len());
    Ok(ExitCode::from(1))
}

fn extract_cmd(
    rules: &dyn RuleSet,
    dir: &Path,
    area: &str,
    fixtures: Option<&Path>,
) -> Result<ExitCode, String> {
    let groups = extract::extract(dir).map_err(|e| format!("extract: {e}"))?;
    let leaks = extract::leaks_in(&groups, rules).map_err(|e| format!("scan: {e}"))?;
    if !leaks.is_empty() {
        eprintln!("refusing to extract: {} leak(s) in the input:", leaks.len());
        for (path, leak) in &leaks {
            eprintln!("  {}:{leak}", path.display());
        }
        return Ok(ExitCode::from(1));
    }
    if let Some(dest) = fixtures {
        let leaks =
            extract::copy_fixtures(&groups, dest, rules).map_err(|e| format!("copy: {e}"))?;
        if !leaks.is_empty() {
            eprintln!("refusing: {} leak(s) in copied fixtures:", leaks.len());
            for (path, leak) in &leaks {
                eprintln!("  {}:{leak}", path.display());
            }
            return Ok(ExitCode::from(1));
        }
    }
    print!("{}", extract::render(rules.id(), area, &groups));
    Ok(ExitCode::SUCCESS)
}

fn diff_cmd(browser: &Path, parla: &Path) -> Result<ExitCode, String> {
    let load = |p: &Path| diff::Capture::load(p).map_err(|e| format!("load {}: {e}", p.display()));
    let items = diff::diff(&load(browser)?, &load(parla)?);
    for item in &items {
        println!("{item}");
    }
    if items.is_empty() {
        println!("no differences");
    }
    Ok(ExitCode::from(u8::from(
        items.iter().any(diff::DiffItem::is_missing),
    )))
}

struct VerifyArgs {
    catalogue: Option<PathBuf>,
    root: Option<PathBuf>,
    map: Option<PathBuf>,
    base_url: String,
    rate: u32,
}

fn verify_cmd(provider: &Provider, a: VerifyArgs) -> Result<ExitCode, String> {
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    // The token and cookie never leave a `Zeroizing` buffer: read into one with
    // room for a typical header value, trim it in place so no plain copy of the
    // secret is made, and let it wipe on drop.
    let mut next = |what: &str| -> Result<zeroize::Zeroizing<String>, String> {
        let mut line = zeroize::Zeroizing::new(String::with_capacity(4 * 1024));
        let n = stdin
            .read_line(&mut line)
            .map_err(|e| format!("read {what} from stdin: {e}"))?;
        let start = line.len() - line.trim_start().len();
        let end = line.trim_end().len();
        line.truncate(end);
        line.drain(..start);
        if n == 0 || line.is_empty() {
            return Err(format!(
                "expected {what} on stdin (two lines: token, cookie)"
            ));
        }
        Ok(line)
    };
    let token = next("the token")?;
    let cookie = next("the cookie")?;
    let map_path = match a.map {
        Some(p) => p,
        None => default_map()?,
    };
    let opts = verify::Options {
        catalogue: a.catalogue.unwrap_or_else(|| {
            PathBuf::from(format!(
                "docs/providers/{}/web-client/catalogue.md",
                provider.id
            ))
        }),
        root: a.root.unwrap_or_else(|| PathBuf::from(".")),
        base_url: a.base_url,
        per_minute: a.rate,
        allowlist: provider.read_only_methods.clone(),
        token,
        cookie,
        map: IdMap::load(&map_path).map_err(|e| format!("load map: {e}"))?,
        rules: provider.rules.clone(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    let report = rt.block_on(verify::run(opts))?;
    for line in &report.lines {
        println!("{line}");
    }
    println!(
        "verified {}, refused {}, skipped {}; {} note(s), {} failure(s)",
        report.verified, report.refused, report.skipped, report.notes, report.failures
    );
    Ok(ExitCode::from(u8::from(report.failed())))
}

/// Parse the process arguments and run the CLI for `provider`.
pub fn run(provider: &Provider) -> ExitCode {
    let cli = Cli::parse();
    let rules = provider.rules.clone();
    let result = match cli.command {
        Command::Redact {
            input,
            out,
            map,
            host_suffix,
        } => redact_cmd(rules, &input, &out, map, &host_suffix),
        Command::Check { paths } => check_cmd(&*rules, &paths),
        Command::Summary { dir } => summary::summarize(&dir)
            .map(|s| {
                print!("{s}");
                ExitCode::SUCCESS
            })
            .map_err(|e| format!("summary: {e}")),
        Command::Extract {
            dir,
            area,
            fixtures,
        } => extract_cmd(&*rules, &dir, &area, fixtures.as_deref()),
        Command::Diff { browser, parla } => diff_cmd(&browser, &parla),
        Command::Verify {
            catalogue,
            root,
            map,
            base_url,
            rate,
        } => verify_cmd(
            provider,
            VerifyArgs {
                catalogue,
                root,
                map,
                base_url,
                rate,
            },
        ),
    };
    result.unwrap_or_else(|e| {
        eprintln!("recon: {e}");
        ExitCode::from(2)
    })
}
