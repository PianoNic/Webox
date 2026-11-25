//! "New message": header + > "Send message". Layout (pt, from the content card's top-left) and strings from the
//! language pack:
//! - a temporary row "New message" (ring avatar with the start-chat glyph, selected) at the top of the list's "Other" section;
//! - header: "New message" + pencil ("Edit space name"), outlined pill "Cancel" at the right;
//! - "To:" + a rounded search field "Add by name, number, or email", an x clear button while there
//!   is text;
//! - a dropdown under the field: "People" and "Spaces" sections; a section without matches shows the cactus
//!   (desert-open-results-192-cobalt-lime) and "We can't find any people ..." / "We can't find any spaces in your
//!   list that match your search."; the keyboard row has a blue focus ring (#64B4FA);
//! - the normal composer below with "Write a message" and no AI button; Esc does not close the pane, "Cancel" does and
//!   re-selects the previous conversation.
//! Behaviour: a picked person becomes a chip (several allowed); the conversation with exactly the chosen people is looked up
//! and shown in the pane, the send goes there; without one the user's own Send creates it: one person = `Req::Direct`,
//! several = `Req::Group` (named via the pencil, if a name was entered). A space pick opens the space. People come from the
//! known list plus the directory search (`Req::DirectorySearch`).
use crate::*;
use crate::cloud::Req;

/// The composer draft key of the pane (also the "space id" the composer sees).
pub(crate) const DRAFT_ID: &str = "\u{1}new-message";
/// Where composer.rs leaves the text of a send in the pane (picked up right after the composer ran).
pub(crate) const SEND_KEY: &str = "\u{1}new-message-send";

#[derive(Clone, Default)]
pub(crate) struct NewMsg { pub open: bool, pub prev: Option<String>, pub query: String, pub to: Vec<Person>, pub sel: Option<usize>, pub focus: bool,
    /// The space name typed via the pencil (empty = the default title "New message").
    pub name: String,
    /// The pencil's edit mode (label + pencil hidden, "Space name" field shown) and its text.
    pub editing: bool, pub name_buf: String,
    /// The last query sent to the directory (one search per change of the text).
    pub asked: String,
    /// The conversation whose messages were requested for the matched-conversation view.
    pub loaded: String }

/// Directory results per query (Resp::Directory; the most recent queries only).
static DIR: std::sync::Mutex<Vec<(String, Vec<Person>)>> = std::sync::Mutex::new(Vec::new());

/// Called by app.rs for `Resp::Directory`: adds a batch of directory people for `q`.
pub(crate) fn directory_results(q: &str, people: Vec<Person>, _finished: bool) {
    let mut d = DIR.lock().unwrap();
    match d.iter_mut().find(|(k, _)| k == q) {
        Some((_, v)) => for p in people { if !v.iter().any(|x| x.id == p.id) { v.push(p); } },
        None => { d.push((q.to_string(), people)); let n = d.len(); if n > 12 { d.drain(..n - 12); } }
    }
}
fn directory_for(q: &str) -> Vec<Person> { DIR.lock().unwrap().iter().find(|(k, _)| k == q).map(|(_, v)| v.clone()).unwrap_or_default() }

/// The people offered for a query: the known people first, then directory people not already known (same id or e-mail).
pub(crate) fn merged_people(known: &[Person], dir: &[Person]) -> Vec<Person> {
    let mut v = known.to_vec();
    for p in dir { if !v.iter().any(|x| x.id == p.id || (!p.email.is_empty() && x.email.eq_ignore_ascii_case(&p.email))) { v.push(p.clone()); } }
    v
}

fn same_person(a: &Person, b: &Person) -> bool { a.id == b.id || (!a.email.is_empty() && a.email.eq_ignore_ascii_case(&b.email)) }

/// The existing conversation with exactly these recipients: one person = the one-to-one space, several = a group space whose
/// other participants are exactly them.
pub(crate) fn matched_space(spaces: &[Space], direct_person: impl Fn(&Space) -> Option<Person>, to: &[Person], me: &Person) -> Option<String> {
    match to.len() {
        0 => None,
        1 => existing_direct(spaces, direct_person, &to[0]).map(|s| s.id.clone()),
        _ => spaces.iter().filter(|s| !s.direct).find(|s| {
            let others: Vec<&Person> = s.participants.iter().filter(|p| !same_person(p, me)).collect();
            others.len() == to.len() && to.iter().all(|t| others.iter().any(|o| same_person(o, t)))
        }).map(|s| s.id.clone()),
    }
}

/// After a send / invite: open the direct space with this person once the list has it.
#[derive(Clone, Default)]
struct PendingDirect { id: String, email: String, group: Vec<Person>, known: Vec<String> }
/// (id, email) of a 1:1 requested from a contact card, picked up by `finish_direct`.
static PENDING_PERSON: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

