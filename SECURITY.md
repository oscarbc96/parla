# Security policy

## Reporting a vulnerability

Please report security issues privately, not in public issues. Use GitHub's
private vulnerability reporting for this repository:
https://github.com/oscarbc96/parla/security/advisories/new

Include what you found, the steps to reproduce, and the affected version. You
should get a first reply within a week. Please give us reasonable time to fix the
issue before you disclose it.

## Scope

Parla handles a full Slack web session (token and cookie). Issues that expose
that session, the local encrypted store, the local MCP server, or the plugin
sandbox are the most important to us.

## Slack data

If a flaw could expose Slack data or customer content, we will also notify
`feedback@slack.com` after a fix is available.

## Supported versions

Only the latest release receives security fixes.
