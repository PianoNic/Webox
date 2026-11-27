use super::*;
use eframe::egui::{pos2, Rect};

/// State and lengths for a solve, by name.
struct E<'a> { c: &'a Compiled, keys: Vec<(&'a str, KeyVal<'a>)>, lens: Vec<(&'a str, f32)> }
impl Env for E<'_> {
    fn key(&self, k: usize) -> KeyVal<'_> { self.keys.iter().find(|x| x.0 == self.c.keys[k]).map_or(KeyVal::None, |x| x.1) }
    fn len(&self, n: usize) -> f32 { self.lens.iter().find(|x| x.0 == self.c.lens[n]).map_or(0.0, |x| x.1) }
}

fn default_env<'a>(c: &'a Compiled, page: &'a str, list_hidden: bool) -> E<'a> {
    E { c, keys: vec![("nav.page", KeyVal::Str(page)), ("ui.list_hidden", KeyVal::Bool(list_hidden))],
        lens: vec![("metrics.header_h", 44.8), ("rail.width", 60.8), ("list.width", 367.0), ("settings.nav_width", 280.0)] }
}

fn component(out: &[Placed], c: Component) -> Option<Rect> {
    out.iter().find_map(|p| match p { Placed::Component(x, r) if *x == c => Some(*r), _ => None })
}

#[test]
fn the_default_layout_loads_and_places_the_webex_arrangement() {
    let (l, c) = load(DEFAULT_LAYOUT).expect("the built-in layout is valid");
    assert_eq!(l.id, "webex-default");
    let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(1200.0, 720.0));
    let mut out = vec![];
    c.solve(area, &default_env(&c, "messaging", false), &mut out);
    assert_eq!(component(&out, Component::Header), Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(1200.0, 44.8))));
    assert_eq!(component(&out, Component::Rail), Some(Rect::from_min_max(pos2(0.0, 44.8), pos2(60.8, 720.0))));
    assert_eq!(component(&out, Component::SpaceList), Some(Rect::from_min_max(pos2(60.8, 44.8), pos2(427.8, 720.0))));
    assert_eq!(component(&out, Component::Conversation), Some(Rect::from_min_max(pos2(431.8, 44.8), pos2(1200.0, 720.0))));
    // Cards come before what they hold, components in tree order (the Tab order).
    let order: Vec<_> = out.iter().map(|p| match p { Placed::Component(c, _) => c.id(), Placed::Card(_) => "card" }).collect();
    assert_eq!(order, ["header", "rail", "card", "space_list", "card", "conversation"]);
}

#[test]
fn switches_and_conditions_follow_the_state() {
    let (_, c) = load(DEFAULT_LAYOUT).unwrap();
    let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(1200.0, 720.0));
    let mut out = vec![];
    // The list hidden: the conversation takes the whole content area.
    c.solve(area, &default_env(&c, "messaging", true), &mut out);
    assert_eq!(component(&out, Component::SpaceList), None);
    assert_eq!(component(&out, Component::Conversation), Some(Rect::from_min_max(pos2(60.8, 44.8), pos2(1200.0, 720.0))));
    c.solve(area, &default_env(&c, "settings", false), &mut out);
    assert_eq!(component(&out, Component::SettingsNav), Some(Rect::from_min_max(pos2(60.8, 44.8), pos2(340.8, 720.0))));
    assert_eq!(component(&out, Component::SettingsPage), Some(Rect::from_min_max(pos2(344.8, 44.8), pos2(1200.0, 720.0))));
    c.solve(area, &default_env(&c, "calls", false), &mut out);
    assert_eq!(component(&out, Component::Page), Some(Rect::from_min_max(pos2(60.8, 44.8), pos2(1200.0, 720.0))));
    assert_eq!(component(&out, Component::Conversation), None);
}

#[test]
fn preferred_sizes_are_clamped_and_named_lengths_follow_the_caller() {
    let (_, c) = load(DEFAULT_LAYOUT).unwrap();
    let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(1200.0, 720.0));
    let mut out = vec![];
    let mut e = default_env(&c, "messaging", false);
    e.lens[2].1 = 900.0; // a stored list width beyond the maximum
    e.lens[1].1 = 196.0; // the wide rail
    c.solve(area, &e, &mut out);
    assert_eq!(component(&out, Component::Rail).unwrap().width(), 196.0);
    assert_eq!(component(&out, Component::SpaceList).unwrap().width(), 400.0);
}

