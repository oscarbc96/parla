use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConversationId(pub Arc<str>);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserId(pub Arc<str>);

/// Local client message id (UUID v4 text, generated locally).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientMsgId(pub Arc<str>);

/// Message timestamp, `secs.micros` text. Ordered by numeric value, not text.
#[derive(Clone, Debug)]
pub struct MessageTs(pub Arc<str>);

/// Comparison key: unparsable text sorts first (by text), then numeric.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
enum TsKey<'a> {
    Raw(&'a str),
    Num(u64, u64),
}

impl MessageTs {
    fn key(&self) -> TsKey<'_> {
        let s: &str = &self.0;
        let (secs, frac) = s.split_once('.').unwrap_or((s, ""));
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        let frac_ok = frac.is_empty() || (digits(frac) && frac.len() <= 6);
        if !digits(secs) || !frac_ok {
            return TsKey::Raw(s);
        }
        let Ok(secs) = secs.parse::<u64>() else {
            return TsKey::Raw(s);
        };
        let mut micros = 0u64;
        for i in 0..6 {
            let d = frac.as_bytes().get(i).map_or(0, |b| u64::from(b - b'0'));
            micros = micros * 10 + d;
        }
        TsKey::Num(secs, micros)
    }
}

impl MessageTs {
    /// The canonical `secs.micros` spelling: the fraction padded to six
    /// digits. Unparsable text is returned unchanged. Timestamps that compare
    /// equal normalize to the same text, so a backend keyed by text can store
    /// them once.
    pub fn normalized(&self) -> MessageTs {
        match self.key() {
            TsKey::Num(secs, micros) => MessageTs(Arc::from(format!("{secs}.{micros:06}"))),
            TsKey::Raw(_) => self.clone(),
        }
    }
}

impl MessageTs {
    /// One microsecond earlier (saturating at zero). Unparsable text is returned unchanged.
    pub fn just_before(&self) -> MessageTs {
        match self.key() {
            TsKey::Num(secs, micros) => {
                let total = secs
                    .saturating_mul(1_000_000)
                    .saturating_add(micros)
                    .saturating_sub(1);
                MessageTs(Arc::from(format!(
                    "{}.{:06}",
                    total / 1_000_000,
                    total % 1_000_000
                )))
            }
            TsKey::Raw(_) => self.clone(),
        }
    }
}

impl MessageTs {
    /// One microsecond later. Unparsable text is returned unchanged.
    pub fn just_after(&self) -> MessageTs {
        match self.key() {
            TsKey::Num(secs, micros) => {
                let total = secs
                    .saturating_mul(1_000_000)
                    .saturating_add(micros)
                    .saturating_add(1);
                MessageTs(Arc::from(format!(
                    "{}.{:06}",
                    total / 1_000_000,
                    total % 1_000_000
                )))
            }
            TsKey::Raw(_) => self.clone(),
        }
    }
}

impl PartialEq for MessageTs {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}
impl Eq for MessageTs {}
impl Hash for MessageTs {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}
impl PartialOrd for MessageTs {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for MessageTs {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(&other.key())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MessageId {
    pub conversation: ConversationId,
    pub ts: MessageTs,
}

/// Provider file id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub Arc<str>);

/// A provider-authenticated URL, only ever fetched through
/// `ChatProvider::fetch`/`download`. `Debug` prints the host only.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RemoteUrl(pub Arc<str>);

impl std::fmt::Debug for RemoteUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rest = self.0.split_once("://").map_or(&*self.0, |(_, r)| r);
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = host.rsplit('@').next().unwrap_or("");
        write!(f, "RemoteUrl({host})")
    }
}
