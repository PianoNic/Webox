//! Keyboard shortcut system (`keyboardShortcuts.json`, Windows column `qt`).
//!
//! * `Combo`: a Qt key string ("Ctrl+Shift+N", "Alt+LeftArrow", "PgUp", "Ctrl++") <-> an egui key event.
//! * `registry()`: every command of the JSON with its German description and handler (`table.rs`).
//! * `State`: the user's overrides (settings JSON `shortcuts`), the resolved bindings and the UI state.
//! * `App::shortcuts_frame`: per-frame dispatch.
//! * `popup.rs`: "Suche nach Tastenkombination öffnen" (Ctrl+/). `commands.rs`: the commands implemented here.
//! * `global.rs`: global hotkeys (Windows RegisterHotKey on a message-loop thread).
//! The settings page lives in `ui/settings_shortcuts.rs`.
use crate::*;
use std::sync::OnceLock;
use table::Kind;

mod commands;
#[cfg(windows)]
pub(crate) mod global;
#[cfg(target_os = "linux")]
#[path = "global_linux.rs"]
pub(crate) mod global;
pub(crate) mod popup;
pub(crate) mod table;

/// The shortcut table: the Webex `keyboardShortcuts.json` when it is in `assets/cisco/`, else the Webox
/// table with the same commands, keys and flags (`assets/keyboard-shortcuts.json`, without the descriptions).
fn json() -> &'static str {
    crate::assets::cisco_assets::text("keyboardShortcuts.json").unwrap_or(include_str!("../../assets/keyboard-shortcuts.json"))
}

// Key combinations.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) struct Combo { pub ctrl: bool, pub alt: bool, pub shift: bool, pub key: Key }

const LETTERS: [Key; 26] = [Key::A, Key::B, Key::C, Key::D, Key::E, Key::F, Key::G, Key::H, Key::I, Key::J, Key::K, Key::L, Key::M,
    Key::N, Key::O, Key::P, Key::Q, Key::R, Key::S, Key::T, Key::U, Key::V, Key::W, Key::X, Key::Y, Key::Z];
const DIGITS: [Key; 10] = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
const FKEYS: [Key; 12] = [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9, Key::F10, Key::F11, Key::F12];
/// Named keys; the first alias of a key is its canonical name (what we store in the settings JSON).
const NAMED: &[(&str, Key)] = &[
    ("Return", Key::Enter), ("Enter", Key::Enter), ("Spacebar", Key::Space), ("Space", Key::Space), ("Tab", Key::Tab), ("Backspace", Key::Backspace),
    ("Delete", Key::Delete), ("Del", Key::Delete), ("Insert", Key::Insert), ("Home", Key::Home), ("End", Key::End),
    ("PgUp", Key::PageUp), ("PageUp", Key::PageUp), ("PgDn", Key::PageDown), ("PgDown", Key::PageDown), ("PageDown", Key::PageDown),
    ("LeftArrow", Key::ArrowLeft), ("Left", Key::ArrowLeft), ("RightArrow", Key::ArrowRight), ("Right", Key::ArrowRight),
    ("UpArrow", Key::ArrowUp), ("Up", Key::ArrowUp), ("DownArrow", Key::ArrowDown), ("Down", Key::ArrowDown), ("Esc", Key::Escape), ("Escape", Key::Escape),
    (",", Key::Comma), (".", Key::Period), ("/", Key::Slash), ("\\", Key::Backslash), (";", Key::Semicolon), ("'", Key::Quote),
    ("[", Key::OpenBracket), ("]", Key::CloseBracket), ("-", Key::Minus), ("=", Key::Equals), ("+", Key::Plus), ("`", Key::Backtick),
];
/// Keys that are typed with Shift on some layouts ("/" is Shift+7 on a German keyboard): a binding without Shift also matches with it.
fn is_symbol(k: Key) -> bool {
    matches!(k, Key::Comma | Key::Period | Key::Slash | Key::Backslash | Key::Semicolon | Key::Quote | Key::OpenBracket | Key::CloseBracket
        | Key::Minus | Key::Equals | Key::Plus | Key::Backtick | Key::Colon | Key::Questionmark | Key::Exclamationmark | Key::Pipe)
}

