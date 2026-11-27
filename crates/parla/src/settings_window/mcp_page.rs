//! The MCP page of the Settings window (the SettingsMCP board): the local
//! server switch, Copy client config, each workspace's
//! policy (shared, writes allowed, ask before each write), the tool list and a
//! link to the audit log. The policy keys are read by the MCP host on every
//! reload, so a change applies at once.

use std::path::Path;

use components::controls::{ButtonKind, button, compact, on_activate_app};
use components::settings_controls::{settings_row, switch, tag};
use gpui::{Context, div, prelude::*};
use settings::Settings;
use themes::{ActiveTheme as _, metrics};

use super::{SettingChange, SettingsView, json_text, owned};

/// One of a workspace's MCP policy switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpWorkspaceField {
    Enabled,
    AllowWrites,
    ConfirmWrites,
}

impl McpWorkspaceField {
    fn key(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::AllowWrites => "allow_writes",
            Self::ConfirmWrites => "confirm_writes",
        }
    }
}

/// One edit an MCP page control makes.
#[derive(Clone, Debug, PartialEq)]
pub enum McpChange {
    Enabled(bool),
    /// A workspace's policy switch; the first field is its `<provider>/<id>`.
    Workspace(String, McpWorkspaceField, bool),
}

impl McpChange {
    pub fn key_path(&self) -> Vec<String> {
        match self {
            Self::Enabled(_) => owned(&["mcp", "enabled"]),
            Self::Workspace(key, field, _) => {
                vec![
                    "mcp".into(),
                    "workspaces".into(),
                    key.clone(),
                    field.key().into(),
                ]
            }
        }
    }

    pub fn raw_json(&self) -> String {
        match self {
            Self::Enabled(v) | Self::Workspace(_, _, v) => json_text(v),
        }
    }
}

impl SettingChange for McpChange {
    fn path(&self) -> Vec<String> {
        self.key_path()
    }
    fn raw_json(&self) -> String {
        McpChange::raw_json(self)
    }
}

