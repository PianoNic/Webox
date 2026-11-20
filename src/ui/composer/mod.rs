//! The message composer (bottom of the chat): toolbar, text box, send pill, emoji `:name` autocomplete, `@mention` popup,
//! emoticon replacement, markdown legend + formatting, edit overlay, attachment chips, drafts. Pure text logic lives in
//! `composer_text.rs`, the popups in `emoji_autocomplete.rs` / `mention_popup.rs`.
//!
//! The action bar has the "M" markdown button and no rich-text toolbar by default; the rich-text toolbar (Fett, Kursiv,
//! Unterstrichen, Durchgestrichen, Überschrift 1-3, Link, Weitere) is only shown when the settings key
//! `composer_rich_toolbar` is true. egui's TextEdit has no rich text: the formatting commands insert markdown (or `<u>`,
//! `<sub>`, `<sup>`) markers in both modes. Mentions are stored beside the text (`St::mentions`) and drawn in the mention colour.
use crate::*;
use super::composer_text::{self as ct, Format, Mention};
use super::{emoji_autocomplete as eac, mention_popup as mp};

/// A staged file: every chip is a file on disk (pasted images / captures are written to the staging folder first), see
/// ui/attach/.
#[derive(Clone, Default)]
pub(crate) struct Chip { pub name: String, pub size: u64, pub path: PathBuf, pub image: bool }

#[derive(Clone, Default)]
struct St {
    space: String, loaded: bool,
    mentions: Vec<Mention>,
    md: bool, legend: bool, legend_anchor: Option<Rect>, rich: bool,
    chips: Vec<Chip>,
    /// Inner width of the staging area at the last frame (the flow's rows depend on it); 0 = not drawn yet.
    chips_w: f32,
    emoji_q: Option<(usize, String)>, emoji_sel: usize, emoji_cancel: Option<usize>,
    ment_q: Option<(usize, String)>, ment_sel: usize, ment_cancel: Option<usize>, ment_dir: Option<bool>,
    rows: usize, confirm_all: bool, was_focused: bool, last_len: usize,
}

/// No extra row inside the composer while editing: the 56 pt title sits above the input box (ui/edit_overlay.rs).
const EDIT_HDR: f32 = 0.0;
// Text row top below the composer line (pt), and the space under the text row: placeholder glyphs 65.6 pt below the line and
// 22.4 pt above the bottom, composer 99.2 pt in total. The text box draws its glyphs 5 pt below the box top.
const BASE_TOP: f32 = 60.6;
const BOTTOM_PAD: f32 = 13.6; // 99.2 - 60.6 - 25 (one text row)
const LINE_H: f32 = 20.0;
const CID: &str = "composer-edit";
const PIC_W: f32 = 22.0; // an emoji picture in the box: 20 px + 2 px air

/// Height of the staging area for the composer state (one row until its width is known).
fn chips_height(st: &St) -> f32 { super::attach::staging_height(st.chips.len(), if st.chips_w > 0.0 { st.chips_w } else { f32::INFINITY }) }

fn st_get(ctx: &egui::Context) -> St { ctx.data(|d| d.get_temp::<St>(egui::Id::new("composer-st"))).unwrap_or_default() }
fn st_put(ctx: &egui::Context, st: St) { ctx.data_mut(|d| d.insert_temp(egui::Id::new("composer-st"), st)); }

/// Layout: (text row top offset from the composer line, text row height, total composer height).
fn metrics(edit: bool, rich: bool, chips_h: f32, rows: usize) -> (f32, f32, f32) {
    let mut top = BASE_TOP + if edit { EDIT_HDR } else { 0.0 };
    if rich { top += 30.0; }
    top += chips_h; // the staging area (68 .. 92), 0 without files
    let h = 25.0 + (rows.clamp(1, 6) - 1) as f32 * LINE_H;
    (top, h, top + h + BOTTOM_PAD)
}

fn set_sel(ctx: &egui::Context, a: usize, b: usize) {
    let cid = egui::Id::new(CID);
    let mut state = egui::TextEdit::load_state(ctx, cid).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(a), egui::text::CCursor::new(b))));
    state.store(ctx, cid);
}
fn get_sel(ctx: &egui::Context, len: usize) -> (usize, usize) {
    egui::TextEdit::load_state(ctx, egui::Id::new(CID)).and_then(|s| s.cursor.char_range()).map_or((len, len), |r| (r.secondary.index.min(len), r.primary.index.min(len)))
}

