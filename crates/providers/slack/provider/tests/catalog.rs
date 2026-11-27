//! The provider catalog: the unofficial notice (exact wording in English) and
//! its translations.

use std::path::PathBuf;

use i18n::{LocalePref, LocaleSet, Localizer};

fn localizer(code: &str) -> Localizer {
    let chain = i18n::chain(
        &LocalePref::Code(code.parse().unwrap()),
        &[],
        LocaleSet::embedded(),
    );
    Localizer::new(chain, &[slack::catalog()]).unwrap()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

fn read(name: &str) -> String {
    std::fs::read_to_string(repo_root().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn unofficial_notice_is_exact_in_english_and_translated() {
    assert_eq!(
        localizer("en").format("slack", "unofficial-notice", None),
        "Not created by, affiliated with, or supported by Slack Technologies, LLC."
    );
    for code in ["es", "ca"] {
        let text = localizer(code)
            .format("slack", "unofficial-notice", None)
            .into_owned();
        assert!(text.contains("Slack Technologies, LLC."), "{code}: {text}");
        assert!(!text.starts_with("Not created"), "{code}: {text}");
    }
}

#[test]
fn privacy_and_terms_documents_include_unofficial_notice() {
    let notice = localizer("en")
        .format("slack", "unofficial-notice", None)
        .into_owned();
    for name in ["PRIVACY.md", "TERMS.md"] {
        let text = read(name);
        assert!(text.contains(&notice), "{name} missing notice:\n{text}");
    }
}

#[test]
fn error_texts_exist_in_every_locale() {
    for code in ["en", "es", "ca"] {
        let l = localizer(code);
        for id in ["error-foreign-host", "error-upload-not-visible"] {
            let text = l.format("slack", id, None).into_owned();
            assert!(!text.starts_with("slack."), "{code}/{id}: {text}");
        }
    }
}

#[test]
fn sign_in_texts_are_catalog_ids_translated_in_every_locale() {
    use components::sign_in::{SignIn as _, SignInError, SignInText};
    use slack_auth::webview::CaptureFailure;
    use std::sync::Arc;

    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let s = slack::SlackSignIn::new(
        rt.handle().clone(),
        std::path::PathBuf::from("accounts.json"),
        Arc::new(slack_auth::MemoryStore::default()),
    );
    let mut texts: Vec<SignInText> = s.notice();
    for o in s.options() {
        texts.push(o.label.clone());
        texts.extend(o.help.clone());
        texts.extend(o.fields.iter().map(|f| f.label.clone()));
    }
    for f in [
        CaptureFailure::DisallowedUserAgent,
        CaptureFailure::NoCookie,
        CaptureFailure::NoTeams,
        CaptureFailure::Closed,
    ] {
        match slack::sign_in::capture_failure(f) {
            SignInError::SwitchTo { reason, .. } => texts.push(reason),
            SignInError::Failed(m) => texts.push(m),
        }
    }
    assert!(texts.len() > 10);
    let en = localizer("en");
    for code in ["en", "es", "ca"] {
        let l = localizer(code);
        for t in &texts {
            let SignInText::Localized(lt) = t else {
                panic!("sign-in text must be localised: {t:?}");
            };
            let text = l.text(lt).into_owned();
            assert!(!text.starts_with("slack."), "{code}/{}: {text}", lt.id);
            if code != "en" {
                assert_ne!(
                    text,
                    en.text(lt).into_owned(),
                    "{code}/{} untranslated",
                    lt.id
                );
            }
        }
    }
}
