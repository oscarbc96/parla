# Parla app shell catalog (English, source). Ids start with their area.
# unused-ok: action-app-quit action-app-open-settings action-app-open-settings-file action-app-open-keymap action-app-reload-config action-app-command-palette settings-quiet-mon-name settings-quiet-tue-name settings-quiet-wed-name settings-quiet-thu-name settings-quiet-fri-name settings-quiet-sat-name settings-quiet-sun-name
# unused-ok: action-theme-toggle-mode action-store-rebuild-cache action-mcp-rebuild-audit-log action-mcp-open-audit-log action-auth-sign-in-again action-auth-sign-out
# unused-ok: action-workspace-remove-workspace action-workspace-pause-notifications action-workspace-set-status action-app-delete-all-data action-app-show-crash-reports action-app-check-for-updates action-app-about action-workspace-pop-out-conversation action-workspace-pop-out-thread
# unused-ok: error-session-expired error-not-allowed error-rate-limited error-network error-detail error-unsupported error-too-large error-removal-running
# unused-ok: message-edit-failed message-delete-failed shell-unknown-provider shell-bootstrap-timeout file-error-no-download file-error-download-failed file-error-needs-connection file-error-refused huddle-open-in call-fallback-browser call-fallback-app call-native-fallback-browser call-native-fallback-app call-native-no-mic call-native-failed call-helper-crashed call-switch-title call-switch-leave call-switch-stay
# unused-ok: settings-language-title settings-language-desc settings-startup-title settings-startup-desc settings-dock-title settings-dock-desc settings-messages-title settings-messages-desc settings-updates-title settings-updates-desc settings-page-general settings-page-appearance settings-page-notifications settings-page-keymap settings-page-workspaces settings-page-privacy settings-page-calls settings-page-ai settings-page-plugins settings-page-mcp settings-page-advanced settings-theme-title settings-theme-desc settings-accent-title settings-accent-desc settings-composer-title settings-composer-desc settings-density-title settings-density-desc settings-sidebar-title settings-sidebar-desc settings-desktop-title settings-desktop-desc settings-keywords-title settings-keywords-desc settings-sound-title settings-sound-desc settings-quiet-title settings-quiet-desc settings-huddles-title settings-huddles-desc settings-quiet-mon settings-quiet-tue settings-quiet-wed settings-quiet-thu settings-quiet-fri settings-quiet-sat settings-quiet-sun
# unused-ok: action-call-toggle-mute action-call-leave action-call-show-window
# unused-ok: shortcuts-area-navigation shortcuts-area-messages shortcuts-area-composer shortcuts-area-calls shortcuts-area-app shortcuts-area-text
# unused-ok: call-window-title call-window-title-empty
# unused-ok: action-app-hide action-app-hide-others action-app-show-all action-app-minimize action-app-zoom action-app-close-window action-app-bring-all-to-front action-app-open-docs action-app-open-keyboard-shortcuts action-app-report-issue
# unused-ok: settings-accounts-list-title settings-accounts-list-desc settings-calls-ringing-title settings-calls-ringing-desc settings-calls-join-title settings-calls-join-desc settings-danger-title settings-danger-desc settings-mcp-server-title settings-mcp-server-desc settings-mcp-connect-title settings-mcp-connect-desc settings-mcp-workspaces-title settings-mcp-workspaces-desc settings-mcp-tools-desc settings-mcp-audit-title settings-mcp-audit-desc settings-mcp-ws-enabled settings-mcp-ws-writes settings-mcp-ws-confirm settings-memory-title settings-memory-desc settings-logs-title settings-logs-desc settings-store-title settings-store-desc settings-privacy-title settings-privacy-desc
# unused-ok: settings-calls-devices-title settings-calls-devices-desc call-native-no-speaker

