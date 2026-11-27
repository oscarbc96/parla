# Slack provider catalog (English, source). Provider text only: errors and the
# unofficial notice.
# unused-ok: unofficial-notice error-foreign-host error-upload-not-visible
unofficial-notice = Not created by, affiliated with, or supported by Slack Technologies, LLC.
error-foreign-host = This address is not on a file host of the workspace, so it was not contacted.
error-upload-not-visible = The upload finished but the message is not visible yet.

# unused-ok: signin-notice-unofficial signin-notice-session signin-notice-token signin-notice-employer
# unused-ok: signin-notice-breakage signin-notice-privacy signin-notice-terms signin-notice-age signin-option-window signin-option-paste
# unused-ok: signin-field-cookie signin-field-console signin-help-paste signin-error-refused
# unused-ok: signin-error-no-cookie signin-error-no-teams signin-error-closed signin-error-interrupted
# unused-ok: signin-error-unknown-option signin-error-window-open signin-error-window-unavailable signin-error-accounts-update
# unused-ok: signin-error-config signin-error-expired signin-error-failed
# Sign-in screen (ids are built at run time, so the check cannot see their use)
signin-notice-unofficial = Not created by, affiliated with, or supported by Slack Technologies, LLC. Parla is an unofficial client.
signin-notice-session = Parla signs in with your own Slack web session.
signin-notice-token = The session token gives full access to your account.
signin-notice-employer = Extracting it may breach your employer's IT policy.
signin-notice-breakage = Slack may change its web client and break sign-in.
signin-notice-privacy = Privacy: https://oscarbc96.github.io/parla/docs/privacy/
signin-notice-terms = Terms: https://oscarbc96.github.io/parla/docs/terms/
signin-notice-age = You must be 16 or older to use Parla.
signin-option-window = Sign in with browser window
signin-option-paste = Paste token and cookie
signin-field-cookie = Cookie (the d cookie value, starts with xoxd-)
signin-field-console = Console output
signin-help-paste = Sign in at the workspace in your browser, open the developer console, run the snippet below and paste its output. See docs/sign-in for the steps (credit: emacs-slack).
signin-error-refused = The sign-in window was refused by the identity provider. Paste your token and cookie instead.
signin-error-no-cookie = No session cookie was found after sign-in.
signin-error-no-teams = No workspace was found in the signed-in session.
signin-error-closed = The sign-in window was closed.
signin-error-interrupted = Sign-in was interrupted.
signin-error-unknown-option = Unknown sign-in option.
signin-error-window-open = The sign-in window could not be opened.
signin-error-window-unavailable = The sign-in window is not available on this platform yet.
signin-error-accounts-update = The account list could not be updated.
signin-error-config = The local configuration could not be read.
signin-error-expired = The session has expired.
signin-error-failed = Sign-in failed: { $detail }

# Channel admin answers (2026-11-24)
# unused-ok: error-name-taken error-name-invalid error-general-channel error-archived error-admin-not-allowed error-already-in-channel error-not-in-channel error-cant-invite-self error-user-restricted error-topic-too-long error-purpose-too-long
error-name-taken = That name is already taken by a channel, a person or a user group.
error-name-invalid = That name cannot be used. Use lowercase letters, numbers, hyphens and underscores.
error-general-channel = The workspace's general channel cannot be left or archived.
error-archived = The channel is archived.
error-admin-not-allowed = You are not allowed to do that in this channel.
error-already-in-channel = Already in the channel.
error-not-in-channel = You are not in this channel.
error-cant-invite-self = You cannot invite yourself.
error-user-restricted = This person is a guest and cannot be added to this channel.
error-topic-too-long = The topic is too long.
error-purpose-too-long = The description is too long.
