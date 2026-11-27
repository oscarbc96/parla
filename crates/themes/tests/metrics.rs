//! The Rust metric constants must equal `docs/design/parla.tokens.json`, and every
//! token in those groups must have a constant (a new token fails until added).

use std::collections::BTreeSet;
use std::time::Duration;

use gpui::{FontWeight, Pixels, px};
use serde_json::Value;
use themes::metrics::*;

fn tokens() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/design/parla.tokens.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("tokens file")).expect("tokens json")
}

fn group_keys(root: &Value, path: &[&str]) -> BTreeSet<String> {
    let mut v = root;
    for p in path {
        v = &v[*p];
    }
    v.as_object()
        .unwrap_or_else(|| panic!("{path:?} is not an object"))
        .keys()
        .filter(|k| !k.starts_with('$'))
        .cloned()
        .collect()
}

fn raw(root: &Value, path: &[&str], key: &str) -> Value {
    let mut v = root;
    for p in path {
        v = &v[*p];
    }
    v[key]["$value"].clone()
}

fn px_of(v: &Value) -> f32 {
    v.as_str()
        .and_then(|s| s.strip_suffix("px"))
        .unwrap_or_else(|| panic!("not a px value: {v}"))
        .parse()
        .unwrap()
}

fn check_px(group: &[&str], table: &[(&str, Pixels)]) {
    let root = tokens();
    let have: BTreeSet<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    assert_eq!(
        group_keys(&root, group),
        have,
        "{group:?}: tokens and constants differ"
    );
    for (key, value) in table {
        assert_eq!(
            value.as_f32(),
            px_of(&raw(&root, group, key)),
            "{group:?}.{key}"
        );
    }
}

#[test]
fn space_matches_tokens() {
    check_px(
        &["space"],
        &[
            ("0", SPACE_0),
            ("1", SPACE_1),
            ("2", SPACE_2),
            ("3", SPACE_3),
            ("4", SPACE_4),
            ("5", SPACE_5),
            ("6", SPACE_6),
            ("7", SPACE_7),
            ("8", SPACE_8),
            ("9", SPACE_9),
            ("10", SPACE_10),
            ("11", SPACE_11),
            ("12", SPACE_12),
        ],
    );
}

#[test]
fn radius_matches_tokens() {
    check_px(
        &["radius"],
        &[
            ("xs", RADIUS_XS),
            ("sm", RADIUS_SM),
            ("md", RADIUS_MD),
            ("lg", RADIUS_LG),
            ("xl", RADIUS_XL),
            ("2xl", RADIUS_2XL),
            ("full", RADIUS_FULL),
            ("avatar", RADIUS_AVATAR),
        ],
    );
}

#[test]
fn font_size_matches_tokens() {
    check_px(
        &["font", "size"],
        &[
            ("3xs", FONT_SIZE_3XS),
            ("2xs", FONT_SIZE_2XS),
            ("xs", FONT_SIZE_XS),
            ("sm", FONT_SIZE_SM),
            ("md", FONT_SIZE_MD),
            ("lg", FONT_SIZE_LG),
            ("xl", FONT_SIZE_XL),
            ("2xl", FONT_SIZE_2XL),
            ("3xl", FONT_SIZE_3XL),
            ("4xl", FONT_SIZE_4XL),
            ("5xl", FONT_SIZE_5XL),
        ],
    );
}

#[test]
fn icon_size_matches_tokens() {
    check_px(
        &["icon-size"],
        &[
            ("sm", ICON_SIZE_SM),
            ("base", ICON_SIZE_BASE),
            ("md", ICON_SIZE_MD),
            ("lg", ICON_SIZE_LG),
            ("xl", ICON_SIZE_XL),
        ],
    );
}