## Actions (titles in the command palette and the keymap docs)
action-app-quit = Quit Parla
action-app-open-settings = Open settings
action-app-open-keymap = Open keymap
action-app-command-palette = Command palette
action-app-reload-config = Reload configuration
action-app-check-for-updates = Check for updates
action-app-about = About Parla
action-theme-toggle-mode = Toggle light and dark mode
action-store-rebuild-cache = Rebuild cache
action-mcp-rebuild-audit-log = Rebuild audit log
action-auth-sign-in-again = Sign in again
action-auth-sign-out = Sign out
action-workspace-remove-workspace = Remove this workspace from Parla
action-app-delete-all-data = Delete all Parla data

## Provider errors that carry no text of their own
error-session-expired = session expired
error-not-allowed = not allowed: { $detail }
error-rate-limited = rate limited
error-network = network error: { $detail }
error-detail = { $detail }
error-unsupported = not supported here
error-too-large = too large
error-removal-running = the workspace is still running

## Window
shell-no-accounts = No accounts yet. Sign in to get started.
shell-connecting = Connecting...
shell-still-connecting = Still connecting...
shell-select-conversation = Select a conversation
shell-retry = Retry
shell-start-failed = Could not start the workspace.
shell-load-failed = Could not load the workspace: { $reason }
shell-unknown-provider = This account belongs to an unknown provider ({ $provider }).
shell-bootstrap-timeout = the server did not answer in time
shell-cancel = Cancel
shell-remove-title = Remove { $name } from Parla?
shell-remove-detail = This signs you out and deletes this workspace's data from this device: cached messages, drafts and queued messages that were not sent are lost. Nothing is deleted on the server.
shell-remove-confirm = Remove workspace
shell-set-status = Set a status...
shell-set-away = Set yourself as away
shell-set-active = Set yourself as active
shell-delete-all-title = Delete all Parla data?
shell-delete-all-detail = This signs you out of every workspace and deletes everything Parla stores on this device: caches, drafts, queued messages, saved sign-ins and the local database. Your settings, keymap and themes are kept.
shell-delete-all-confirm = Delete all data
shell-delete-message-title = Delete message?
shell-delete-message-confirm = Delete
shell-removal-failed = Could not finish removing the data: { $reason }
menu-view-details = View details
menu-star = Star
menu-move-to = Move to { $section }
menu-move-to-channels = Move to Channels
menu-move-to-direct-messages = Move to Direct messages
menu-sidebar-settings = Sidebar settings…
menu-organise-sidebar = Organise sidebar
menu-unstar = Unstar
menu-mark-read = Mark as read
menu-copy-link = Copy link
menu-edit-channel = Edit channel…
menu-add-people = Add people…
menu-leave-channel = Leave channel
menu-archive-channel = Archive channel
shell-pop-out = Open in new window
shell-notify-settings = All notification settings…
shell-show-details = Show channel details
shell-hide-details = Hide channel details
shell-pop-out-title = { $conversation } · { $workspace }

## Banners
expiry-message = Your { $workspace } session expired.
expiry-detail = You can still read cached messages.
expiry-sign-in-again = Sign in again
offline-message = You are offline.
offline-detail = Messages you send are queued and go out when you reconnect. Parla keeps retrying.
offline-retry-now = Retry now
store-banner-unreadable = The local cache for this workspace cannot be opened. Messages will load again from { $provider }. Queued messages and drafts saved on this device cannot be recovered.
store-banner-rebuild = Rebuild cache
store-full-message = Disk nearly full
store-full-detail = The disk that holds Parla's local store has less than 1 GiB free.
store-full-manage = Manage storage
store-notice-unencrypted = Full-disk encryption is off: image caches are stored unencrypted on this device.
mcp-notice-socket-unsafe = MCP: the folder { $path } is not private to your user, so MCP is not listening. Make it yours with mode 0700 and restart Parla.
mcp-notice-audit = MCP: the audit log cannot be opened. Run "Rebuild audit log" to start a new one.
warning-keymap = keymap: { $detail }
warning-keymap-bad-keystrokes = keymap: bad keystrokes { $keystrokes }

