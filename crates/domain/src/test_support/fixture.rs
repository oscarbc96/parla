//! The synthetic `perf` workspace: 1 workspace, 300 conversations, 2,000
//! members and 20,000 messages, deterministic from a seed.
//! Everything is generated; nothing comes from a real workspace.

use std::sync::Arc;

use super::{FakeProvider, conv, member, uid};
use crate::{
    Capabilities, ConversationKind, CustomEmoji, EmojiRef, FileId, FileKind, FileRef, Message,
    MessageId, MessageTs, ProviderExt, Reaction, Rendition, UserId,
};

/// A 32x32 green disc: the fixture's custom `:solved:` emoji.
const SOLVED_PNG: &[u8] = include_bytes!("solved.png");

/// A 32x32, 3-frame animated GIF (100 ms frames): the fixture's custom
/// `:party:` emoji, for the animated-emoji perf scenarios.
const PARTY_GIF: &[u8] = include_bytes!("party.gif");

pub const CONVERSATIONS: usize = 300;
pub const MEMBERS: usize = 2_000;
pub const MESSAGES: usize = 20_000;
/// The first conversations are busy channels of this many messages each.
pub const BUSY_CONVERSATIONS: usize = 8;
pub const BUSY_MESSAGES: usize = 2_000;

/// Splitmix64: a tiny deterministic generator, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Conversation id `i` (`C0000`..).
pub fn conversation_id(i: usize) -> String {
    format!("C{i:04}")
}

/// Member id `i` (`U0000`..). `U0000` is the self user.
pub fn member_id(i: usize) -> String {
    format!("U{i:04}")
}

const WORDS: &[&str] = &[
    "build", "deploy", "review", "merge", "ship", "fix", "test", "release", "queue", "cache",
    "latency", "budget", "rollout", "metrics", "plan", "draft", "sync", "check", "alert", "retry",
];

fn text(r: &mut Rng, users: usize) -> String {
    let mut out = String::new();
    for k in 0..(4 + r.below(14)) {
        if k > 0 {
            out.push(' ');
        }
        out.push_str(WORDS[r.below(WORDS.len() as u64) as usize]);
    }
    match r.below(6) {
        0 => out.push_str(&format!(
            " *bold* and _italic_ <https://example.com/p/{}|link>",
            r.below(1000)
        )),
        1 => out.push_str(&format!(
            " <@{}> please look `code_{}`",
            member_id(r.below(users as u64) as usize),
            r.below(100)
        )),
        2 => out.push_str(" :tada: :solved: ~old~ <#C0001|general>"),
        3 => out.push_str("\n> quoted line\n```\nlet x = 1;\n```"),
        _ => {}
    }
    out
}

/// Timestamp of the `k`th message of a conversation: 37 s apart from a fixed base.
fn ts_of(conv_index: usize, k: usize) -> MessageTs {
    let secs = 1_700_000_000u64 + k as u64 * 37 + conv_index as u64 % 7;
    MessageTs(Arc::from(format!(
        "{secs}.{:06}",
        (conv_index * 13 + k) % 1_000_000
    )))
}

fn messages_in(i: usize) -> usize {
    if i < BUSY_CONVERSATIONS {
        return BUSY_MESSAGES;
    }
    let rest = MESSAGES - BUSY_CONVERSATIONS * BUSY_MESSAGES;
    let others = CONVERSATIONS - BUSY_CONVERSATIONS;
    rest / others + usize::from(i - BUSY_CONVERSATIONS < rest % others)
}

/// The perf workspace for `seed`. Self user is `U0000`.
pub fn perf_workspace(seed: u64) -> FakeProvider {
    perf_workspace_as("perf", "Perf Fixture", seed)
}

/// `n` perf workspaces for the three-workspace budgets: the
/// first is [`perf_workspace`]`(seed)`, the others are `perf-2`, `perf-3`, ...
/// with seeds `seed + 1`, ... so no two hold the same history.
pub fn perf_workspaces(seed: u64, n: usize) -> Vec<FakeProvider> {
    (0..n)
        .map(|i| match i {
            0 => perf_workspace(seed),
            _ => perf_workspace_as(
                &format!("perf-{}", i + 1),
                &format!("Perf Fixture {}", i + 1),
                seed + i as u64,
            ),
        })
        .collect()
}

