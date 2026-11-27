use slack_api::{CredentialError, SessionCookie, XoxcToken};

#[test]
fn token_must_be_xoxc() {
    assert_eq!(
        XoxcToken::parse("xoxb-1").unwrap_err(),
        CredentialError::NotXoxc
    );
    assert_eq!(XoxcToken::parse("  ").unwrap_err(), CredentialError::Empty);
    assert_eq!(
        XoxcToken::parse("xoxc-1 2").unwrap_err(),
        CredentialError::NotXoxc
    );
    let t = XoxcToken::parse(" xoxc-1-2-3 \n").unwrap();
    assert_eq!(t.expose(), "xoxc-1-2-3");
}

#[test]
fn cookie_accepts_three_forms() {
    let a = SessionCookie::parse("xoxd-a%2Fb").unwrap();
    let b = SessionCookie::parse("d=xoxd-a%2Fb").unwrap();
    let c = SessionCookie::parse("Cookie: x=1; d=xoxd-a%2Fb; y=2").unwrap();
    assert_eq!(a.header_value(), "d=xoxd-a%2Fb");
    assert_eq!(a.header_value(), b.header_value());
    assert_eq!(a.header_value(), c.header_value());
}

#[test]
fn cookie_encodes_decoded_input() {
    let c = SessionCookie::parse("xoxd-a/b+c=").unwrap();
    assert_eq!(c.header_value(), "d=xoxd-a%2Fb%2Bc%3D");
}

#[test]
fn cookie_keeps_encoded_input() {
    let c = SessionCookie::parse("xoxd-a%2Fb").unwrap();
    assert_eq!(c.header_value(), "d=xoxd-a%2Fb");
}

#[test]
fn cookie_keeps_d_s() {
    let c = SessionCookie::parse("d=xoxd-a; d-s=1700000000").unwrap();
    assert_eq!(c.header_value(), "d=xoxd-a; d-s=1700000000");
}

#[test]
fn cookie_rejects_bad_input() {
    assert_eq!(
        SessionCookie::parse("").unwrap_err(),
        CredentialError::Empty
    );
    assert_eq!(
        SessionCookie::parse("d=abc").unwrap_err(),
        CredentialError::NotXoxd
    );
    assert_eq!(
        SessionCookie::parse("x=1").unwrap_err(),
        CredentialError::NotXoxd
    );
}

#[test]
fn secret_debug_is_redacted() {
    let t = XoxcToken::parse("xoxc-1-2-3").unwrap();
    assert!(!format!("{t:?}").contains("xoxc"));
    let c = SessionCookie::parse("d=xoxd-a; d-s=99").unwrap();
    let dbg = format!("{c:?}");
    assert!(!dbg.contains("xoxd") && !dbg.contains("99"));
    assert_eq!(format!("{}", t.expose().len()), "10");
}

#[test]
fn duplicates_equal_their_source() {
    let c = SessionCookie::parse("d=xoxd-abc%2Fdef; d-s=1234").unwrap();
    let d = c.duplicate();
    assert_eq!(c.header_value(), d.header_value());
    let t = XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap();
    assert_eq!(t.expose(), t.duplicate().expose());
}