## Files
file-error-no-download = This file cannot be downloaded
file-error-download-failed = Download failed: { $reason }
file-error-needs-connection = Uploads need a connection: go online and try again (uploads are not queued)
file-error-refused = { $reasons }

## Media viewer
media-viewer-meta = { $sender } in { $place } · { $time } · { $size }
media-viewer-meta-direct = { $sender } · { $time } · { $size }
media-viewer-dimensions = { $width } × { $height }
media-viewer-percent = { $value }%
media-viewer-zoom-in = Zoom in
media-viewer-zoom-out = Zoom out
media-viewer-actual-size = Actual size
media-viewer-fit = Fit to window
media-viewer-copy = Copy image
media-viewer-copied = Image copied
media-viewer-download = Download
media-viewer-open-external = Open in default app
media-viewer-close = Close viewer
media-viewer-today = Today { $time }
media-viewer-yesterday = Yesterday { $time }
media-viewer-open-in-channel = Open in channel
media-viewer-prev = Previous file
media-viewer-next = Next file
media-viewer-position = { $index } of { $count }
media-viewer-position-in = { $index } of { $count } in { $place }
media-viewer-loading = Loading image
media-viewer-failed = This image cannot be shown here. Open it in your default app instead.
media-viewer-pdf = PDF · { $size }
media-viewer-pdf-note = Parla does not draw PDF pages. Open the file in your default app to read it.

## About
about-version = Parla { $version }
about-licence = Licence: MPL-2.0
about-privacy = Privacy
about-terms = Terms
about-licences = Third-party licences
about-source = Source code
about-security = Report a security issue
about-name = Parla
about-tagline = A small, fast desktop chat client.
about-version-label = Version
about-licence-label = Licence
about-copy-version = Copy version info
about-copied = Copied
about-made-by = Made by Parla contributors. Docs CC-BY-4.0.

## Notifications
notification-action-reply = Reply
notification-action-mark-read = Mark as read
notification-title-channel = { $author } in #{ $channel }
notification-body-files =
    { $count ->
        [one] { $count } file
       *[other] { $count } files
    }
notification-reminder-title = Reminder
reminder-about-message = Follow up on this message
session-expired-title = Sign in to { $workspace } again
session-expired-body = Your session ended. Click to sign in.
notifications-permission-denied = Notifications are turned off for Parla in System Settings. Turn them on under Notifications > Parla.

## Scheduled messages
schedule-late-title =
    { $count ->
        [one] Send the overdue scheduled message?
       *[other] Send the { $count } overdue scheduled messages?
    }
schedule-late-detail =
    { $count ->
        [one] This message was scheduled for a time when Parla was not open.
       *[other] These messages were scheduled for a time when Parla was not open.
    }
schedule-late-keep = Keep scheduled for later
schedule-late-send = Send now
schedule-late-line = { $label }: { $text }
action-workspace-pause-notifications = Pause notifications
action-workspace-set-status = Set a status
action-workspace-pop-out-conversation = Open conversation in new window
action-workspace-pop-out-thread = Open thread in new window
message-edit-failed = Could not edit the message. It was left as it was.
message-delete-failed = Could not delete the message. It is still there.
huddle-open-in = Open this huddle in { $provider }
call-fallback-browser = Couldn't open the call window; opened in your browser
call-fallback-app = Couldn't open the call window; opened in the provider's app
call-native-fallback-browser = Couldn't join with native audio; opened in your browser
call-native-fallback-app = Couldn't join with native audio; opened in the provider's app
call-native-no-mic = The microphone is unavailable; you can listen but not speak
call-native-no-speaker = The speaker is unavailable; check your audio output
call-person-unknown = Someone
call-native-failed = The call ended unexpectedly
call-helper-crashed = The call window closed unexpectedly
call-switch-title = Leave the current huddle and join this one?
call-switch-leave = Leave and join
call-switch-stay = Stay
call-window-title = Huddle in { $name }
call-window-title-empty = Huddle
action-call-toggle-mute = Toggle microphone mute
action-call-leave = Leave the huddle
action-call-show-window = Show the call window

