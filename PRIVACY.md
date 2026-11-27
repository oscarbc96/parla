# Privacy

Not created by, affiliated with, or supported by Slack Technologies, LLC.

Parla is a desktop client. The project runs no servers for it and receives none of
your data.

## Outbound destinations

Parla makes network requests only for features you use:

- **Provider endpoints and file hosts.** Parla talks to the provider's own
  endpoints and file hosts with your session to sign you in, load and send
  messages, files and related data. The call window loads on the provider's
  domains when you join a call or huddle.
- **Update check.** Parla can check GitHub Releases for a newer version at most
  once a day and when you choose "Check for updates". You can turn automatic
  checks off with `updates.check = false`. The check is a `GET` request to
  `https://api.github.com/repos/oscarbc96/parla/releases/latest` with only the
  headers `Accept: application/vnd.github+json` and `User-Agent: Parla/<version>`,
  and no cookies. It sends no identifying data beyond what any HTTPS request to
  GitHub carries.
- **Update download.** Only when you click **Update** in the update notice
  (macOS), Parla downloads the new `.dmg` and its `.minisig` signature from
  `https://github.com/oscarbc96/parla/releases/download/`, which GitHub serves
  from its release asset host. The requests carry only `Accept` and
  `User-Agent: Parla/<version>` headers and no cookies, and the download is
  refused unless its signature matches the key built into Parla.
- **Optional features you enable.** Plugins or features that use the network
  (for example GIF providers) are off by default. When you enable one, data goes
  where that feature says, such as the provider you choose or a local agent you
  connect.

## Local data

- **On your device.** Messages, files, caches and settings live in a local store.
  Your session token and cookie live only in the OS keychain. Debug builds keep
  them unencrypted in `<data_dir>/development_credentials.json` instead.
- **Logs.** Logs stay on your device. The diagnostic log holds no message text.
  `log.include_content` adds message text to the MCP audit log only.
- **Crash reports.** If Parla quits unexpectedly it saves a redacted report on
  your device. Crash reports are local only and are never sent. The
  `crash_reports` setting turns them off, keeps them without asking, or (the
  default) asks at the next start whether to show or delete them.
- **Idle lock.** With `storage.lock_after_minutes` on, Parla closes its
  encrypted store and drops the store keys from memory after that long without
  input, and covers its windows until you unlock it.
- **No telemetry.** Parla collects no analytics or usage data and sends none
  anywhere.

## Local MCP server

The local MCP server is off by default and local only: it never listens on the
network. When you enable it, local agents can read the workspaces you enable and
send messages, reactions or a status only with your confirmation (unless you turn
that off). Its audit log stays on your device, encrypted with your other local
data, and holds message text only if you turn on `log.include_content`.

## Deletion

Removing a workspace deletes its local database, keychain entries and caches.
"Delete all data" removes the data and cache Parla stores on this device but
keeps your settings. Nothing is held by the project.

## Age and support

Parla is for users aged 16 and over. Support is best effort through GitHub issues.

Your session token is full access to your account. Keep it private, and read the
risk notes in the README.