fn key_from_name(n: &str) -> Option<Key> {
    if let Some(k) = NAMED.iter().find(|(a, _)| a.eq_ignore_ascii_case(n)).map(|(_, k)| *k) { return Some(k); }
    let mut ch = n.chars();
    if let (Some(c), None) = (ch.next(), ch.next()) {
        if c.is_ascii_alphabetic() { return Some(LETTERS[(c.to_ascii_uppercase() as u8 - b'A') as usize]); }
        if c.is_ascii_digit() { return Some(DIGITS[(c as u8 - b'0') as usize]); }
    }
    let up = n.to_ascii_uppercase();
    if let Some(num) = up.strip_prefix('F').and_then(|d| d.parse::<usize>().ok()) { if (1..=12).contains(&num) { return Some(FKEYS[num - 1]); } }
    None
}
fn key_name(k: Key) -> String {
    if let Some(i) = LETTERS.iter().position(|x| *x == k) { return ((b'A' + i as u8) as char).to_string(); }
    if let Some(i) = DIGITS.iter().position(|x| *x == k) { return i.to_string(); }
    if let Some(i) = FKEYS.iter().position(|x| *x == k) { return format!("F{}", i + 1); }
    NAMED.iter().find(|(_, x)| *x == k).map_or_else(|| format!("{k:?}"), |(n, _)| n.to_string())
}
/// German key names as Qt prints them (QKeySequence::NativeText on a German system).
fn key_name_de(k: Key) -> String {
    match k {
        Key::Enter => "Eingabe".into(), Key::Space => "Leertaste".into(), Key::Backspace => "Rücktaste".into(), Key::Delete => "Entf".into(),
        Key::Insert => "Einfg".into(), Key::Home => "Pos1".into(), Key::End => "Ende".into(), Key::PageUp => "Bild↑".into(), Key::PageDown => "Bild↓".into(),
        Key::ArrowLeft => "Links".into(), Key::ArrowRight => "Rechts".into(), Key::ArrowUp => "Auf".into(), Key::ArrowDown => "Ab".into(),
        _ => key_name(k),
    }
}

impl Combo {
    /// Parses a Qt key string. `None` for "UNSUPPORTED", macOS strings ("Command+..."), empty strings and unknown keys.
    pub(crate) fn parse(s: &str) -> Option<Combo> {
        let mut rest = s.trim();
        let (mut ctrl, mut alt, mut shift) = (false, false, false);
        loop {
            let mut hit = false;
            for (p, f) in [("Ctrl+", 0), ("Control+", 0), ("Alt+", 1), ("Shift+", 2)] {
                if rest.len() > p.len() && rest[..p.len()].eq_ignore_ascii_case(p) {
                    match f { 0 => ctrl = true, 1 => alt = true, _ => shift = true }
                    rest = &rest[p.len()..]; hit = true; break;
                }
            }
            if !hit { break; }
        }
        Some(Combo { ctrl, alt, shift, key: key_from_name(rest)? })
    }
    /// Canonical Qt string (as stored in the settings): modifiers in the order Ctrl, Alt, Shift.
    pub(crate) fn qt(&self) -> String {
        format!("{}{}{}{}", if self.ctrl { "Ctrl+" } else { "" }, if self.alt { "Alt+" } else { "" }, if self.shift { "Shift+" } else { "" }, key_name(self.key))
    }
    /// How it is shown in German: "Strg+Umschalt+N".
    pub(crate) fn display(&self) -> String {
        format!("{}{}{}{}", if self.ctrl { "Strg+" } else { "" }, if self.alt { "Alt+" } else { "" }, if self.shift { "Umschalt+" } else { "" }, key_name_de(self.key))
    }
    pub(crate) fn from_event(key: Key, m: &Modifiers) -> Combo { Combo { ctrl: m.ctrl || m.command, alt: m.alt, shift: m.shift, key } }
    pub(crate) fn matches(&self, key: Key, m: &Modifiers) -> bool {
        if key != self.key && !(self.key == Key::Plus && key == Key::Equals && m.shift) { return false; }
        if (m.ctrl || m.command) != self.ctrl || m.alt != self.alt { return false; }
        m.shift == self.shift || (is_symbol(self.key) && !self.shift)
    }
    /// A modifier or a function key: the minimum for a user-chosen combination ("starting with Control, Option, Command or Shift").
    pub(crate) fn capturable(&self) -> bool { self.ctrl || self.alt || self.shift || FKEYS.contains(&self.key) }
    pub(crate) fn plain_modifier_free(&self) -> bool { !self.ctrl && !self.alt }
}

// Registry.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Group { General, Messaging, Calling, Formatting, Developer }
impl Group {
    /// The page's section titles ("Allgemein" / "Nachrichten" / "Formatierung" / "Meetings und Anrufe").
    pub(crate) fn title(self) -> &'static str {
        match self { Group::General => "Allgemein", Group::Messaging => "Nachrichten", Group::Calling => "Meetings und Anrufe", Group::Formatting => "Formatierung", Group::Developer => "Entwickler" }
    }
    /// The sections shown on the settings page (Developer stays hidden).
    pub(crate) const SHOWN: [Group; 4] = [Group::General, Group::Messaging, Group::Calling, Group::Formatting];
}