#[test]
fn solving_reuses_the_buffer() {
    let (_, c) = load(DEFAULT_LAYOUT).unwrap();
    let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(1200.0, 720.0));
    let mut out = Vec::with_capacity(16);
    let p = out.as_ptr();
    for _ in 0..3 { c.solve(area, &default_env(&c, "messaging", false), &mut out); }
    assert_eq!(out.as_ptr(), p);
}

fn errors(json: &str) -> Vec<String> { load(json).err().unwrap_or_default().iter().map(|e| e.to_string()).collect() }

#[test]
fn findings_name_the_json_path() {
    let e = errors(r#"{"schema":"webox-ui-layout/1","id":"x","surfaces":{"main":{"dock":"top","child":{"slot":"headr"},"rest":{"slot":"page"}}}}"#);
    assert!(e.iter().any(|m| m.starts_with("surfaces.main.child.slot: unknown component")), "{e:?}");
    let e = errors(r#"{"schema":"webox-ui-layout/1","id":"x","surfaces":{"main":{"slot":"page","colour":"red"}}}"#);
    assert!(e.iter().any(|m| m == "surfaces.main.colour: unknown key"), "{e:?}");
    let e = errors(r#"{"schema":"webox-ui-layout/2","id":"x","surfaces":{"main":{"slot":"page"}}}"#);
    assert!(e.iter().any(|m| m.starts_with("schema: unknown schema")), "{e:?}");
    let e = errors(r#"{"schema":"webox-ui-layout/1","id":"x","surfaces":{"main":{"dock":"top","size":{"min":50,"max":10},"child":{"slot":"page"}}}}"#);
    assert!(e.iter().any(|m| m == "surfaces.main.rest: missing"), "{e:?}");
    assert!(e.iter().any(|m| m.contains("min 50 is larger than max 10")), "{e:?}");
    let e = errors("{ not json");
    assert!(e[0].starts_with("line 1 column"), "{e:?}");
}

#[test]
fn required_components_and_variants_are_checked() {
    let e = errors(r#"{"schema":"webox-ui-layout/1","id":"x","surfaces":{"main":{"slot":"page"}}}"#);
    for c in ["header", "rail", "space_list", "conversation", "settings_nav", "settings_page"] {
        assert!(e.iter().any(|m| m.starts_with(&format!("surfaces.main: {c} is not placed"))), "{c}: {e:?}");
    }
    let bad = DEFAULT_LAYOUT.replace("\"labels\": \"when_wide\"", "\"labels\": \"sometimes\"").replace("\"orientation\": \"vertical\"", "\"shape\": \"round\"");
    let e = errors(&bad);
    assert!(e.iter().any(|m| m == "surfaces.main.rest.child: rail.labels has no option \"sometimes\""), "{e:?}");
    assert!(e.iter().any(|m| m.contains("rail has no variant \"shape\" (contract rail@1)")), "{e:?}");
}

#[test]
fn contracts_line_up_with_the_components() {
    for c in Component::ALL {
        let s = c.spec();
        assert_eq!(Component::parse(s.id), Some(c), "{} is at the wrong place in SPECS", s.id);
        assert!(s.version >= 1 && s.min[0] > 0.0 && s.min[1] > 0.0, "{}", s.id);
        for (v, opts) in s.variants { assert!(!opts.is_empty(), "{}.{v} has no options", s.id); }
        if let Need::PartOf(whole) = s.need { assert!(whole.spec().parts.contains(&s.id), "{} is not listed as a part of {}", s.id, whole.id()); }
        for a in s.emits { assert!(a.contains('.'), "{}: action id {a}", s.id); }
    }
}

#[test]
fn every_contract_is_documented() {
    let docs = include_str!("../../../docs/plugins.md");
    for c in Component::ALL {
        assert!(docs.contains(&format!("| `{}@{}` |", c.id(), c.spec().version)), "docs/plugins.md lacks {}@{}", c.id(), c.spec().version);
    }
}

#[test]
fn parts_cannot_be_placed_on_their_own_yet() {
    let bad = DEFAULT_LAYOUT.replace(r#""child": { "slot": "conversation" }"#, r#""child": { "slot": "message_list" }"#);
    let e = errors(&bad);
    assert!(e.iter().any(|m| m == "surfaces.main: message_list is drawn by conversation and cannot be placed on its own yet (conversation@1)"), "{e:?}");
}