## Ringing
ring-title = { $caller } is huddling with you
ring-body = Incoming huddle
ring-join = Join
ring-decline = Decline
ring-missed-title = Missed huddle from { $caller }

## Menu-bar icon
tray-open = Open Parla
tray-quit = Quit Parla
tray-pause = Pause notifications
tray-resume = Resume notifications
tray-header =
    { $mentions ->
        [one] { $unread } unread, { $mentions } mention
       *[other] { $unread } unread, { $mentions } mentions
    }
tray-set-status = Set status…
tray-settings = Settings…
tray-workspace-mentions = { $name } ({ $count })
tray-workspace-expired = { $name } (sign in again)
tray-tooltip = Parla
tray-tooltip-mentions =
    { $count ->
        [one] Parla, { $count } mention
       *[other] Parla, { $count } mentions
    }
action-mcp-open-audit-log = Open the MCP audit log

## MCP status
mcp-status-off = MCP: off
mcp-status-on =
    { $count ->
        [0] MCP: on, no clients
        [one] MCP: on, { $count } client
       *[other] MCP: on, { $count } clients
    }

## Crash reports
crash-notice = Parla quit unexpectedly. Report saved locally.
crash-show = Show
crash-delete = Delete
action-app-show-crash-reports = Show crash reports

## Updates
update-available = Parla { $version } is available
update-download = Download
update-skip = Skip this version
update-up-to-date = Parla is up to date
update-check-failed = Could not check for updates
update-install = Update
update-retry = Try again
update-downloading = Downloading Parla { $version }… { $percent }%
update-installing = Installing Parla { $version }…
update-ready = Parla { $version } is ready. Restart to update.
update-restart = Restart to update
update-dismiss = Dismiss this notice
update-download-failed = Could not download Parla { $version }
update-signature-invalid = Parla { $version } did not pass the signature check and was not installed
update-install-failed = Could not install Parla { $version }

titlebar-back = Back
titlebar-forward = Forward
titlebar-search = Search { $workspace }
titlebar-search-plain = Search