pub(crate) struct Command {
    pub id: &'static str, pub group: Group, pub comment: &'static str, pub customizable: bool, pub default_global: bool, pub can_be_global: bool,
    /// The Windows (`qt`) default; `None` for UNSUPPORTED (and for the macOS strings the JSON ships for Minimize/Maximize).
    pub default: Option<Combo>, pub unsupported: bool,
    pub label: &'static str, pub kind: Kind,
}
impl Command {
    /// Shown on the settings page: not a macOS-only window command.
    pub(crate) fn listed(&self) -> bool {
        self.group != Group::Developer && !matches!(self.id, "Hide" | "HideOthers" | "Zoom" | "FindAndReplace" | "Minimize" | "Maximize")
    }
}
pub(crate) struct Registry { pub cmds: Vec<Command>, by_id: HashMap<&'static str, usize> }
impl Registry { pub(crate) fn index(&self, id: &str) -> Option<usize> { self.by_id.get(id).copied() } }

pub(crate) fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| build_registry(json()))
}
fn leak(s: &str) -> &'static str { Box::leak(s.to_string().into_boxed_str()) }
fn build_registry(json: &str) -> Registry {
    let v: serde_json::Value = serde_json::from_str(json.trim_start_matches('\u{feff}')).unwrap_or_default();
    let mut cmds = Vec::new();
    for g in v["keyboardShortcutGroups"].as_array().into_iter().flatten() {
        let group = match g["name"].as_str().unwrap_or("") { "General" => Group::General, "Messaging" => Group::Messaging, "MeetingsAndCalling" => Group::Calling, "Formatting" => Group::Formatting, _ => Group::Developer };
        for s in g["shortcuts"].as_array().into_iter().flatten() {
            let id = leak(s["command"].as_str().unwrap_or(""));
            let qt = s["keyCombination"]["qt"].as_str().unwrap_or("");
            let (label, kind) = table::spec(id).unwrap_or((id, Kind::Stub));
            cmds.push(Command { id, group, comment: leak(s["comment"].as_str().unwrap_or("")), customizable: s["isCustomizable"].as_bool().unwrap_or(false),
                default_global: s["isGlobal"].as_bool().unwrap_or(false), can_be_global: s["canBeGlobal"].as_bool().unwrap_or(false),
                default: Combo::parse(qt), unsupported: qt == "UNSUPPORTED", label, kind });
        }
    }
    let by_id = cmds.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
    Registry { cmds, by_id }
}

// User overrides / resolved bindings.

/// What the user changed: `key` None = default, Some(None) = removed, Some(Some(c)) = custom; `global` None = default.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(crate) struct Ov { pub key: Option<Option<Combo>>, pub global: Option<bool> }

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(crate) struct Resolved {
    pub key: Option<Combo>, pub global: bool, pub custom: bool,
    /// The command's own default is taken by this other command (a custom binding): the command is disabled ("Standardtastenkombinationen werden bereits verwendet").
    pub default_taken_by: Option<usize>,
}

pub(crate) fn resolve(cmds: &[Command], ov: &HashMap<usize, Ov>) -> Vec<Resolved> {
    let mut out: Vec<Resolved> = cmds.iter().enumerate().map(|(i, c)| {
        let o = ov.get(&i).copied().unwrap_or_default();
        let key = match o.key { Some(k) => k, None => c.default };
        Resolved { key, global: key.is_some() && c.can_be_global && o.global.unwrap_or(c.default_global), custom: o.key.is_some(), default_taken_by: None }
    }).collect();
    for i in 0..cmds.len() {
        if out[i].custom { continue; }
        if let Some(d) = cmds[i].default {
            if let Some(j) = (0..cmds.len()).find(|&j| j != i && out[j].custom && out[j].key == Some(d)) { out[i].key = None; out[i].global = false; out[i].default_taken_by = Some(j); }
        }
    }
    out
}

/// What happens to the command that already uses a combination if the user continues.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Fate { Disable, ResetTo(Combo), Locked }
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Conflict { pub other: usize, pub fate: Fate }

