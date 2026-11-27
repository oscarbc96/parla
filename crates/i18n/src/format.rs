//! Locale-aware numbers, dates, times and relative time (ICU4X compiled data).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use fixed_decimal::{Decimal, FloatPrecision};
use fluent_bundle::FluentValue;
use icu_datetime::DateTimeFormatter;
use icu_datetime::DateTimeFormatterPreferences;
use icu_datetime::fieldsets::{T, YMD, YMDT};
use icu_datetime::options::TimePrecision;
use icu_decimal::DecimalFormatter;
use icu_decimal::DecimalFormatterPreferences;
use icu_locale::Locale;
use icu_locale::preferences::extensions::unicode::keywords::HourCycle as IcuHourCycle;
use jiff_icu::ConvertInto;
use unic_langid::LanguageIdentifier;

use crate::Localizer;
use crate::locales::PSEUDO;
use crate::text::LocalizedText;

/// Clock choice; `settings::Clock` maps onto this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HourCycle {
    /// The locale's own convention.
    Locale,
    H24,
    H12,
}

/// Formatter caches, one per thread: ICU's compiled-data formatters are not
/// `Send`, so they cannot live in the `Localizer`. Bounded by the locale list
/// times three clock choices.
#[derive(Default)]
struct Cache {
    decimal: HashMap<Locale, Rc<Option<DecimalFormatter>>>,
    date: HashMap<Locale, Rc<Option<DateTimeFormatter<YMD>>>>,
    time: HashMap<(Locale, usize), Rc<Option<DateTimeFormatter<T>>>>,
    date_time: HashMap<(Locale, usize), Rc<Option<DateTimeFormatter<YMDT>>>>,
}

/// The cached formatter for `key`, built by `ctor` on first use. A failed
/// construction is cached too (as `None`) so it is not retried per call.
fn cached<K: std::hash::Hash + Eq, V>(
    map: &mut HashMap<K, Rc<Option<V>>>,
    key: K,
    ctor: impl FnOnce() -> Option<V>,
) -> Rc<Option<V>> {
    map.entry(key).or_insert_with(|| Rc::new(ctor())).clone()
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

pub(crate) fn icu_locale(lang: &LanguageIdentifier) -> Locale {
    // The pseudo-locale formats like its source.
    let tag = if *lang == PSEUDO {
        "en".to_string()
    } else {
        lang.to_string()
    };
    tag.parse().unwrap_or(Locale::UNKNOWN)
}

fn slot(hc: HourCycle) -> usize {
    match hc {
        HourCycle::Locale => 0,
        HourCycle::H24 => 1,
        HourCycle::H12 => 2,
    }
}

fn prefs(locale: &Locale, hc: HourCycle) -> DateTimeFormatterPreferences {
    let mut prefs = DateTimeFormatterPreferences::from(locale);
    prefs.hour_cycle = match hc {
        HourCycle::Locale => prefs.hour_cycle,
        HourCycle::H24 => Some(IcuHourCycle::H23),
        HourCycle::H12 => Some(IcuHourCycle::H12),
    };
    prefs
}

fn format_decimal(locale: &Locale, d: &Decimal) -> String {
    let f = CACHE.with(|c| {
        cached(&mut c.borrow_mut().decimal, locale.clone(), || {
            DecimalFormatter::try_new(
                DecimalFormatterPreferences::from(locale),
                Default::default(),
            )
            .ok()
        })
    });
    match &*f {
        Some(f) => f.format(d).to_string(),
        None => d.to_string(),
    }
}

/// Carries only the locale; the formatter itself comes from the thread cache.
struct FluentLocale(Locale);

impl intl_memoizer::Memoizable for FluentLocale {
    type Args = ();
    type Error = ();
    fn construct(lang: LanguageIdentifier, _: ()) -> Result<Self, ()> {
        Ok(Self(icu_locale(&lang)))
    }
}

/// Formats Fluent number placeables with the bundle's locale.
pub(crate) fn fluent_number(
    v: &FluentValue<'_>,
    memoizer: &intl_memoizer::concurrent::IntlLangMemoizer,
) -> Option<String> {
    let FluentValue::Number(n) = v else {
        return None;
    };
    let mut d = Decimal::try_from_f64(n.value, FloatPrecision::RoundTrip).ok()?;
    if let Some(min) = n.options.minimum_fraction_digits {
        d.pad_end(-(min as i16));
    }
    memoizer
        .with_try_get::<FluentLocale, _, _>((), |l| format_decimal(&l.0, &d))
        .ok()
}

fn local(
    t: jiff::Timestamp,
    tz: &jiff::tz::TimeZone,
) -> icu_datetime::input::DateTime<icu_calendar::Iso> {
    t.to_zoned_datetime(tz.clone()).datetime().convert_into()
}

impl Localizer {
    fn icu(&self) -> Locale {
        self.icu.clone()
    }

    pub fn number(&self, n: impl Into<Decimal>) -> String {
        format_decimal(&self.icu(), &n.into())
    }

    /// "14:05" or "2:05 PM".
    pub fn time(&self, t: jiff::Timestamp, tz: &jiff::tz::TimeZone, hc: HourCycle) -> String {
        let loc = self.icu();
        let f = CACHE.with(|c| {
            cached(&mut c.borrow_mut().time, (loc.clone(), slot(hc)), || {
                DateTimeFormatter::try_new(prefs(&loc, hc), T::hm()).ok()
            })
        });
        match &*f {
            Some(f) => f.format(&local(t, tz).time).to_string(),
            None => t.to_string(),
        }
    }

    /// Medium date.
    pub fn date(&self, t: jiff::Timestamp, tz: &jiff::tz::TimeZone) -> String {
        let loc = self.icu();
        let f = CACHE.with(|c| {
            cached(&mut c.borrow_mut().date, loc.clone(), || {
                DateTimeFormatter::try_new(prefs(&loc, HourCycle::Locale), YMD::medium()).ok()
            })
        });
        match &*f {
            Some(f) => f.format(&local(t, tz).date).to_string(),
            None => t.to_string(),
        }
    }

    pub fn date_time(&self, t: jiff::Timestamp, tz: &jiff::tz::TimeZone, hc: HourCycle) -> String {
        let loc = self.icu();
        let f = CACHE.with(|c| {
            cached(
                &mut c.borrow_mut().date_time,
                (loc.clone(), slot(hc)),
                || {
                    DateTimeFormatter::try_new(
                        prefs(&loc, hc),
                        YMDT::medium().with_time_precision(TimePrecision::Minute),
                    )
                    .ok()
                },
            )
        });
        match &*f {
            Some(f) => f.format(&local(t, tz)).to_string(),
            None => t.to_string(),
        }
    }

    /// "just now", "5 minutes ago", "3 hours ago", "2 days ago" from the
    /// `i18n` catalog. A `then` in the future reads as "just now".
    pub fn relative(&self, then: jiff::Timestamp, now: jiff::Timestamp) -> String {
        let secs = now.as_second().saturating_sub(then.as_second());
        let (id, count) = match secs {
            ..60 => ("time-just-now", 0),
            60..3600 => ("time-minutes-ago", secs / 60),
            3600..86400 => ("time-hours-ago", secs / 3600),
            _ => ("time-days-ago", secs / 86400),
        };
        let mut text = LocalizedText::new("i18n", id);
        if count > 0 {
            text = text.with("count", count);
        }
        self.text(&text).into_owned()
    }
}