fn perf_workspace_as(id: &str, name: &str, seed: u64) -> FakeProvider {
    let p = FakeProvider::new(id, uid(&member_id(0))).with_capabilities(Capabilities {
        custom_emoji: true,
        ..Capabilities::default()
    });
    p.set_workspace_name(name);
    let url = crate::RemoteUrl(Arc::from("https://example.com/emoji/solved.png"));
    p.add_file(url.clone(), SOLVED_PNG.to_vec());
    p.add_custom_emoji(CustomEmoji {
        shortcode: Arc::from("solved"),
        image: crate::ImageRef::Provider(url),
    });
    let party = crate::RemoteUrl(Arc::from("https://example.com/emoji/party.gif"));
    p.add_file(party.clone(), PARTY_GIF.to_vec());
    p.add_custom_emoji(CustomEmoji {
        shortcode: Arc::from("party"),
        image: crate::ImageRef::Provider(party),
    });
    for i in 0..MEMBERS {
        p.add_member(member(&member_id(i), &format!("Member {i}")));
    }
    for i in 0..CONVERSATIONS {
        let mut c = conv(&conversation_id(i), &format!("chan-{i}"));
        c.kind = match i % 10 {
            0..=6 => ConversationKind::Channel,
            7 => ConversationKind::PrivateChannel,
            8 => ConversationKind::Direct,
            _ => ConversationKind::Group,
        };
        c.topic = Arc::from(
            "Team updates and the <https://example.com/handbook|handbook>, see <#C0001|general> :tada:",
        );
        p.add_conversation(c);
        // History is built when a conversation is first read: 20,000 messages
        // held up front would be a large slice of the idle memory budget that
        // belongs to the (fake) server, not to Parla.
        p.add_lazy_messages(
            &crate::ConversationId(Arc::from(conversation_id(i))),
            move || conversation_history(seed, i),
        );
    }
    p
}

/// The messages of conversation `i`, ascending by timestamp, deterministic
/// from `seed` and `i` alone (each conversation has its own generator).
fn conversation_history(seed: u64, i: usize) -> Vec<Message> {
    let mut r = Rng(seed ^ (i as u64 + 1).wrapping_mul(0xD6E8_FEB8_6659_FD93));
    let n = messages_in(i);
    let mut batch = Vec::with_capacity(n);
    let mut k = 0;
    while k < n {
        let author = UserId(Arc::from(member_id(
            1 + r.below(MEMBERS as u64 - 1) as usize,
        )));
        let id = MessageId {
            conversation: crate::ConversationId(Arc::from(conversation_id(i))),
            ts: ts_of(i, k),
        };
        let mut m = Message::new(id.clone(), Some(author), Arc::from(text(&mut r, MEMBERS)));
        let thread = k % 20 == 0 && k + 2 < n;
        if thread {
            m.reply_count = 2;
            m.latest_reply = Some(ts_of(i, k + 2));
        }
        if k % 7 == 0 {
            let users: Arc<[UserId]> = Arc::from([uid(&member_id(1 + r.below(50) as usize))]);
            m.reactions = Arc::from([Reaction {
                emoji: EmojiRef {
                    shortcode: Arc::from("tada"),
                    unicode: Some(Arc::from("\u{1F389}")),
                    skin_tone: None,
                },
                count: users.len() as u32,
                users,
                me: false,
            }]);
        }
        if k % 11 == 0 {
            let users: Arc<[UserId]> = Arc::from([uid(&member_id(1))]);
            let mut all = m.reactions.to_vec();
            all.push(Reaction {
                emoji: EmojiRef {
                    shortcode: Arc::from("solved"),
                    unicode: None,
                    skin_tone: None,
                },
                count: users.len() as u32,
                users,
                me: false,
            });
            m.reactions = all.into();
        }
        if k % 50 == 3 {
            m.files = Arc::from([FileRef {
                id: FileId(Arc::from(format!("F{i:04}{k:05}"))),
                name: Arc::from("shot.png"),
                title: None,
                mime: Arc::from("image/png"),
                size: 40_000,
                kind: FileKind::Image,
                thumbs: Arc::from([Rendition {
                    url: crate::RemoteUrl(Arc::from(format!("https://example.com/f/{i}/{k}.png"))),
                    width: 360,
                    height: 240,
                }]),
                download: None,
                preview: None,
                video: None,
                tombstone: false,
                lines: None,
                ext: ProviderExt::none(),
            }]);
        }
        batch.push(m);
        k += 1;
        if thread {
            for j in 1..=2 {
                let mut reply = Message::new(
                    MessageId {
                        conversation: id.conversation.clone(),
                        ts: ts_of(i, k),
                    },
                    Some(uid(&member_id(1 + r.below(MEMBERS as u64 - 1) as usize))),
                    Arc::from(format!("reply {j} {}", text(&mut r, MEMBERS))),
                );
                reply.thread_root = Some(id.ts.clone());
                batch.push(reply);
                k += 1;
            }
        }
    }
    batch.sort_by(|a, b| a.id.ts.cmp(&b.id.ts));
    batch
}

/// Scale of [`sidebar_scale_workspace`]: a large account, for sidebar scroll
/// and build-cost checks.
pub const SCALE_CONVERSATIONS: usize = 500;
pub const SCALE_MEMBERS: usize = 10_000;