/// Composer formatting keys (from the shortcut table): AddLink Ctrl+K, ToggleBold/Italic/Underline Ctrl+B/I/U, ToggleMarkdown
/// Alt+Shift+M, ToggleNumberList Alt+Shift+O, ToggleBulletList Alt+Shift+U, ToggleCodeBlock Alt+Shift+0, Headings Alt+Shift+1/2/3,
/// ToggleStrikeThrough Alt+K, ToggleInlineCode Alt+Shift+K, Subscript Ctrl+=, Superscript Ctrl+Shift+=, AddGif Ctrl+G.
#[derive(Clone, Copy, PartialEq, Debug)]
enum KeyAct { Fmt(Format), ToggleMarkdown, Gif }
fn local_format_keys(i: &mut egui::InputState) -> Option<KeyAct> {
    let as_ = Modifiers::ALT | Modifiers::SHIFT;
    let cs = Modifiers::COMMAND | Modifiers::SHIFT;
    let table: [(Modifiers, Key, KeyAct); 18] = [
        (Modifiers::COMMAND, Key::B, KeyAct::Fmt(Format::Bold)), (Modifiers::COMMAND, Key::I, KeyAct::Fmt(Format::Italic)),
        (Modifiers::COMMAND, Key::U, KeyAct::Fmt(Format::Underline)), (Modifiers::COMMAND, Key::K, KeyAct::Fmt(Format::Link)),
        (as_, Key::M, KeyAct::ToggleMarkdown), (as_, Key::O, KeyAct::Fmt(Format::NumberList)), (as_, Key::U, KeyAct::Fmt(Format::BulletList)),
        (as_, Key::Num0, KeyAct::Fmt(Format::CodeBlock)), (as_, Key::Num1, KeyAct::Fmt(Format::Heading(1))), (as_, Key::Num2, KeyAct::Fmt(Format::Heading(2))),
        (as_, Key::Num3, KeyAct::Fmt(Format::Heading(3))), (Modifiers::ALT, Key::K, KeyAct::Fmt(Format::Strike)), (as_, Key::K, KeyAct::Fmt(Format::InlineCode)),
        (Modifiers::COMMAND, Key::Equals, KeyAct::Fmt(Format::Sub)), (cs, Key::Equals, KeyAct::Fmt(Format::Sup)), (cs, Key::Plus, KeyAct::Fmt(Format::Sup)),
        (Modifiers::COMMAND, Key::Plus, KeyAct::Fmt(Format::Sup)), (Modifiers::COMMAND, Key::G, KeyAct::Gif),
    ];
    table.iter().find(|(m, k, _)| i.consume_key(*m, *k)).map(|(_, _, a)| *a)
}

fn mention_colour(t: &Th) -> Color32 { t.tk("message-bodyText-mention-all-text") }

fn em_none(items: &[&eac::Item]) -> bool { items.is_empty() }

/// The field's focus filter (egui's `Memory::begin_pass` moves / drops the focus on keys the filter does not match). While the
/// emoji or mention popup is open, Tab picks the row and Esc only closes the popup, so both keep the focus in the field. The
/// arrows always stay in the field (TextEdit's default).
fn focus_filter(popup_open: bool) -> egui::EventFilter {
    egui::EventFilter { tab: popup_open, escape: popup_open, horizontal_arrows: true, vertical_arrows: true }
}

/// The composer's own popup layers (emoji suggestions, mention list): a press on them keeps the field's focus.
const POPUP_LAYERS: [&str; 2] = ["emoji-autocomplete", "mention-popup"];
fn is_popup_layer(l: egui::LayerId) -> bool { l.order == egui::Order::Foreground && POPUP_LAYERS.iter().any(|n| l.id == egui::Id::new(*n)) }
/// A primary/secondary press this frame landed on one of the composer popups (layer hit test uses last frame's areas).
fn popup_pressed(ctx: &egui::Context) -> bool {
    let p = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
    p.and_then(|p| ctx.layer_id_at(p)).is_some_and(is_popup_layer)
}

/// Where the char ranges `(start, end)` of a laid-out text ended up: x of the first glyph .. x of the glyph after the range, and the row, relative to the galley.
fn cluster_rects(g: &egui::Galley, ranges: &[(usize, usize)]) -> Vec<Option<Rect>> {
    let mut out = vec![None; ranges.len()];
    let mut off = 0usize;
    for row in &g.rows {
        let n = row.glyphs.len();
        for (k, &(a, b)) in ranges.iter().enumerate() {
            if a >= off && a < off + n {
                let x0 = row.glyphs[a - off].pos.x;
                let x1 = if b < off + n { row.glyphs[b - off].pos.x } else { row.rect.right() };
                out[k] = Some(Rect::from_min_max(pos2(x0, row.rect.top()), pos2(x1.max(x0), row.rect.bottom())));
            }
        }
        off += n + usize::from(row.ends_with_newline);
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::*;
    #[test]
    fn popup_keeps_esc_and_tab_in_the_field() {
        let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        let open = super::focus_filter(true);
        let closed = super::focus_filter(false);
        // Matches = the field keeps the key (egui does not move / drop the focus).
        assert!(open.matches(&key(egui::Key::Escape)) && open.matches(&key(egui::Key::Tab)));
        assert!(!closed.matches(&key(egui::Key::Escape)), "no popup: Esc goes on (edit / reply cancel, then focus)");
        assert!(!closed.matches(&key(egui::Key::Tab)), "no popup: Tab moves the focus as usual");
        for f in [open, closed] { assert!(f.matches(&key(egui::Key::ArrowLeft)) && f.matches(&key(egui::Key::ArrowUp))); }
    }
}

mod draw;
mod restore; // first frame / space switch
mod bar; // toolbar row + send pill
mod actions;
