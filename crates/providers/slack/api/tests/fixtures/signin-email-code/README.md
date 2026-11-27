---
synthetic: true
---

# Email-code sign-in fixtures

**Synthetic, not verified.** Every file here is hand-written from the
hypothesis in `docs/providers/slack/web-client/auth.md` ("Native email-code
sign-in"), not from a capture. Method names, form fields, error strings, cookie
names and the login-link redirect chain are guesses until an owner recon run
replaces these files with redacted exchanges and removes the `synthetic` flag.

Values are placeholders: the email is `user1@example.test`, codes, cookies and
login codes are `REDACTED`, ids are synthetic (`T0000001`).

| File | Exchange |
|---|---|
| `0001` | send a code |
| `0002` | resend a code |
| `0003`, `0004` | the address needs SSO or a password |
| `0005` | invalid address |
| `0006` | rate limited, with `Retry-After` |
| `0007` to `0009` | wrong, expired and too many codes |
| `0010` | code confirmed, confirmation cookies set |
| `0011` | workspaces for the confirmed email, with login links |
| `0012` | following a login link (headers and status only) |
| `0013` | a team's boot page (headers and status only, body dropped) |
