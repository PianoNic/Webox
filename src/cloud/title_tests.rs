use super::{announcement_blocked, closed_direct, direct_title};
#[test]
fn closed_only_for_a_direct_chat_without_its_person_or_read_only() {
    assert!(closed_direct(true, false, 0, true)); // rule 3: a closed direct chat
    assert!(!closed_direct(true, false, 0, false)); // rule 3 needs its toggle
    assert!(!closed_direct(true, false, 1, true)); // an empty direct chat with its welcome item, and normal ones
    assert!(closed_direct(true, true, 1, false)); // rule 2: READ_ONLY 1:1 (no toggle)
    assert!(!closed_direct(false, true, 0, true)); // a group space is never "closed" by these rules
}
#[test]
fn announcement_only_for_non_moderators_of_an_announcement_space() {
    assert!(announcement_blocked(false, true, false));
    assert!(!announcement_blocked(false, true, true)); // moderators keep the composer
    assert!(!announcement_blocked(false, false, false)); // LOCKED (moderated) alone is not announcement mode
    assert!(!announcement_blocked(true, true, false));
}
#[test]
fn direct_title_fallbacks() {
    assert_eq!(direct_title(Some("Ann Bell"), "Old Name", "Unbenannt"), "Ann Bell");
    assert_eq!(direct_title(None, "Old Name", "Unbenannt"), "Old Name");
    assert_eq!(direct_title(Some("  "), "Old Name", "Unbenannt"), "Old Name");
    assert_eq!(direct_title(None, "", "Unbenannt"), "Unbenannt");
    assert_eq!(direct_title(None, " ", "Unbenannt"), "Unbenannt");
}