## Settings window
settings-title = Settings
settings-open-file = Open settings.json
settings-page-general = General
settings-page-appearance = Appearance
settings-page-notifications = Notifications
settings-page-keymap = Keymap
settings-page-workspaces = Workspaces
settings-page-privacy = Privacy and storage
settings-page-calls = Calls
settings-page-ai = AI
settings-page-plugins = Plugins
settings-page-mcp = MCP
settings-page-advanced = Advanced
settings-footer-version = Parla { $version }
settings-footer-file = Settings file: { $path }
settings-placeholder-title = Not in this window yet
settings-placeholder-body = These settings are not in the Settings window yet. Edit them in settings.json; Parla applies a saved change at once.
settings-general-title = General
settings-general-subtitle = Language, startup and how Parla lives on your desktop.
settings-language-title = Language
settings-language-desc = The app follows your system language unless you pick one. Dates and numbers follow it too.
settings-language-label = Interface language
settings-language-system = System default
settings-startup-title = Startup
settings-startup-desc = Parla uses no CPU while idle, so it is safe to keep running.
settings-login-label = Open Parla at login
settings-login-help = Starts hidden in the menu bar
settings-restore-label = Restore windows from last session
settings-restore-help = Takes effect the next time Parla starts.
settings-dock-title = Dock and menu bar
settings-dock-desc = Where the Parla icon shows on macOS.
settings-dock-always = Dock and menu bar
settings-dock-always-desc = Always visible in both.
settings-dock-window = Dock with window
settings-dock-window-desc = Dock icon only while a window is open.
settings-dock-never = Menu bar only
settings-dock-never-desc = No Dock icon at all.
settings-messages-title = Messages
settings-messages-desc = How time and media appear in conversations.
settings-clock-label = Clock
settings-clock-24 = 24-hour · 16:42
settings-clock-12 = 12-hour · 4:42 PM
settings-images-label = Inline images
settings-images-always = Always
settings-images-click = On click
settings-images-never = Never
settings-updates-title = Updates
settings-updates-desc = Signed releases from the project, checked once a day.
settings-updates-auto = Check for updates automatically
settings-updates-status = You are on version { $version }.
settings-updates-checked = Checked at { $time }.
settings-updates-check-now = Check now
settings-error-write = Could not save settings.json: { $reason }
settings-error-not-object = settings.json is not a JSON object, so nothing was saved. Fix the file or delete it.
settings-appearance-title = Appearance
settings-appearance-subtitle = Make Parla yours. Changes apply instantly and are saved to settings.json.
settings-theme-title = Theme
settings-theme-desc = Follow the system or pin one. Each mode has its own theme.
settings-theme-system = System
settings-theme-light = Light
settings-theme-dark = Dark
settings-accent-title = Accent
settings-accent-desc = Buttons, focus and selection. Every preset passes AA in light and dark.
settings-accent-brand = Brand
settings-accent-ember = Ember
settings-accent-amber = Amber
settings-accent-coral = Coral
settings-accent-ink = Ink
settings-accent-high-contrast = High contrast is on, so the accent stays fixed.
settings-composer-title = Composer
settings-composer-desc = How the message box sends.
settings-send-key-row = Send key
settings-send-key-row-desc = Shift-Enter makes a new line when Enter sends; Enter does when ⌘⏎ sends.
settings-send-key-enter = Enter sends
settings-send-key-cmd-enter-mac = ⌘⏎ sends
settings-send-key-cmd-enter = Ctrl+⏎ sends
settings-density-title = Density
settings-density-desc = Row height in the sidebar and spacing of messages.
settings-density-compact = Compact
settings-density-cozy = Cozy
settings-sidebar-title = Sidebar
settings-sidebar-desc = What the conversation list shows.
settings-sidebar-unreads = Show Unreads section
settings-sidebar-sort = Sort conversations
settings-sidebar-sort-alpha = A to Z
settings-sidebar-sort-recent = Recent
settings-appearance-footer = Every choice writes settings.json. Themes are plain JSON files: share yours, or drop someone else's into the themes folder.
settings-notifications-title = Notifications
settings-notifications-subtitle = Decide what reaches you, how it sounds and when Parla stays quiet.
settings-desktop-title = Desktop notifications
settings-desktop-desc = Native notifications from your OS.
settings-desktop-all = All messages
settings-desktop-all-desc = Every new message
settings-desktop-mentions = Mentions and DMs
settings-desktop-mentions-desc = Recommended
settings-desktop-none = Nothing
settings-desktop-none-desc = Badges only
settings-preview-label = Show message previews
settings-preview-help = Off: a notification shows only who wrote and where.
settings-keywords-title = Keywords
settings-keywords-desc = Treat these words like a mention.
settings-keywords-add = Add a keyword...
settings-sound-title = Sound
settings-sound-desc = Parla's own sound. Volume follows the system.
settings-sound-label = Play a sound for notifications
settings-quiet-title = Quiet hours
settings-quiet-desc = Pause notifications on a schedule. Mentions still badge.
settings-quiet-from = From
settings-quiet-to = to
settings-quiet-mon = M
settings-quiet-mon-name = Monday
settings-quiet-tue = T
settings-quiet-tue-name = Tuesday
settings-quiet-wed = W
settings-quiet-wed-name = Wednesday
settings-quiet-thu = T
settings-quiet-thu-name = Thursday
settings-quiet-fri = F
settings-quiet-fri-name = Friday
settings-quiet-sat = S
settings-quiet-sat-name = Saturday
settings-quiet-sun = S
settings-quiet-sun-name = Sunday
settings-time-placeholder = HH:MM
settings-time-invalid = Use a 24-hour time such as 08:30
settings-huddles-title = Huddles
settings-huddles-desc = Incoming DM huddles.
settings-huddles-ring = Ring for incoming DM huddles
settings-unit-mb = MB
settings-unit-days = days
settings-unit-messages = msgs
settings-unit-minutes = min
settings-applies-on-restart = Applies the next time Parla starts.
settings-mcp-title = MCP
settings-mcp-subtitle = Let AI tools on this computer read and, if you allow it, act in your workspaces. Off by default.
settings-mcp-server-title = Local MCP server
settings-mcp-server-desc = Clients reach Parla only through it, on this computer.
settings-mcp-enabled = Run the MCP server
settings-mcp-connect-title = Connect a client
settings-mcp-connect-desc = How clients talk to Parla.
settings-mcp-config = Client config
settings-mcp-config-help = Copies a config that launches this copy of Parla.
settings-mcp-copy-config = Copy client config
settings-mcp-copied = Copied
settings-mcp-workspaces-title = Workspaces
settings-mcp-workspaces-desc = Clients see only the workspaces you share.
settings-mcp-no-workspaces = No workspaces signed in yet.
settings-mcp-ws-enabled = Share with MCP clients
settings-mcp-ws-writes = Allow write actions
settings-mcp-ws-confirm = Ask before each write
settings-mcp-tools-title = Tools · { $count }
settings-mcp-tools-desc = What a client can call. Write tools need Allow write actions.
settings-mcp-tool-read = READ
settings-mcp-tool-write = WRITE
settings-mcp-audit-title = Audit log
settings-mcp-audit-desc = Every tool call, kept on this computer.
settings-mcp-audit = Agent activity
settings-mcp-audit-help = Calls, decisions and results.
settings-mcp-audit-open = Open audit log
settings-calls-title = Calls and huddles
settings-calls-subtitle = Ringing and how you join.
settings-calls-ringing-title = Ringing
settings-calls-ringing-desc = When someone starts a huddle with you in a DM.
settings-calls-ring = Ring for incoming DM huddles
settings-calls-ringtone = Ringtone
settings-calls-ringtone-chime = Parla chime
settings-calls-ringtone-none = Silent (banner only)
settings-calls-join-title = Join with
settings-calls-join-desc = Calls from other services always open in their own app.
settings-calls-join-native = Parla audio
settings-calls-join-window = Parla window
settings-calls-join-browser = Browser
settings-calls-join-provider = Provider app
settings-calls-join-help = Parla audio joins without a window; if it cannot connect, Parla tries the window, then the browser, then the provider's app.
settings-calls-mute = Mute microphone when joining
settings-calls-devices-title = Audio devices
settings-calls-devices-desc = For Parla audio. Changes apply to the running call at once.
settings-calls-input = Microphone
settings-calls-output = Speaker
settings-calls-device-default = System default
settings-accounts-title = Workspaces and privacy
settings-accounts-subtitle = Your sessions and what Parla keeps on this computer.
settings-accounts-list-title = Workspaces
settings-accounts-list-desc = Each workspace keeps its own session.
settings-accounts-none = No workspaces signed in.
settings-accounts-connected = Connected
settings-accounts-expired = Session expired
settings-accounts-stopped = Not connected
settings-accounts-sign-in = Sign in again
settings-accounts-sign-out = Sign out
settings-accounts-sign-out-title = Sign out of { $name }?
settings-accounts-sign-out-detail = Parla deletes this workspace's saved sign-in. Sign in again to use it.
settings-store-title = Local store
settings-store-desc = Messages are cached in an encrypted database per workspace.
settings-store-cache = Cache limit
settings-store-retention = Keep messages for
settings-store-clear-label = Clear cache
settings-store-clear-help = Deletes the cached messages; each workspace loads again.
settings-privacy-title = Privacy
settings-privacy-desc = How Parla guards what it keeps on this device.
settings-lock-idle = Lock store when idle
settings-lock-idle-help = After this long without input, Parla closes the encrypted store, drops its keys from memory and covers its windows until you unlock. Applies at once.
settings-lock-off = Off
settings-lock-minutes = { $n } min
settings-lock-hour = 1 hour
settings-crash = Crash reports
settings-crash-help = A redacted report stays on this device and is never sent. Ask offers to show or delete it at the next start.
settings-crash-off = Off
settings-crash-ask = Ask every time
settings-crash-local = Keep locally
settings-store-clear = Clear cache
settings-store-clear-title = Clear the message cache?
settings-store-clear-detail = Every workspace deletes its cached messages and drafts and loads again from the server.
settings-danger-title = Danger zone
settings-danger-desc = These cannot be undone.
settings-danger-delete = Delete local data and sign out everywhere
settings-danger-delete-help = Removes the encrypted store and the saved sign-ins. Settings stay.
settings-danger-delete-button = Delete…
settings-danger-reset-label = Reset all settings
settings-danger-reset-help = Back to the defaults. Theme and keymap files stay on disk.
settings-danger-reset = Reset settings
settings-danger-reset-title = Reset all settings?
settings-danger-reset-detail = settings.json goes back to the commented defaults. Your themes and keymap.json are kept.
settings-advanced-title = Advanced
settings-advanced-subtitle = Memory, logs and recovery. The defaults keep Parla light; change them only if you need to.
settings-memory-title = Memory
settings-memory-desc = Upper bounds. Parla never grows past them.
settings-advanced-image-cache = Image cache
settings-advanced-image-cache-help = Applies the next time Parla starts.
settings-advanced-message-window = Messages kept in memory per conversation
settings-advanced-lean = Go lean after the window is hidden for
settings-step-cache-dec = Decrease cache limit
settings-step-cache-inc = Increase cache limit
settings-step-retention-dec = Keep messages for less time
settings-step-retention-inc = Keep messages for longer
settings-step-image-cache-dec = Decrease image cache
settings-step-image-cache-inc = Increase image cache
settings-step-message-window-dec = Keep fewer messages in memory
settings-step-message-window-inc = Keep more messages in memory
settings-step-lean-dec = Go lean sooner
settings-step-lean-inc = Go lean later
action-app-open-settings-file = Open settings.json

