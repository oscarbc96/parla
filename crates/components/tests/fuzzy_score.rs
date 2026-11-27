use components::fuzzy_score;

#[test]
fn fuzzy_score_is_pinned() {
    let cases: &[(&str, &str, Option<i32>)] = &[
        ("", "anything", Some(0)),
        ("", "", Some(0)),
        ("a", "", None),
        ("abc", "ab", None),
        ("smile", "smile", Some(1000 + 50 + 30 + 15 * 4)),
        ("SMI", "smile", Some(30 + 10 + 25 + 25 - 2)),
        ("sl", "smile", Some(30 + 10 + 10 - 3)),
        ("ile", "smile", Some(10 + 25 + 25 - 2)),
        ("ca", "ĉat", None),
        ("x", "É", None),
    ];
    for (q, c, want) in cases {
        assert_eq!(fuzzy_score(q, c), *want, "{q:?} in {c:?}");
    }
}
