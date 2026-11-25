//! Webox's own translations: strings the official Webex packs don't have (Webox-only errors, settings, plugin host
//! texts). Files `assets/i18n/webox/<code>/*.tsv` (any number per language, `<code>` as in [`LANGUAGES`]), embedded by
//! `build.rs`. One `English source<TAB>translation` per line; `#` starts a comment line, blank lines are ignored,
//! `\t` `\n` `\\` are escapes. The source is matched exactly, without a context.
//!
//! [`lookup`](super::lookup) consults it after the official pack missed. The fallback chain is the one of the packs:
//! `fr_CA` -> `fr` -> English (identity).
use std::collections::HashMap;
use std::sync::OnceLock;

mod data {
    include!(concat!(env!("OUT_DIR"), "/webox_i18n.rs"));
}

type Table = HashMap<&'static str, &'static str>;

/// `\t`, `\n`, `\\` unescaped. Strings with an escape are leaked once (the tables live for the whole process).
fn unescape(s: &'static str) -> &'static str {
    if !s.contains('\\') { return s; }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' { out.push(c); continue; }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(o) => { out.push('\\'); out.push(o); }
            None => out.push('\\'),
        }
    }
    Box::leak(out.into_boxed_str())
}

/// Adds the lines of one file to `into` (a later file wins on equal sources).
pub(super) fn parse(text: &'static str, into: &mut Table) {
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() || line.trim_start().starts_with('#') { continue; }
        let Some((src, tr)) = line.split_once('\t') else { continue };
        if src.is_empty() || tr.is_empty() { continue; }
        into.insert(unescape(src), unescape(tr));
    }
}

fn tables() -> &'static HashMap<&'static str, Table> {
    static T: OnceLock<HashMap<&'static str, Table>> = OnceLock::new();
    T.get_or_init(|| {
        let mut m: HashMap<&'static str, Table> = HashMap::new();
        for (code, text) in data::WEBOX_I18N { parse(text, m.entry(code).or_default()); }
        m
    })
}

/// `code`, then its language without the region (`fr_CA` -> `fr`); English ends the chain.
fn chain(code: &str) -> impl Iterator<Item = &str> {
    let base = code.split('_').next().unwrap_or(code);
    [code, base].into_iter().enumerate().filter(move |(i, c)| (*i == 0 || *c != code) && *c != "en").map(|(_, c)| c)
}

/// Webox's translation of `source` for the language `code`.
pub(super) fn get(code: &str, source: &str) -> Option<&'static str> {
    chain(code).find_map(|c| tables().get(c)?.get(source).copied())
}

/// Whether Webox has any own translation for `code` (its chain).
pub(super) fn has(code: &str) -> bool {
    chain(code).any(|c| tables().get(c).is_some_and(|t| !t.is_empty()))
}