## Menu bar
action-app-hide = Hide Parla
action-app-hide-others = Hide others
action-app-show-all = Show all
action-app-minimize = Minimize
action-app-zoom = Zoom
action-app-close-window = Close window
action-app-bring-all-to-front = Bring all to front
action-app-open-docs = Parla documentation
action-app-open-keyboard-shortcuts = Keyboard shortcuts
action-app-report-issue = Report an issue
menu-settings = Settings…
menu-edit = Edit
menu-view = View
menu-window = Window
menu-help = Help
menu-undo = Undo
menu-redo = Redo
menu-cut = Cut
menu-copy = Copy
menu-paste = Paste
menu-select-all = Select all
shell-switch-to = Switch to { $workspace }
shell-dm-unsupported = { $person } cannot be messaged from here yet.
shell-dm-failed = Could not open a conversation with { $person }: { $reason }
shell-admin-failed = That did not work: { $reason }
shell-join-failed = Couldn't join the channel
shell-invite-title = Add people to #{ $channel }
shell-invite-person-failed = { $person }: { $reason }
shell-leave-title = Leave #{ $channel }?
shell-leave-body = You can join again from Browse channels.
shell-leave-body-private = It is private: you need an invitation to come back.
shell-leave-ok = Leave
shell-archive-title = Archive #{ $channel }?
shell-archive-body = Nobody can post in it any more. Its messages stay readable and searchable.
shell-archive-ok = Archive