/// Finds the (listed) command that already uses `combo`, and what continuing would do to it.
pub(crate) fn find_conflict(cmds: &[Command], res: &[Resolved], idx: usize, combo: Combo) -> Option<Conflict> {
    let other = (0..cmds.len()).find(|&j| j != idx && cmds[j].listed() && res[j].key == Some(combo))?;
    let fate = if !cmds[other].customizable { Fate::Locked } else {
        match cmds[other].default {
            Some(d) if res[other].custom && d != combo && !(0..cmds.len()).any(|j| j != other && res[j].key == Some(d)) => Fate::ResetTo(d),
            _ => Fate::Disable,
        }
    };
    Some(Conflict { other, fate })
}
/// The conflict texts (language pack, QObject); {{0}} = combination, {{1}} = the other command, {{2}} = its new combination.
pub(crate) fn conflict_message(cmds: &[Command], c: &Conflict, combo: Combo, resetting: bool) -> String {
    let (k, who) = (combo.display(), cmds[c.other].label);
    match (c.fate, resetting) {
        (Fate::Locked, _) => format!("{k} wird von {who} verwendet und kann nicht geändert werden."),
        (Fate::Disable, false) => format!("{k} wird von {who} verwendet. Wenn Sie fortfahren möchten, wird die Tastenkombination für {who} deaktiviert."),
        (Fate::ResetTo(d), false) => format!("{k} wird von {who} verwendet. Wenn Sie fortfahren möchten, wird die Tastenkombination für {who} auf {} zurückgesetzt.", d.display()),
        (Fate::Disable, true) => format!("{k} wird von {who} verwendet. Wenn Sie die Werte auf die Standardwerte zurücksetzen, wird die Tastenkombination für {who} deaktiviert."),
        (Fate::ResetTo(d), true) => format!("{k} wird von {who} verwendet. Wenn Sie die Werte auf die Standardwerte zurücksetzen, wird die Tastenkombination für {who} auf {} zurückgesetzt.", d.display()),
    }
}
pub(crate) const CONFLICT_TITLE: &str = "Tastenkombination wird bereits verwendet"; // pack QObject
pub(crate) const DEFAULT_TAKEN: &str = "Standardtastenkombinationen werden bereits verwendet"; // pack string
/// Pack QObject "This is a new shortcut. It's default key combination {{0}} is already assigned to {{1}}. ..."
pub(crate) fn default_taken_message(cmds: &[Command], idx: usize, by: usize) -> String {
    let k = cmds[idx].default.map(|d| d.display()).unwrap_or_default();
    format!("Das ist eine neue Tastenkombination. Die standardmäßige Tastenkombination für {k} ist bereits {} zugewiesen. Um diese Tastenkombination zu verwenden, fügen Sie eine benutzerdefinierte Kombination hinzu.", cmds[by].label)
}

// State.

#[derive(Default)]
pub(crate) struct State {
    loaded: bool, zoom_restored: bool,
    pub ov: HashMap<usize, Ov>, pub res: Vec<Resolved>,
    /// Composer formatting commands pressed since the last frame (ids from the registry, e.g. "ToggleBold"): the composer drains this.
    pub composer_events: Vec<&'static str>,
    pub popup: popup::Popup, pub page: crate::ui::settings_shortcuts::Page,
    pub sign_out_ask: bool,
    #[cfg(any(windows, target_os = "linux"))] pub global: global::Handle,
    global_sig: Vec<(usize, Combo)>,
}

impl State {
    pub(crate) fn load(&mut self, settings: &serde_json::Value) {
        let reg = registry();
        self.ov.clear();
        if let Some(m) = settings["shortcuts"].as_object() {
            for (id, v) in m {
                let Some(i) = reg.index(id) else { continue };
                let key = v["key"].as_str().map(|s| if s.is_empty() { None } else { Combo::parse(s) });
                self.ov.insert(i, Ov { key: key.filter(|k| k.is_some() || v["key"].as_str() == Some("")), global: v["global"].as_bool() });
            }
        }
        self.res = resolve(&reg.cmds, &self.ov);
        self.loaded = true;
    }
    pub(crate) fn to_json(&self) -> serde_json::Value {
        let reg = registry();
        let mut m = serde_json::Map::new();
        let mut keys: Vec<_> = self.ov.iter().collect();
        keys.sort_by_key(|(i, _)| **i);
        for (i, o) in keys {
            let mut e = serde_json::Map::new();
            if let Some(k) = o.key { e.insert("key".into(), json!(k.map(|c| c.qt()).unwrap_or_default())); }
            if let Some(g) = o.global { e.insert("global".into(), json!(g)); }
            if !e.is_empty() { m.insert(reg.cmds[*i].id.into(), serde_json::Value::Object(e)); }
        }
        serde_json::Value::Object(m)
    }
    pub(crate) fn refresh(&mut self) { self.res = resolve(&registry().cmds, &self.ov); }
}

/// The user's override equals the defaults: drop it.
fn normalize(c: &Command, o: &mut Ov) {
    if o.key == Some(c.default) { o.key = None; }
    if o.global == Some(c.default_global) { o.global = None; }
}

impl Command {
    fn kind_native(&self) -> bool { matches!(self.kind, Kind::Native) }
    /// Commands that act on a running call.
    pub(crate) fn needs_call(&self) -> bool { matches!(self.id, "AnswerCall" | "DeclineCall" | "EndCall" | "ToggleAudioMute") }
}

#[cfg(test)]
mod tests;

mod bindings;
#[allow(unused_imports)]
pub(crate) use bindings::*;