fn key() -> egui::Id { egui::Id::new("new-message-state") }
pub(crate) fn state(ctx: &egui::Context) -> NewMsg { ctx.data(|d| d.get_temp::<NewMsg>(key())).unwrap_or_default() }
pub(crate) fn set_state(ctx: &egui::Context, s: NewMsg) { ctx.data_mut(|d| d.insert_temp(key(), s)); }
pub(crate) fn is_open(ctx: &egui::Context) -> bool { state(ctx).open }

// Geometry (pt).
const TITLE_X: f32 = 33.6;
const HEAD_Y: f32 = 32.0;
const PILL: (f32, f32) = (91.0, 28.0);
const RIGHT: f32 = 31.0;
const FIELD_X: f32 = 89.0;
const FIELD_Y: f32 = 58.4;
const FIELD_H: f32 = 28.0;
const DROP_GAP: f32 = 4.0;
const HEADER_H: f32 = 33.0;
const ROW_H: f32 = 50.0;
const EMPTY_H: f32 = 224.0;
const FOCUS: Color32 = Color32::from_rgb(0x64, 0xB4, 0xFA);

/// One line of the dropdown.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Line { Header(&'static str), Person(usize), Space(usize), Empty(&'static str) }

impl Line {
    fn h(&self) -> f32 { match self { Line::Header(_) => HEADER_H, Line::Empty(_) => EMPTY_H, _ => ROW_H } }
    fn pickable(&self) -> bool { matches!(self, Line::Person(_) | Line::Space(_)) }
}

/// The dropdown for `q`: people (index into `people`) and group spaces (index into `spaces`) whose name / e-mail / title
/// contains the query, each section with its empty state.
pub(crate) fn lines(q: &str, people: &[Person], spaces: &[Space], me: &Person) -> Vec<Line> {
    let q = q.trim().to_lowercase();
    let mut v = vec![Line::Header("People")];
    let is_me = |p: &Person| p.id == me.id || (!me.email.is_empty() && p.email.eq_ignore_ascii_case(&me.email));
    let ps: Vec<usize> = people.iter().enumerate().filter(|(_, p)| !is_me(p) && (p.name.to_lowercase().contains(&q) || p.email.to_lowercase().contains(&q)))
        .map(|(i, _)| i).take(8).collect();
    if ps.is_empty() { v.push(Line::Empty("We can't find anyone who matches your search.")); } else { v.extend(ps.into_iter().map(Line::Person)); }
    v.push(Line::Header("Spaces"));
    let ss: Vec<usize> = spaces.iter().enumerate().filter(|(_, s)| !s.direct && s.title.to_lowercase().contains(&q)).map(|(i, _)| i).take(8).collect();
    if ss.is_empty() { v.push(Line::Empty("We can't find any spaces in your list that match your search.")); } else { v.extend(ss.into_iter().map(Line::Space)); }
    v
}

/// The line the keyboard ring sits on: the chosen one, else the first pickable line, else the last empty block.
pub(crate) fn ring_index(ls: &[Line], sel: Option<usize>) -> Option<usize> {
    sel.filter(|&i| ls.get(i).is_some_and(|l| l.pickable()))
        .or_else(|| ls.iter().position(|l| l.pickable()))
        .or_else(|| ls.iter().rposition(|l| matches!(l, Line::Empty(_))))
}

/// The existing one-to-one space with `p` (by person id or e-mail).
pub(crate) fn existing_direct<'a>(app_spaces: &'a [Space], direct_person: impl Fn(&Space) -> Option<Person>, p: &Person) -> Option<&'a Space> {
    app_spaces.iter().filter(|s| s.direct).find(|s| direct_person(s).is_some_and(|o| o.id == p.id || (!p.email.is_empty() && o.email.eq_ignore_ascii_case(&p.email))))
}

/// Name with the matched part in semibold ("**Jonas** Weber").
fn hl_job(text: &str, q: &str, size: f32, col: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let lo = text.to_lowercase();
    let ql = q.trim().to_lowercase();
    let reg = egui::TextFormat { font_id: FontId::proportional(size), color: col, ..Default::default() };
    let bold = egui::TextFormat { font_id: semibold(size), color: col, ..Default::default() };
    match (!ql.is_empty()).then(|| lo.find(&ql)).flatten().filter(|&a| text.is_char_boundary(a) && text.is_char_boundary(a + ql.len())) {
        Some(a) => {
            job.append(&text[..a], 0.0, reg.clone());
            job.append(&text[a..a + ql.len()], 0.0, bold);
            job.append(&text[a + ql.len()..], 0.0, reg);
        }
        None => job.append(text, 0.0, reg),
    }
    job
}

#[cfg(test)]
mod tests;

mod pane;
