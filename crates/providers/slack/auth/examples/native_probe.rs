//! spike: the email-code sign-in end to end without a webview.
//!
//! Reads the email address from stdin, asks the service to email a code, reads
//! the code from stdin, confirms it, lists the email's workspaces, opens the
//! first one with a login link (`slack_auth::native::open_team`), reads its
//! token from the boot page (`boot_token`) and runs `auth.test`.
//!
//! This probe NEVER prints an email address, code, link, cookie, token, team id
//! or name: only booleans, counts and lengths (same rule as `login_probe`).
//! Verdicts: PASS (cookie, token, `auth.test` ok), PARTIAL (cookie, no token) or FAIL.
//!
//! Run (owner only, against the test workspace):
//! `mise exec -- cargo run -p slack_auth --example native_probe`

use std::io::BufRead;

use slack_api::{
    ClientConfig, Secret, SendCodeOutcome, WebClient, XoxcToken, confirm_code,
    find_workspaces_for_email, send_code,
};
use slack_auth::native::{boot_token, open_team};
use url::Url;
use zeroize::Zeroizing;

fn read_line(prompt: &str) -> Zeroizing<String> {
    println!("{prompt}");
    let mut s = Zeroizing::new(String::new());
    std::io::stdin().lock().read_line(&mut s).expect("stdin");
    Zeroizing::new(s.trim().to_string())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let base = Url::parse("https://slack.com/").unwrap();
    let email = read_line("email address (not echoed back):");
    match send_code(&base, &email, "en-US", false).await {
        Ok(SendCodeOutcome::Sent { resend_after }) => {
            println!(
                "send_code: sent, resend_after_known={}",
                resend_after.is_some()
            )
        }
        Ok(other) => return println!("send_code: {other:?} (FAIL for the code path)"),
        Err(e) => return println!("send_code: error kind={}", kind(&e)),
    }
    let code = read_line("code from the email:");
    println!("code: len={}", code.len());
    let code = Secret::new(code.to_string());
    let conf = match confirm_code(&base, &email, &code).await {
        Ok(c) => c,
        Err(e) => return println!("confirm_code: error kind={}", kind(&e)),
    };
    println!("confirm_code: ok, cookies={}", conf.cookie_count());
    let rows = match find_workspaces_for_email(&base, &conf, &email).await {
        Ok(r) => r,
        Err(e) => return println!("find_workspaces_for_email: error kind={}", kind(&e)),
    };
    let with_link = rows.iter().filter(|r| r.login_link.is_some()).count();
    let sso = rows.iter().filter(|r| r.team.sso_required).count();
    println!(
        "workspaces: count={}, with_link={with_link}, sso_required={sso}",
        rows.len()
    );
    let Some(row) = rows.iter().find(|r| r.login_link.is_some()) else {
        return println!("verdict: FAIL (no login link)");
    };
    let link = row.login_link.as_ref().unwrap();
    let cookie = match open_team(link, &base).await {
        Ok(c) => c,
        Err(e) => return println!("open_team: {e:?}; verdict: FAIL"),
    };
    println!(
        "open_team: d_len={}, has_d_s={}",
        cookie.d_value().len(),
        cookie.d_s_value().is_some()
    );
    let team_url = match Url::parse(&format!("https://{}.slack.com/", row.team.domain)) {
        Ok(u) => u,
        Err(_) => return println!("team url: malformed; verdict: PARTIAL"),
    };
    let token = match boot_token(&team_url, &cookie).await {
        Ok(t) => t,
        Err(e) => return println!("boot_token: {e:?}; verdict: PARTIAL"),
    };
    println!("boot_token: len={}", token.expose().len());
    let Ok(token) = XoxcToken::parse(token.expose()) else {
        return println!("boot_token: not a session token; verdict: PARTIAL");
    };
    let ok = match WebClient::new(ClientConfig::for_workspace(&team_url), token, cookie) {
        Ok(client) => client.auth_test().await.is_ok(),
        Err(_) => false,
    };
    println!(
        "auth.test ok={ok}; verdict: {}",
        if ok { "PASS" } else { "PARTIAL" }
    );
}

/// The error's variant name only, never its text.
fn kind(e: &slack_api::ApiError) -> &'static str {
    use slack_api::ApiError::*;
    match e {
        InvalidAuth => "invalid_auth",
        Slack { .. } => "service_error",
        RateLimited { .. } => "rate_limited",
        Http { .. } => "http",
        Transport(_) => "transport",
        WrongCode { .. } => "wrong_code",
        CodeExpired => "code_expired",
        TooManyAttempts => "too_many_attempts",
        _ => "other",
    }
}