## Keyboard shortcuts (the sheet and the Settings Keymap page)
shortcuts-area-navigation = Navigation
shortcuts-area-messages = Messages
shortcuts-area-composer = Composer
shortcuts-area-calls = Calls
shortcuts-area-app = App and windows
shortcuts-workspace-range = Switch to workspace 1–9
shortcuts-area-text = Text editing
settings-keymap-subtitle = Every action has a shortcut you can change. Your bindings live in keymap.json and layer over the defaults.
settings-keymap-open-file = Open keymap.json
settings-keymap-search = Search by action or keys
settings-keymap-count = { $shown } of { $total } actions
settings-keymap-filter-all = All
settings-keymap-filter-changed = Changed
settings-keymap-filter-conflicts = Conflicts · { $count }
settings-keymap-needs-modifier = Add a modifier such as Cmd, Ctrl or Alt: a plain key here would fire while you type.
settings-keymap-shadow = Also bound to { $action }. While { $context } has focus, its binding wins.
settings-keymap-col-action = Action
settings-keymap-col-shortcut = Shortcut
settings-keymap-col-context = Context
settings-keymap-col-source = Source
settings-keymap-context-any = Anywhere
settings-keymap-source-default = Default
settings-keymap-source-user = You
settings-keymap-unbound = Not set
settings-keymap-edit-label = Change the shortcut of { $action }
settings-keymap-press-keys = Press keys…
settings-keymap-save = Save
settings-keymap-cancel = Cancel
settings-keymap-reset = Reset
settings-keymap-conflict = Also bound to { $action } here. Saving replaces it.
settings-keymap-footer = Enter saves, Esc cancels; Tab cannot be recorded. A second key makes a sequence such as Cmd-K Cmd-S. Contexts nest: Composer bindings win over global ones while you type.
settings-keymap-error-not-array = keymap.json is not a JSON array, so nothing was saved. Fix the file or delete it.