/// A large synthetic account: 500 conversations (channels, DMs, and group DMs
/// of 3 to 6 participants, a third of them unread), 10,000 members with
/// handles, no messages. Deterministic.
pub fn sidebar_scale_workspace() -> FakeProvider {
    let p = FakeProvider::new("scale", uid(&member_id(0)));
    p.set_workspace_name("Scale Fixture");
    for i in 0..SCALE_MEMBERS {
        let mut m = member(&member_id(i), &format!("Member {i}"));
        m.handle = Arc::from(format!("member.{i}"));
        p.add_member(m);
    }
    for i in 0..SCALE_CONVERSATIONS {
        let mut c = conv(&conversation_id(i), &format!("chan-{i}"));
        c.kind = match i % 10 {
            0..=5 => ConversationKind::Channel,
            6 => ConversationKind::PrivateChannel,
            7 | 8 => ConversationKind::Direct,
            _ => ConversationKind::Group,
        };
        match c.kind {
            ConversationKind::Direct => {
                c.peer = Some(uid(&member_id(1 + (i * 37) % (SCALE_MEMBERS - 1))));
            }
            ConversationKind::Group => {
                let n = 3 + i % 4;
                c.participants = (0..n)
                    .map(|k| {
                        Arc::<str>::from(format!(
                            "member.{}",
                            1 + (i * 131 + k * 977) % (SCALE_MEMBERS - 1)
                        ))
                    })
                    .collect();
            }
            _ => {}
        }
        c.unread = i % 3 == 0;
        p.add_conversation(c);
    }
    p
}

/// A small workspace whose directory is incomplete, like a large account that
/// streams its members: some authors, a replier and a bot are not in the
/// bootstrap and resolve through `members()`, and one author never does.
/// Self user is `u1`; `#general` holds the messages. The sidebar also has
/// direct messages: one open DM whose peer is in the directory, one whose peer
/// resolves on demand, one whose peer never resolves (its name is the raw id,
/// as an old cache stored it), a group DM mixing display-name, real-name-only
/// and unresolved participants, and two closed ones that must stay hidden.
/// `UNRESOLVED_PEER` is shaped like a provider user id.
/// A peer id the provider never resolves; shaped like a provider user id.
pub const UNRESOLVED_PEER: &str = "U0AB12CD34E";

pub fn unresolved_names_workspace() -> FakeProvider {
    const T0: u64 = 1_700_000_000;
    let at = |secs: u64| MessageTs(Arc::from(format!("{secs}.000001")));
    let p = FakeProvider::new("names", uid("u1")).with_capabilities(crate::Capabilities {
        threads: true,
        member_lookup: true,
        ..crate::Capabilities::default()
    });
    p.set_workspace_name("Names Fixture");
    p.add_member(member("u1", "Me"));
    let mut ana = member("u2", "Ana Reyes");
    ana.handle = Arc::from("ana.reyes");
    p.add_member(ana);
    let mut dana = member("u6", "Dana Ortiz");
    dana.handle = Arc::from("dana.ortiz");
    // Real name only: no display name.
    dana.display_name = Arc::from("");
    p.add_lookup_member(dana);
    p.add_lookup_member(member("u7", "Eli Park"));
    let mut bot = member("B0001", "Build Notifier");
    bot.is_bot = true;
    p.add_lookup_member(bot);
    p.add_conversation(conv("C1", "general"));
    let dm = |id: &str, peer: &str, name: &str, open: bool| {
        let mut c = conv(id, name);
        c.kind = ConversationKind::Direct;
        c.peer = Some(uid(peer));
        c.is_open = open;
        c
    };
    p.add_conversation(dm("D1", "u2", "", true));
    p.add_conversation(dm("D2", "u6", "", true));
    p.add_conversation(dm("D3", UNRESOLVED_PEER, UNRESOLVED_PEER, true));
    p.add_conversation(dm("D4", "u7", "", false));
    let group = |id: &str, handles: &[&str], open: bool| {
        let mut c = conv(id, "mpdm");
        c.kind = ConversationKind::Group;
        c.participants = handles.iter().map(|h| Arc::<str>::from(*h)).collect();
        c.is_open = open;
        c
    };
    p.add_conversation(group("G1", &["ana.reyes", "dana.ortiz", "scott.m"], true));
    p.add_conversation(group("G2", &["ana.reyes", "scott.m"], false));
    let by = |secs: u64, author: &str, text: &str| {
        let id = MessageId {
            conversation: crate::ConversationId(Arc::from("C1")),
            ts: at(secs),
        };
        Message::new(id, Some(uid(author)), Arc::from(text))
    };
    let mut root = by(T0, "u6", "Which database should the cache use?");
    root.reply_count = 2;
    root.latest_reply = Some(at(T0 + 120));
    root.reply_users = Arc::from([uid("u7"), uid("u9")]);
    let mut r1 = by(T0 + 60, "u7", "SQLite, it is already a dependency.");
    r1.thread_root = Some(root.id.ts.clone());
    let mut r2 = by(T0 + 120, "u9", "Agreed, one file and no server.");
    r2.thread_root = Some(root.id.ts.clone());
    p.add_messages(
        &crate::ConversationId(Arc::from("C1")),
        vec![
            root,
            r1,
            r2,
            by(T0 + 600, "u2", "A person from the directory."),
            by(T0 + 700, "B0001", "Build 4821 passed."),
            by(T0 + 800, "u9", "Someone the provider cannot resolve."),
        ],
    );
    p
}