#[test]
fn layout_matches_tokens() {
    check_px(
        &["layout"],
        &[
            ("titlebar", LAYOUT_TITLEBAR),
            ("rail", LAYOUT_RAIL),
            ("sidebar", LAYOUT_SIDEBAR),
            ("detail-panel", LAYOUT_DETAIL_PANEL),
            ("lock-text", LAYOUT_LOCK_TEXT),
            ("composer-min", LAYOUT_COMPOSER_MIN),
            ("row-compact", LAYOUT_ROW_COMPACT),
            ("row-cozy", LAYOUT_ROW_COZY),
            ("code-head", LAYOUT_CODE_HEAD),
            ("code-max-w", LAYOUT_CODE_MAX_W),
            ("code-gutter", LAYOUT_CODE_GUTTER),
            ("code-fade", LAYOUT_CODE_FADE),
            ("paste-preview-h", LAYOUT_PASTE_PREVIEW_H),
            ("titlebar-button", LAYOUT_TITLEBAR_BUTTON),
            ("titlebar-search", LAYOUT_TITLEBAR_SEARCH),
            ("titlebar-lights", LAYOUT_TITLEBAR_LIGHTS),
            ("msg-pad-cozy", LAYOUT_MSG_PAD_COZY),
            ("msg-avatar", LAYOUT_MSG_AVATAR),
            ("sidebar-header", LAYOUT_SIDEBAR_HEADER),
            ("sidebar-search", LAYOUT_SIDEBAR_SEARCH),
            ("rail-tile", LAYOUT_RAIL_TILE),
            ("rail-control", LAYOUT_RAIL_CONTROL),
            ("logo-mark", LAYOUT_LOGO_MARK),
            ("logo-about", LAYOUT_LOGO_ABOUT),
            ("header", LAYOUT_HEADER),
            ("mac-top-inset", LAYOUT_MAC_TOP_INSET),
            ("menu-row", LAYOUT_MENU_ROW),
            ("avatar-sm", LAYOUT_AVATAR_SM),
            ("avatar-xs", LAYOUT_AVATAR_XS),
            ("timeline-min", LAYOUT_TIMELINE_MIN),
            ("context-pane-min", LAYOUT_CONTEXT_PANE_MIN),
            ("avatar-lg", LAYOUT_AVATAR_LG),
            ("avatar-card", LAYOUT_AVATAR_CARD),
            ("people-panel", LAYOUT_PEOPLE_PANEL),
            ("preset-row", LAYOUT_PRESET_ROW),
            ("icon-button-sm", LAYOUT_ICON_BUTTON_SM),
            ("command-name", LAYOUT_COMMAND_NAME),
            ("indicator-dot", LAYOUT_INDICATOR_DOT),
            ("workspace-badge", LAYOUT_WORKSPACE_BADGE),
            ("accent-swatch", LAYOUT_ACCENT_SWATCH),
            ("sign-in-top", LAYOUT_SIGN_IN_TOP),
            ("dialog-top", LAYOUT_DIALOG_TOP),
            ("media-width", LAYOUT_MEDIA_WIDTH),
            ("media-height", LAYOUT_MEDIA_HEIGHT),
            ("clip-volume", LAYOUT_CLIP_VOLUME),
            ("caret", LAYOUT_CARET),
            ("field-xs", LAYOUT_FIELD_XS),
            ("field-sm", LAYOUT_FIELD_SM),
            ("field-md", LAYOUT_FIELD_MD),
            ("input-min", LAYOUT_INPUT_MIN),
            ("text-area-min", LAYOUT_TEXT_AREA_MIN),
            ("header-info-max", LAYOUT_HEADER_INFO_MAX),
            ("tab-height", LAYOUT_TAB_HEIGHT),
            ("header-total", LAYOUT_HEADER_TOTAL),
            ("details-tab-height", LAYOUT_DETAILS_TAB_HEIGHT),
            ("member-row", LAYOUT_MEMBER_ROW),
            ("setting-row", LAYOUT_SETTING_ROW),
            ("popover-sm", LAYOUT_POPOVER_SM),
            ("popover-lg", LAYOUT_POPOVER_LG),
            ("toast-width", LAYOUT_TOAST_WIDTH),
            ("popover-max-height", LAYOUT_POPOVER_MAX_HEIGHT),
            ("dialog-sm", LAYOUT_DIALOG_SM),
            ("call-card-max", LAYOUT_CALL_CARD_MAX),
            ("dialog-md", LAYOUT_DIALOG_MD),
            ("sign-in-width", LAYOUT_SIGN_IN_WIDTH),
            ("sign-in-content-max", LAYOUT_SIGN_IN_CONTENT_MAX),
            ("sign-in-height", LAYOUT_SIGN_IN_HEIGHT),
            ("window-width", LAYOUT_WINDOW_WIDTH),
            ("window-height", LAYOUT_WINDOW_HEIGHT),
            ("window-popout-height", LAYOUT_WINDOW_POPOUT_HEIGHT),
            ("window-popout-width", LAYOUT_WINDOW_POPOUT_WIDTH),
            ("window-about-width", LAYOUT_WINDOW_ABOUT_WIDTH),
            ("window-about-height", LAYOUT_WINDOW_ABOUT_HEIGHT),
            ("window-media-width", LAYOUT_WINDOW_MEDIA_WIDTH),
            ("window-media-height", LAYOUT_WINDOW_MEDIA_HEIGHT),
            ("window-media-min-width", LAYOUT_WINDOW_MEDIA_MIN_WIDTH),
            ("window-media-min-height", LAYOUT_WINDOW_MEDIA_MIN_HEIGHT),
            ("media-viewer-header", LAYOUT_MEDIA_VIEWER_HEADER),
            ("media-viewer-footer", LAYOUT_MEDIA_VIEWER_FOOTER),
            ("media-viewer-thumb-width", LAYOUT_MEDIA_VIEWER_THUMB_WIDTH),
            (
                "media-viewer-thumb-height",
                LAYOUT_MEDIA_VIEWER_THUMB_HEIGHT,
            ),
            ("media-viewer-nav", LAYOUT_MEDIA_VIEWER_NAV),
            ("media-viewer-zoom-label", LAYOUT_MEDIA_VIEWER_ZOOM_LABEL),
            ("media-viewer-meta", LAYOUT_MEDIA_VIEWER_META),
            ("media-viewer-card", LAYOUT_MEDIA_VIEWER_CARD),
            ("code-viewer-toolbar", LAYOUT_CODE_VIEWER_TOOLBAR),
            ("code-viewer-line", LAYOUT_CODE_VIEWER_LINE),
            ("code-viewer-gutter", LAYOUT_CODE_VIEWER_GUTTER),
            ("code-viewer-status", LAYOUT_CODE_VIEWER_STATUS),
            ("code-viewer-find-input", LAYOUT_CODE_VIEWER_FIND_INPUT),
            ("code-viewer-find-count", LAYOUT_CODE_VIEWER_FIND_COUNT),
            ("window-audit-width", LAYOUT_WINDOW_AUDIT_WIDTH),
            ("window-audit-height", LAYOUT_WINDOW_AUDIT_HEIGHT),
            ("window-confirm-min-width", LAYOUT_WINDOW_CONFIRM_MIN_WIDTH),
            (
                "window-confirm-min-height",
                LAYOUT_WINDOW_CONFIRM_MIN_HEIGHT,
            ),
            ("window-confirm-height", LAYOUT_WINDOW_CONFIRM_HEIGHT),
            ("confirm-label", LAYOUT_CONFIRM_LABEL),
            ("signature-bar", LAYOUT_SIGNATURE_BAR),
            ("load-state-height", LAYOUT_LOAD_STATE_HEIGHT),
            ("stepper-segment", LAYOUT_STEPPER_SEGMENT),
            ("stepper-segment-active", LAYOUT_STEPPER_SEGMENT_ACTIVE),
            ("code-cell-width", LAYOUT_CODE_CELL_W),
            ("code-cell-height", LAYOUT_CODE_CELL_H),
            ("code-dash-width", LAYOUT_CODE_DASH_W),
            ("code-dash-height", LAYOUT_CODE_DASH_H),
            ("provider-card", LAYOUT_PROVIDER_CARD),
            ("first-sync-width", LAYOUT_FIRST_SYNC_WIDTH),
            ("sync-step-icon", LAYOUT_SYNC_STEP_ICON),
            ("coach-width", LAYOUT_COACH_WIDTH),
            ("coach-height", LAYOUT_COACH_HEIGHT),
            ("switcher-width", LAYOUT_SWITCHER_WIDTH),
            ("sheet-width", LAYOUT_SHEET_WIDTH),
            ("sheet-search", LAYOUT_SHEET_SEARCH),
            ("switcher-top", LAYOUT_SWITCHER_TOP),
            ("switcher-input-height", LAYOUT_SWITCHER_INPUT_HEIGHT),
            ("popout-header", LAYOUT_POPOUT_HEADER),
            ("column-wide", LAYOUT_COLUMN_WIDE),
            ("center-min", LAYOUT_CENTER_MIN),
            ("window-min-width", LAYOUT_WINDOW_MIN_WIDTH),
            ("window-min-height", LAYOUT_WINDOW_MIN_HEIGHT),
            ("sign-in-min-width", LAYOUT_SIGN_IN_MIN_WIDTH),
            ("sign-in-min-height", LAYOUT_SIGN_IN_MIN_HEIGHT),
            ("window-about-min-width", LAYOUT_WINDOW_ABOUT_MIN_WIDTH),
            ("window-about-min-height", LAYOUT_WINDOW_ABOUT_MIN_HEIGHT),
            ("window-popout-min-width", LAYOUT_WINDOW_POPOUT_MIN_WIDTH),
            ("window-popout-min-height", LAYOUT_WINDOW_POPOUT_MIN_HEIGHT),
            ("window-audit-min-width", LAYOUT_WINDOW_AUDIT_MIN_WIDTH),
            ("window-audit-min-height", LAYOUT_WINDOW_AUDIT_MIN_HEIGHT),
            ("window-settings-width", LAYOUT_WINDOW_SETTINGS_WIDTH),
            ("window-settings-height", LAYOUT_WINDOW_SETTINGS_HEIGHT),
            (
                "window-settings-min-width",
                LAYOUT_WINDOW_SETTINGS_MIN_WIDTH,
            ),
            (
                "window-settings-min-height",
                LAYOUT_WINDOW_SETTINGS_MIN_HEIGHT,
            ),
            ("settings-header", LAYOUT_SETTINGS_HEADER),
            ("settings-nav", LAYOUT_SETTINGS_NAV),
            ("settings-label", LAYOUT_SETTINGS_LABEL),
            ("settings-content-max", LAYOUT_SETTINGS_CONTENT_MAX),
            ("switch-width", LAYOUT_SWITCH_WIDTH),
            ("day-tile", LAYOUT_DAY_TILE),
            ("stepper-value", LAYOUT_STEPPER_VALUE),
            ("status-pill", LAYOUT_STATUS_PILL),
            ("theme-card-width", LAYOUT_THEME_CARD_WIDTH),
            ("theme-preview-height", LAYOUT_THEME_PREVIEW_HEIGHT),
            ("theme-preview-sidebar", LAYOUT_THEME_PREVIEW_SIDEBAR),
            ("density-card-width", LAYOUT_DENSITY_CARD_WIDTH),
            ("time-field-width", LAYOUT_TIME_FIELD_WIDTH),
        ],
    );
}

