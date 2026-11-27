# auth

## `localConfig_v2` (localStorage of `app.slack.com`)

Shape as read by community extractors (wee-slack); **not yet confirmed on a live
sign-in**. Values
below are synthetic.

```json
{
  "teams": {
    "T0000001": {
      "id": "T0000001",
      "name": "Example Team",
      "url": "https://workspace1.slack.com/",
      "token": "{{TOKEN_A}}"
    },
    "E0000001": { "id": "E0000001", "name": "Example Org" }
  }
}
```

Entries whose id starts with `E` are org-level and are skipped. Only the
presence, count and field names of this blob may be logged, never the values.

## Desktop-app handoff after sign-in

Verified 2026-11-18 (recon, redacted). After a successful sign-in Slack sends
the webview to `https://<ws>.slack.com/ssb/redirect`, a page titled "Starting
<ws>" that tries `slack://` and offers "use Slack in the browser". The URL
carries only the workspace subdomain, no team id. Parla handles it in the
navigation handler (`webview/policy.rs`, `handoff`) while signing in:

- the navigation is not loaded and the capture (`run_capture_with`) loads
  `https://app.slack.com/client/<team id>` instead, so the capture finishes
  with no click;
- a `slack://<team id>/...` or `slack://...?team=<team id>` link carries the
  id, so it needs no lookup;
- for `ssb/redirect` the id is resolved from the subdomain with
  `signin.findWorkspaces` (below), called with the session cookie from the
  webview. If that fails the handoff page loads as it is, once per URL (no
  repeated lookups), so the user can still click through (the old behaviour).

## Signed-in workspaces (`signin.findWorkspaces`)

Verified 2026-11-18 against a redacted recon capture; fixture
`crates/providers/slack/api/tests/fixtures/signin-find-workspaces/0045-signin.findWorkspaces.json`.
`POST https://slack.com/api/signin.findWorkspaces`, multipart form, cookie
authenticated (the `d` cookie), **no token**, `Origin: https://slack.com`.
Form as the web client sends it on the sign-in page when a session exists:
`only_logged_in_teams=true`, `ssb_signin=false`,
`_x_reason=get_started_already_signed_into`, `_x_mode=online`. The workspace
chooser sends a different variant (`commingled_response=false`,
`_x_reason=get_started_workspaces`) that Parla does not use. Real responses of
the chooser variant carry per-team login links, so they are never kept as
fixtures.

Response keys: `ok`, `confirmed_email`, `current_orgs`, `current_teams[]`
(`email`, `teams[]`), `domain_enabled_teams`, `invited_teams`,
`has_valid_cookie`. Each team: `id`, `name`, `domain`, `url`
(`https://<domain>.slack.com/ssb/redirect`), `icon{image_*, image_default}`,
`sso_required`, `two_factor_required`, `is_logged_out`, `associated_user`.
Parla reads `id`, `name`, `domain`, `icon`, `sso_required`,
`two_factor_required`, `is_logged_out` (`slack_api::find_workspaces`, one
attempt, no retry); a team without an `id` is skipped, and ids are checked
before they go into a URL.

A workspace `url` (from `localConfig_v2`, a manual paste or the keychain) is used only when it is
`https`, has no credentials or port and its host is `slack.com` or `slack-gov.com` or a
subdomain of them; otherwise the entry is skipped (or the stored credentials are rejected), so
the token and cookie never go to another host.

## Session ready before capture

`webview/capture.rs` (`classify_url`, `run_capture_with`): as soon as the
cookie jar holds a `d` cookie on a sign-in page (`UrlKind::SessionPage`), a
ready client page (`UrlKind::ClientReady`), or a desktop handoff
(`UrlKind::DesktopHandoff`), `run_capture_with` returns
`CaptureState::SessionReady` with it so the provider can list the workspaces
first. A desktop handoff is still rewritten or shown as before so the flow can
continue if the listing fails. The workspace chooser is an in-page route; Parla
listens for route changes. Event driven, no polling.

`sign_in/mod.rs` (`run_window`): on `SessionReady` the provider calls
`signin.findWorkspaces` with that cookie. A non-empty list opens the native
picker at once (`sign_in/picker.rs`, `prepare_picker`): added rows are marked
`Updating...` and refreshed in the picker session in the background, one team
after another, with `capture_teams`. A failed lookup falls back to reading the
team on the current client page and saving it as before; if the page cannot be
read, or the lookup returns an empty list, the webview stays visible and the
capture loop keeps waiting.

## Signing in several workspaces in one webview

`webview/sequence.rs` (`capture_teams`): with the session cookie in the
webview, each chosen team id is loaded as `app.slack.com/client/<id>` and its
token is read from `localConfig_v2`. Driven by navigation events only (no
polling, no timers). A page that leaves the web client (SSO, two-factor,
signed out) yields `NeedsUser`: the UI shows the webview, then resumes
(`Start::Resume`) or skips the team.
A resumed capture that ends on the desktop-app handoff step (`/ssb/redirect` or
a `slack://` link, which is never loaded) loads the team's web client once, so
the capture finishes instead of waiting for a page that never comes.

The picker keeps the `Rc<LoginWebview>` alive and hidden with
`set_visible(false)` while captures run, shown again for a `NeedsUser` team,
and hidden after `Start::Resume`. Background refreshes skip `NeedsUser` teams
(the row becomes "Needs you" and can be added later) and report each outcome to
the picker so refreshed keys are reported on Add, Close and Cancel. Add queues
behind a running refresh because the URL handoff receiver and webview are
shared; nothing runs two captures at once. Each captured workspace is saved with
`complete_capture`, the same path as a single-workspace sign-in; the cookie is
held as a `Zeroizing` header and parsed again per save because
`SessionCookie` is not `Clone`. For a capture that already holds several
workspaces, the ones already in Parla are refreshed from that capture before the
picker opens for the new ones.