/// The client config Copy client config puts on the clipboard: the common
/// `mcpServers` shape, launching this binary with `mcp`.
pub fn mcp_client_config(exe: &Path) -> String {
    let v = serde_json::json!({
        "mcpServers": {
            "parla": { "command": exe.display().to_string(), "args": ["mcp"] }
        }
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// The workspaces the page lists: every signed-in account, then any other
/// workspace the file already has a policy for. `(key, name)`.
fn workspace_rows(s: &Settings, cx: &gpui::App) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = super::accounts(cx)
        .into_iter()
        .map(|a| (a.key.to_string(), a.name.to_string()))
        .collect();
    for key in s.mcp.workspaces.keys() {
        if !rows.iter().any(|(k, _)| k == key) {
            rows.push((key.clone(), key.clone()));
        }
    }
    rows
}

impl SettingsView {
    pub(super) fn mcp_page(&mut self, l: &i18n::Localizer, cx: &mut Context<Self>) -> gpui::Div {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let subtle = tokens.text_subtle;
        let s = self.settings(cx);

        let enabled = self.wire(
            switch("settings-mcp-enabled", s.mcp.enabled, &colors),
            cx,
            |v, _, cx| {
                let on = !v.settings(cx).mcp.enabled;
                v.apply(McpChange::Enabled(on), cx)
            },
        );
        let status = crate::mcp_status::McpStatus::text(cx);
        let server_body = settings_row(
            t!(l, "settings-mcp-enabled").into_owned(),
            status.map(Into::into),
            enabled,
            &colors,
            subtle,
        );

        let copy_label = if self.copied {
            t!(l, "settings-mcp-copied").into_owned()
        } else {
            t!(l, "settings-mcp-copy-config").into_owned()
        };
        let copy = self.wire(
            compact(button(
                "settings-mcp-copy-config",
                copy_label,
                ButtonKind::Secondary,
                true,
                &colors,
            ))
            .debug_selector(|| "settings-mcp-copy-config".into()),
            cx,
            |v, _, cx| {
                let exe = std::env::current_exe().unwrap_or_else(|_| "parla".into());
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(mcp_client_config(&exe)));
                v.copied = true;
                cx.notify();
            },
        );
        let transport_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .child(settings_row(
                t!(l, "settings-mcp-config").into_owned(),
                Some(t!(l, "settings-mcp-config-help").into_owned().into()),
                copy,
                &colors,
                subtle,
            ));

        let rows = workspace_rows(&s, cx);
        let mut workspaces_body = div().flex().flex_col().gap(metrics::SPACE_4);
        if rows.is_empty() {
            workspaces_body = workspaces_body.child(
                div()
                    .text_size(metrics::FONT_SIZE_SM)
                    .text_color(colors.text_muted)
                    .child(t!(l, "settings-mcp-no-workspaces").into_owned()),
            );
        }
        for (i, (key, name)) in rows.into_iter().enumerate() {
            let policy = s.mcp.workspaces.get(&key).cloned().unwrap_or_default();
            let mut row = div()
                .flex()
                .flex_col()
                .gap(metrics::SPACE_2)
                .p(metrics::SPACE_5)
                .rounded(metrics::RADIUS_MD)
                .border_1()
                .border_color(colors.border)
                .bg(colors.surface_background)
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_SM)
                        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                        .child(name),
                );
            for (suffix, label, field, on) in [
                (
                    "enabled",
                    "settings-mcp-ws-enabled",
                    McpWorkspaceField::Enabled,
                    policy.enabled,
                ),
                (
                    "writes",
                    "settings-mcp-ws-writes",
                    McpWorkspaceField::AllowWrites,
                    policy.allow_writes,
                ),
                (
                    "confirm",
                    "settings-mcp-ws-confirm",
                    McpWorkspaceField::ConfirmWrites,
                    policy.confirm_writes,
                ),
            ] {
                let key = key.clone();
                let el = self.wire(
                    switch(format!("settings-mcp-ws-{i}-{suffix}"), on, &colors),
                    cx,
                    move |v, _, cx| {
                        let now = v
                            .settings(cx)
                            .mcp
                            .workspaces
                            .get(&key)
                            .cloned()
                            .unwrap_or_default();
                        let current = match field {
                            McpWorkspaceField::Enabled => now.enabled,
                            McpWorkspaceField::AllowWrites => now.allow_writes,
                            McpWorkspaceField::ConfirmWrites => now.confirm_writes,
                        };
                        v.apply(McpChange::Workspace(key.clone(), field, !current), cx)
                    },
                );
                row = row.child(settings_row(
                    l.format("parla", label, None).into_owned(),
                    None,
                    el,
                    &colors,
                    subtle,
                ));
            }
            workspaces_body = workspaces_body.child(row);
        }

        let kinds = mcp::server::tool_kinds();
        let mut tools_body = div().flex().flex_col();
        for (name, write) in kinds {
            let (label, soft, text) = if *write {
                (
                    t!(l, "settings-mcp-tool-write").into_owned(),
                    tokens.signal_soft,
                    tokens.signal_text,
                )
            } else {
                (
                    t!(l, "settings-mcp-tool-read").into_owned(),
                    colors.accent_soft,
                    tokens.accent_soft_text,
                )
            };
            let selector = format!("settings-mcp-tool-{name}");
            tools_body = tools_body.child(
                div()
                    .debug_selector(move || selector.clone())
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_4)
                    .h(metrics::LAYOUT_ROW_COMPACT)
                    .child(
                        div()
                            .flex_1()
                            .font_family("JetBrains Mono")
                            .text_size(metrics::FONT_SIZE_XS)
                            .child(name.clone()),
                    )
                    .child(tag(label, soft, text)),
            );
        }

        let audit = {
            let el = compact(button(
                "settings-mcp-audit",
                t!(l, "settings-mcp-audit-open").into_owned(),
                ButtonKind::Secondary,
                true,
                &colors,
            ))
            .debug_selector(|| "settings-mcp-audit".into());
            on_activate_app(el, |window, cx| {
                window.dispatch_action(Box::new(crate::actions::OpenAuditLog), cx);
            })
        };
        let audit_body = settings_row(
            t!(l, "settings-mcp-audit").into_owned(),
            Some(t!(l, "settings-mcp-audit-help").into_owned().into()),
            audit,
            &colors,
            subtle,
        );

        let tools_title = t!(l, "settings-mcp-tools-title", count = kinds.len()).into_owned();
        div()
            .flex()
            .flex_col()
            .child(Self::page_header(
                t!(l, "settings-mcp-title").into_owned(),
                Some(t!(l, "settings-mcp-subtitle").into_owned()),
                &colors,
            ))
            .child(div().h(metrics::SPACE_4))
            .child(Self::section(
                l,
                "settings-mcp-server-title",
                "settings-mcp-server-desc",
                server_body,
                true,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-mcp-connect-title",
                "settings-mcp-connect-desc",
                transport_body,
                false,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-mcp-workspaces-title",
                "settings-mcp-workspaces-desc",
                workspaces_body,
                false,
                cx,
            ))
            .child(Self::section_titled(
                l,
                tools_title,
                "settings-mcp-tools-desc",
                tools_body,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-mcp-audit-title",
                "settings-mcp-audit-desc",
                audit_body,
                false,
                cx,
            ))
            .pb(metrics::SPACE_8)
    }
}