settings-logs-title = Logs and diagnostics
settings-logs-desc = Tokens, cookies and message text are kept out of the log and the export.
settings-log-level = Log level
settings-log-level-help = How much Parla writes to its log. It applies at once.
settings-log-error = Error
settings-log-info = Info
settings-log-debug = Debug
settings-open-logs = Open logs folder
settings-open-logs-label = Logs
settings-open-logs-help = Shows parla.log in your file manager.
settings-export-diagnostics = Export…
settings-export-label = Redacted diagnostics for a bug report
settings-export-help = Saves a text file with the version, settings, recent log lines, counters and the newest crash report, and shows it. Read it before you share it.
settings-error-export = Could not export diagnostics: { $reason }
lock-title = Parla is locked
lock-detail = Parla closed the encrypted store after a while without use. Unlock to open it again with the key from your keychain.
lock-unlock = Unlock
lock-error = Could not read the encrypted store key from the keychain. Try Unlock again.

code-viewer-meta = { $sender } in { $place } · { $time }
code-viewer-meta-direct = { $sender } · { $time }
code-viewer-jump = Jump to message
code-viewer-wrap = Wrap
code-viewer-line-numbers = Line numbers
code-viewer-copy = Copy
code-viewer-copied = Copied
code-viewer-download = Download
code-viewer-find-placeholder = Find in file
code-viewer-find-count = { $index } of { $count }
code-viewer-find-count-capped = { $index } of { $count }+
code-viewer-find-none = No results
code-viewer-find-count-label = Match { $index } of { $count }
code-viewer-find-count-label-capped = Match { $index } of more than { $count }
code-viewer-region =
    { $count ->
        [one] Code, 1 line
       *[other] Code, { $count } lines
    }
code-viewer-find-prev = Previous match
code-viewer-find-next = Next match
code-viewer-find-close = Close find
code-viewer-loading = Loading the whole file
code-viewer-failed = The whole file could not be loaded; showing the preview.
code-viewer-failed-empty = This file could not be loaded. Download it instead.
code-viewer-too-large = This file is too large to show here. Download it instead.
code-viewer-not-text = This file is not text. Download it instead.
code-viewer-lines =
    { $count ->
        [one] 1 line
       *[other] { $count } lines
    }