## Workspace name, icon and organisation (`team.info`)

Unverified: `team.info` is written from the public method documentation and
tested against synthetic fixtures (`crates/providers/slack/api/tests/fixtures/team/`),
not a capture. Tier 3, called once per workspace start (no polling), best
effort: a failure never blocks the workspace from starting.

- `team.id`, `team.name`: the workspace.
- `team.icon.image_132` (falling back to smaller sizes): the icon. When
  `image_default` is `true` the team has no custom icon and none is used.
- `team.enterprise_id`, `team.enterprise_name`: present when the workspace
  belongs to an Enterprise Grid organisation; the rail groups by them. The
  Grid shape is hand-written from the public docs, not captured.

The result is cached in `accounts.json` (`enterprise_name`, `icon_url`; older
files without them still load).

## Removing a workspace

Each workspace has its own keychain item holding its token and a copy of the
session cookie, so deleting that item removes the session for that workspace
only and leaves every other one usable. Removal is refused while the
workspace has a live session. It then deletes the local store file, WAL and
key, drops the row from `accounts.json`, and deletes the keychain item last.
Every step is safe to repeat (a missing keychain item counts as done), so a
failure part way is fixed by removing again. No API call is made (the server
session is not revoked). `accounts.json` is only changed through
`AccountIndex::update`, which serialises load, change and save.

## Chooser after the email code

After an email code (`signin.confirmCode`), Slack's chooser at
`app.slack.com/signin#/workspaces` lists the workspaces before any `d` cookie
exists: only the email-confirmation cookies (`ec`, `pec`) are set. The page
calls `signin.findWorkspaces` with `email` and `commingled_response=false`
(`_x_reason=get_started_workspaces`); each team carries a one-time
`magic_login_url`, and following it creates the `d` cookie for that team.
Parla's sign-in webview hooks the page's own `fetch` and `XMLHttpRequest` for
that answer (`webview/policy.rs`, `CHOOSER_HOOK_SCRIPT`), opens the native
picker from it, and opens each chosen team through its link. The links are
secrets: kept in memory only, never logged, each followed once.

## Native email-code sign-in

**Not verified: hypothesis only.** Written before a recon run
from the chooser notes above and
the public get-started page. Fixtures are synthetic
(`crates/providers/slack/api/tests/fixtures/signin-email-code/`, `synthetic:
true` in its README); the capture replaces this section and those files. Every
call is one attempt with no retry, on the bare `slack.com` host, with
`Origin: https://slack.com`, no token and no `d` cookie.

- **Send a code** (`slack_api::send_code`): `POST /api/signup.confirmEmail`,
  multipart form `email`, `locale`, `_x_reason=get_started_send_code`,
  `_x_mode=online`. A resend adds `resend=1` and uses
  `_x_reason=get_started_resend_code`. Errors assumed: `invalid_email`,
  `sso_required` (`SendCodeOutcome::NeedsSso`), `password_required`
  (`NeedsPassword`), `ratelimited` with `Retry-After`. No resend wait is known
  from the response; the client's default applies.
- **Confirm the code** (`slack_api::confirm_code`): `POST
  /api/signin.confirmCode`, form `email`, `code`,
  `_x_reason=get_started_confirm_code`, `_x_mode=online`. On `ok` the reply
  sets the email-confirmation cookies `ec` and `pec`; only those names, and
  only with no `Domain` or a `Domain` under `slack.com` or `slack-gov.com`, are
  kept (`EmailConfirmation`, a zeroized cookie header that never prints).
  Errors assumed: `invalid_code` (optional `attempts_left`, mapped to
  `ApiError::WrongCode`), `code_expired` (`CodeExpired`), `too_many_attempts`
  (`TooManyAttempts`).
- **Code shape**: assumed six digits shown as two groups of three (plan open
  question 2); the screens take the shape from the provider (`CodeSpec`).
- **Workspaces for the email** (`slack_api::find_workspaces_for_email`): the
  chooser variant of `signin.findWorkspaces` (see "Chooser after the email
  code"): form `email`, `commingled_response=false`,
  `_x_reason=get_started_workspaces`, `_x_mode=online`, with the `ec`/`pec`
  cookies. Rows are read like `find_workspaces`; `magic_login_url` becomes
  `login_link` only when it is `https` on a provider host, otherwise it is
  dropped and the row kept.
- **Login link** (`slack_auth::native::open_team`): `GET` the link with
  redirects off, at most 5 hops, each hop on a provider host
  (`is_workspace_url`). Assumed chain: `app.slack.com/t/<ws>/login/<code>`
  302 to `<ws>.slack.com/ssb/redirect`, setting `d` (and `d-s`). A hop whose
  path holds `sso`, `saml`, `two_factor`, `2fa` or `password`, or a final page
  without a `d` cookie, means the user is needed (`NativeError::NeedsUser`).
- **Boot page token** (`slack_auth::native::boot_token`): `GET
  <team>/ssb/redirect` with the `d` cookie; the token is the first
  `"api_token":"xoxc-...` in the body (probed earlier by `login_probe` command
  `b`). The body is never logged or kept.

Nothing here logs an email, code, cookie, link or token. `native_probe`
(`crates/providers/slack/auth/examples/`) runs the chain end to end and prints
only booleans, counts and lengths.
