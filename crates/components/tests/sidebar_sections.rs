//! Parla's own sidebar sections: create, rename, delete, reorder, and moving a
//! conversation in and out of them.

use components::{SectionOp, apply_section_op, section_of};
use settings::CustomSection;

fn sec(id: u32, name: &str, convs: &[&str]) -> CustomSection {
    CustomSection {
        id,
        name: name.into(),
        conversations: convs.iter().map(|c| c.to_string()).collect(),
    }
}

fn names(s: &[CustomSection]) -> Vec<(u32, &str)> {
    s.iter().map(|s| (s.id, s.name.as_str())).collect()
}

#[test]
fn create_appends_with_a_fresh_id_and_trims_the_name() {
    let mut s = vec![sec(3, "Focus", &[])];
    assert!(apply_section_op(
        &mut s,
        SectionOp::Create {
            name: "  Clients ".into()
        }
    ));
    assert_eq!(names(&s), [(3, "Focus"), (4, "Clients")]);
    assert!(!apply_section_op(
        &mut s,
        SectionOp::Create { name: "   ".into() }
    ));
    assert_eq!(s.len(), 2);
    let mut empty = Vec::new();
    apply_section_op(&mut empty, SectionOp::Create { name: "A".into() });
    assert_eq!(names(&empty), [(1, "A")]);
}

#[test]
fn long_names_are_cut() {
    let mut s = Vec::new();
    apply_section_op(
        &mut s,
        SectionOp::Create {
            name: "x".repeat(200),
        },
    );
    assert_eq!(s[0].name.chars().count(), components::SECTION_NAME_MAX);
}

#[test]
fn rename_and_delete() {
    let mut s = vec![sec(1, "Focus", &["C1"]), sec(2, "Clients", &[])];
    assert!(apply_section_op(
        &mut s,
        SectionOp::Rename {
            id: 2,
            name: "Partners".into()
        }
    ));
    assert!(!apply_section_op(
        &mut s,
        SectionOp::Rename {
            id: 2,
            name: "Partners".into()
        }
    ));
    assert!(!apply_section_op(
        &mut s,
        SectionOp::Rename {
            id: 9,
            name: "X".into()
        }
    ));
    assert!(apply_section_op(&mut s, SectionOp::Delete { id: 1 }));
    assert!(!apply_section_op(&mut s, SectionOp::Delete { id: 1 }));
    assert_eq!(names(&s), [(2, "Partners")]);
    assert_eq!(section_of(&s, "C1"), None);
}

#[test]
fn reorder_moves_a_section() {
    let mut s = vec![sec(1, "A", &[]), sec(2, "B", &[]), sec(3, "C", &[])];
    assert!(apply_section_op(
        &mut s,
        SectionOp::Reorder { id: 3, to: 0 }
    ));
    assert_eq!(names(&s), [(3, "C"), (1, "A"), (2, "B")]);
    assert!(apply_section_op(
        &mut s,
        SectionOp::Reorder { id: 3, to: 99 }
    ));
    assert_eq!(names(&s), [(1, "A"), (2, "B"), (3, "C")]);
    assert!(!apply_section_op(
        &mut s,
        SectionOp::Reorder { id: 1, to: 0 }
    ));
}

#[test]
fn a_conversation_sits_in_one_section_at_most() {
    let mut s = vec![sec(1, "A", &["C1"]), sec(2, "B", &[])];
    assert!(apply_section_op(
        &mut s,
        SectionOp::MoveConversation {
            conversation: "C1".into(),
            to: Some(2)
        }
    ));
    assert_eq!(section_of(&s, "C1"), Some(2));
    assert!(s[0].conversations.is_empty());
    // Already there, or an unknown section: nothing to write.
    assert!(!apply_section_op(
        &mut s,
        SectionOp::MoveConversation {
            conversation: "C1".into(),
            to: Some(2)
        }
    ));
    assert!(!apply_section_op(
        &mut s,
        SectionOp::MoveConversation {
            conversation: "C1".into(),
            to: Some(7)
        }
    ));
    assert!(apply_section_op(
        &mut s,
        SectionOp::MoveConversation {
            conversation: "C1".into(),
            to: None
        }
    ));
    assert_eq!(section_of(&s, "C1"), None);
}