#[test]
fn font_weight_matches_tokens() {
    let root = tokens();
    let table: &[(&str, FontWeight)] = &[
        ("regular", FONT_WEIGHT_REGULAR),
        ("medium", FONT_WEIGHT_MEDIUM),
        ("semibold", FONT_WEIGHT_SEMIBOLD),
        ("bold", FONT_WEIGHT_BOLD),
    ];
    let have: BTreeSet<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    assert_eq!(group_keys(&root, &["font", "weight"]), have);
    for (key, w) in table {
        let v = raw(&root, &["font", "weight"], key).as_f64().unwrap() as f32;
        assert_eq!(w.0, v, "font.weight.{key}");
    }
}

#[test]
fn durations_match_tokens() {
    let root = tokens();
    let table: &[(&str, Duration)] = &[
        ("fast", DURATION_FAST),
        ("base", DURATION_BASE),
        ("slow", DURATION_SLOW),
    ];
    let path = ["motion", "duration"];
    let have: BTreeSet<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    assert_eq!(group_keys(&root, &path), have);
    for (key, d) in table {
        let v = raw(&root, &path, key);
        let ms: u64 = v
            .as_str()
            .unwrap()
            .strip_suffix("ms")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(*d, Duration::from_millis(ms), "motion.duration.{key}");
    }
}

#[test]
fn px_const_context_works() {
    assert_eq!(SPACE_4, px(8.0));
}

#[test]
fn header_total_is_the_strip_plus_the_tab_row() {
    assert_eq!(LAYOUT_HEADER_TOTAL, LAYOUT_HEADER + LAYOUT_TAB_HEIGHT);
}
