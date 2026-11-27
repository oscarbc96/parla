use std::sync::Arc;

use domain::{ChatProvider, MessageId, MessageTs, ProviderExt};

#[derive(Debug, PartialEq)]
struct Foo(u32);
struct Bar;

fn ts(s: &str) -> MessageTs {
    MessageTs(Arc::from(s))
}

#[test]
fn ts_orders_numerically() {
    assert!(ts("9.000002") < ts("10.000001"));
    assert!(ts("10.000001") < ts("10.000002"));
    assert!(ts("10.000002") > ts("10.000001"));
    assert_eq!(ts("5.000001"), ts("5.000001"));
}

#[test]
fn ts_orders_without_micros_and_garbage_is_total() {
    assert!(ts("9") < ts("10.000001"));
    // Unparsable values still give a total order (sorted before valid ones).
    assert!(ts("garbage") < ts("1.000000"));
    assert_eq!(ts("garbage").cmp(&ts("garbage")), std::cmp::Ordering::Equal);
}

#[test]
fn ts_ordering_consistent_with_eq() {
    // Equal numeric value with different text must not compare as less/greater
    // while being unequal: Eq and Ord agree.
    let a = ts("1.5");
    let b = ts("1.500000");
    assert_eq!(a == b, a.cmp(&b) == std::cmp::Ordering::Equal);
}

#[test]
fn message_id_orders_by_conversation_then_ts() {
    let c = domain::ConversationId(Arc::from("C1"));
    let a = MessageId {
        conversation: c.clone(),
        ts: ts("2.000000"),
    };
    let b = MessageId {
        conversation: c,
        ts: ts("10.000000"),
    };
    assert!(a < b);
}

#[test]
fn ext_downcasts() {
    let e = ProviderExt::new(Foo(1));
    assert_eq!(e.get::<Foo>(), Some(&Foo(1)));
    assert!(e.get::<Bar>().is_none());
    assert!(ProviderExt::none().get::<Foo>().is_none());
    let c = e;
    assert_eq!(c.get::<Foo>(), Some(&Foo(1)));
}

#[test]
fn ext_debug_prints_type_name_only() {
    let s = format!("{:?}", ProviderExt::new(Foo(42)));
    assert!(s.contains("Foo"), "{s}");
    assert!(!s.contains("42"), "{s}");
    assert_eq!(format!("{:?}", ProviderExt::none()), "ProviderExt(none)");
}

#[test]
fn provider_is_object_safe() {
    fn _f(_: Arc<dyn ChatProvider>) {}
}

#[test]
fn capabilities_default_all_off() {
    let c = domain::Capabilities::default();
    assert!(!c.threads && !c.edit && !c.delete && !c.huddles && !c.user_groups);
}

#[test]
fn neighbouring_ts_saturate_instead_of_overflowing() {
    let huge = ts("18446744073709551615.999999");
    assert_eq!(huge.just_after(), huge.just_after());
    let big = ts("18446744073709.000000");
    assert!(big.just_before() < big);
    assert!(big.just_after() > big);
}
