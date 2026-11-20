use super::*;

fn person(id: &str, name: &str, email: &str) -> Person { Person { id: id.into(), name: name.into(), email: email.into(), ..Default::default() } }

#[test]
fn dropdown_sections_and_empty_states() {
    let me = person("me", "Me Self", "me@x.example");
    let people = vec![me.clone(), person("a", "Jonas Weber", "jonas@x.example"), person("b", "Ben Meier", "ben@x.example")];
    let spaces = vec![Space { id: "s1".into(), title: "Allgemein".into(), ..Default::default() }, Space { id: "d1".into(), title: "Jonas Weber".into(), direct: true, ..Default::default() }];
    let l = lines("jonas", &people, &spaces, &me);
    assert_eq!(l, vec![Line::Header("People"), Line::Person(1), Line::Header("Spaces"), Line::Empty("We can't find any spaces in your list that match your search.")]);
    assert_eq!(ring_index(&l, None), Some(1));
    let l = lines("zzqxw@example.invalid", &people, &spaces, &me);
    assert_eq!(l.iter().filter(|x| matches!(x, Line::Empty(_))).count(), 2);
    assert_eq!(ring_index(&l, None), Some(3)); // the last empty block
    assert!(!lines("self", &people, &spaces, &me).contains(&Line::Person(0)));
    assert_eq!(lines("allg", &people, &spaces, &me)[3], Line::Space(0));
}

#[test]
fn existing_direct_space_is_found_by_id_or_email() {
    let spaces = vec![Space { id: "d1".into(), direct: true, ..Default::default() }, Space { id: "g".into(), ..Default::default() }];
    let other = |_: &Space| Some(person("u-1", "Ann", "ann@x.example"));
    assert_eq!(existing_direct(&spaces, other, &person("u-1", "", "")).map(|s| s.id.as_str()), Some("d1"));
    assert_eq!(existing_direct(&spaces, other, &person("", "", "ANN@x.example")).map(|s| s.id.as_str()), Some("d1"));
    assert!(existing_direct(&spaces, other, &person("u-2", "", "bob@x.example")).is_none());
}

#[test]
fn conversation_match_for_one_or_several_people() {
    let me = person("me", "Me", "me@x.example");
    let (a, b, c) = (person("a", "Ann", "ann@x.example"), person("b", "Bob", "bob@x.example"), person("c", "Cid", "cid@x.example"));
    let spaces = vec![
        Space { id: "d-a".into(), direct: true, ..Default::default() },
        Space { id: "g-ab".into(), participants: vec![me.clone(), a.clone(), b.clone()], ..Default::default() },
        Space { id: "g-abc".into(), participants: vec![a.clone(), b.clone(), c.clone()], ..Default::default() },
    ];
    let dp = |s: &Space| (s.id == "d-a").then(|| a.clone());
    assert_eq!(matched_space(&spaces, dp, &[a.clone()], &me).as_deref(), Some("d-a"));
    assert_eq!(matched_space(&spaces, dp, &[b.clone(), a.clone()], &me).as_deref(), Some("g-ab"));
    assert_eq!(matched_space(&spaces, dp, &[a.clone(), b.clone(), c.clone()], &me).as_deref(), Some("g-abc"));
    assert_eq!(matched_space(&spaces, dp, &[a.clone(), c.clone()], &me), None);
    assert_eq!(matched_space(&spaces, dp, &[b.clone()], &me), None);
    assert_eq!(matched_space(&spaces, dp, &[], &me), None);
}

#[test]
fn directory_people_are_merged_without_duplicates() {
    let known = vec![person("a", "Ann", "ann@x.example")];
    let dir = vec![person("u-a", "Ann", "ANN@x.example"), person("u-z", "Zoe", "zoe@x.example")];
    let m = merged_people(&known, &dir);
    assert_eq!(m.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["a", "u-z"]);
    directory_results("zo", vec![person("u-z", "Zoe", "")], false);
    directory_results("zo", vec![person("u-z", "Zoe", ""), person("u-y", "Zoey", "")], true);
    assert_eq!(directory_for("zo").len(), 2);
}

#[test]
fn highlight_bolds_the_typed_part() {
    let j = hl_job("Jonas Weber", "jonas", 16.0, Color32::WHITE);
    assert_eq!(j.sections.len(), 3);
    assert_eq!(&j.text[j.sections[1].byte_range.clone()], "Jonas");
}
