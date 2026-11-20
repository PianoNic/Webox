//! The message list of a space, laid out like the Webex desktop client with its MomentumDark / MomentumDefault theme tokens.
//! Layout: [reply bar] [avatar 16 + 34] [message box] with the text 8 pt (the frame radius) inside the box. The box is
//! transparent until the row is hovered (token message-background: normal #FFFFFF@0, hovered #FFFFFF@0.07). The hover
//! toolbar is 24 pt high with 32 pt per button. All sizes are in points.
use super::emoji_text;
use super::menu;
mod html;
use crate::*;
use cloud::{Attachment, LinkPreview, REACTIONS};
use egui::{epaint::Vertex, Align2, Galley, Id, Mesh, Shape, StrokeKind, TextureId};
use std::sync::Arc;

/// Replies shown under a thread root before it collapses into the "N Antworten" row.
pub(crate) const THREAD_PREVIEW: usize = 2;
/// The three favourite emoji of the hover toolbar: 👍 😀 🙌.
const QUICK: [(&str, &str); 3] = [("👍", "Daumen hoch"), ("😀", "Grinsendes Gesicht"), ("🙌", "Erhobene Hände")];

/// The reaction name sent for an emoji: the legacy name for the ones that have one (thumbsup, heart, ...), else the emoji itself.
fn reaction_name(ch: &str) -> String {
    let bare = |s: &str| s.replace('\u{fe0f}', "");
    REACTIONS.iter().find(|r| bare(r.1) == bare(ch)).map_or_else(|| ch.to_string(), |r| r.0.to_string())
}

/// The density table.
fn dm() -> &'static crate::ui::theme::density::Metrics { crate::ui::theme::density::m() }
// Geometry (pt; the functions are Standard density values from `dm()`).
const GRID_X: f32 = 1.0; // row content x
const AVATAR_LEFT: f32 = 16.0; // avatar left margin
const AVATAR: f32 = 32.0; // avatar size
const AVATAR_SLOT: f32 = 34.0; // avatar column width
const BOX_LEFT: f32 = GRID_X + AVATAR_LEFT + AVATAR_SLOT; // 51; the text starts RADIUS further in
const RADIUS: f32 = 8.0; // message frame radius
const RIGHT_MARGIN: f32 = 5.0; // message frame right margin
fn avatar_top() -> f32 { dm().msg_avatar_top } // avatar top margin (18)
/// Emoji in text are drawn this big (~1.15 x the font size; the layout reserves 1.2 em).
const EMOJI_SIZE: f32 = 17.0;
// Link-preview card: width min(620, available); side by side ("wide mode") when available >= 620, else the picture above
// the texts; picture box 200 x 105; spacing 5; text column at most 420 wide (wide mode); title top / site bottom margin 10.
const CARD_MAX_W: f32 = 620.0;
const CARD_THUMB_W: f32 = 200.0;
const CARD_THUMB_H: f32 = 105.0;
const CARD_SPACING: f32 = 5.0;
const CARD_TEXT_MAX_W: f32 = 420.0;
const CARD_MARGIN: f32 = 10.0;
const LB_MARGIN: f32 = 48.0; // image viewer margin around the picture
const BODY: f32 = 16.0; // body text size
const SECOND: f32 = 14.0; // secondary text size (names, times)
const HEADER_GAP: f32 = 12.0; // gap between name and time in the header
fn top_pad() -> f32 { dm().msg_top_pad } // row top -> first line (6)
fn header_to_body() -> f32 { dm().msg_header_to_body } // header line bottom -> first content (5.1)
/// egui sets Segoe UI 1.2 pt lower inside its line box than Qt does: body text is drawn that much higher so that header cap -> body
/// ascender is 24.0 pt while header cap -> image top stays 19.2 pt.
const TEXT_SHIFT: f32 = -1.2;
fn bottom_pad() -> f32 { dm().msg_bottom_pad } // last line -> next row (7.7; grouped single-line rows are 35 pt apart)
fn attach_gap() -> f32 { dm().msg_attach_gap } // gap between attachments (8)
const ATTACH_BOTTOM: f32 = 5.4; // below the attachments (image bottom -> next header cap = 23.2 pt)
const SMALL_IMG: f32 = 80.0; // smaller pictures keep their size
const IMG_MAX: f32 = 320.0; // largest inline image width / height
const IMG_RADIUS: f32 = 4.0; // inline image corner radius
const REACT_H: f32 = 22.0; // height of the reactions row
const REPLY_LEFT: f32 = 64.0; // left margin of thread replies
const REPLY_BAR: f32 = 4.0; // width of the thread reply bar
const TB_H: f32 = 24.0; // hover toolbar height
const TB_W: f32 = 32.0; // hover toolbar button width
/// Extra space between an attachment and the caption text below it (image bottom -> caption cap top = 12.8 pt).
const CAPTION_GAP: f32 = 2.6;
// Scrollbar: track 16 pt wide, thumb 7.6 pt, chevron buttons at both ends.
/// Space under the last list item ("Gesehen von" row centre -> composer line = 48 pt).
fn list_bottom() -> f32 { dm().msg_list_bottom } // 25.5
/// The offline sample's number of older pages.
const SAMPLE_PAGES: u32 = 2;
/// The scroll bar's width (the bar itself is the shared `autoscroll::arrow_bar`).
const SB_W: f32 = crate::ui::autoscroll::BAR_W;
/// Readers shown as avatars at most; the rest collapse into "+N".
const MAX_SEEN: usize = 10;
// Last line's top -> "Gesehen von" centre = 47.6 pt (SEEN_TOP + seen_h() / 2 = 23.4 in Standard).
const SEEN_TOP: f32 = -0.6;
// Height of the "Gesehen von" row; the label and avatars are centred in it (the avatars drawn in `seen_banner` are 22 pt).
fn seen_h() -> f32 { dm().seen_h } // 48
fn day_pad() -> f32 { dm().day_pad } // vertical padding of info rows (30)

/// Messages of one sender closer than this are grouped under one header.
const GROUP_MINUTES: i32 = 5;
/// An image that hasn't arrived after this many seconds is shown as failed instead of loading forever.
const IMG_FAIL_SECS: f64 = 45.0;

/// The message theme tokens of the active theme. Body text = message-bodyText-primary-text (white 95 %), names / times =
/// message-headerText-default-text (70 %).
struct Mc {
    body: Color32, head: Color32, hover_bg: Color32, accent: Color32, chip_border: Color32, chip_hover: Color32,
    ring: Color32, bubble: Color32, tile: Color32, tile_hover: Color32, tile_border: Color32, file_icon_bg: Color32,
    mention_other: Color32, code_bg: Color32, select: Color32, pill_bg: Color32, pill_hover: Color32, pill_border: Color32, day: Color32, bar: Color32, white: Color32, checked: Color32,
}
fn mc(t: &Th) -> Mc {
    let dark = !t.light;
    let f = if dark { Color32::WHITE } else { Color32::BLACK };
    Mc {
        body: t.tk("message-bodyText-primary-text"),
        head: t.tk("message-headerText-default-text"), // = message-bodyText-secondary-text
        hover_bg: wa(f, 0.07),      // message-background hovered
        accent: t.tk("message-bodyText-mention-self-text"), // = hyperlink-accent-text / reactions-count-has-self-reaction
        chip_border: wa(f, 0.5),    // dualbutton-standard-border
        chip_hover: wa(f, 0.07),    // dualbutton-standard-button-background hovered
        ring: t.tk("avatar-chat-bubble-border"),
        bubble: t.tk("avatar-chat-bubble-icon-normal"),
        tile: wa(f, 0.11),          // attachment-file-default-sent-background / attachment-unfurl-background / thumbnail background
        tile_hover: wa(f, 0.2),     // ... hovered
        tile_border: wa(f, 0.2),    // attachment-file-default-sent-border
        file_icon_bg: t.tk("attachment-file-default-sent-previewBox-background"),
        mention_other: t.tk("message-bodyText-mention-other-text"), code_bg: wa(f, 0.11), // inline code / pre
        // text-highlight is #064157 / #91EBFF in dark; light uses a 30 % blue (the selected text keeps its colour).
        select: if dark { t.tk("text-highlight") } else { wa(hex(0x1170CF), 0.3) },
        pill_bg: t.tk("wx-menuList-background"), // hover toolbar background
        pill_hover: t.tk("wx-menuList-background:hovered"),
        pill_border: t.tk("wx-default-border"),
        day: t.text2, // text-secondary
        bar: t.tk("legacy-separator-primary"), // reply bar
        white: t.tk("message-bodyText-primary-text"),
        checked: wa(f, 0.11), // message-background checked: #FFFFFF/0.11 dark, #000000/0.11 light
    }
}

/// An emoji of a message: where its placeholder starts (char index in the laid-out text) and which picture of the emoji set
/// to draw (`None`: the set doesn't have it and there's no system font - a box is drawn).
struct EmojiRun { char_idx: usize, picture: Option<String>, keycap: Option<char>, orig: String, size: f32 }

/// Builds the layout job of a message: ordinary text, links, and emoji (see emoji_text.rs).
struct JobBuilder { job: egui::text::LayoutJob, nchars: usize, runs: Vec<EmojiRun>, sys: bool, sysfont: FontId, st: Style }
/// Inline formatting of rich text (see messages/html.rs).
#[derive(Clone, Default)]
struct Style { bold: bool, italic: bool, underline: bool, strike: bool, code: bool, size: Option<f32>, bg: Option<Color32> }

/// What a message needs below its text, measured before painting.
/// `files` / `file_atts`: the file cards (names and their attachments); `videos`: the video files of the share, drawn as video
/// cards after the pictures (a video with a thumbnail comes in `Message.images` and is drawn there).
struct Content { text: Option<Arc<Galley>>, links: Vec<(std::ops::Range<usize>, String)>, emojis: Vec<EmojiRun>, quotes: Vec<(usize, usize)>, jump: Option<String>, files: Vec<String>, file_atts: Vec<Option<Attachment>>, videos: Vec<Attachment>, h: f32 }

#[cfg(test)]
mod tests;

mod helpers;
#[allow(unused_imports)]
pub(crate) use helpers::*;
mod text;
#[allow(unused_imports)]
pub(crate) use text::*;
mod list;
mod history;
mod new_banners;
mod jump_button;
pub(crate) use jump_button::*;
mod loading_item;
pub(crate) use loading_item::*;
mod multisel;
pub(crate) use multisel::*;
mod rows;
mod media;
mod video;
mod attachments;
use attachments::split_files;
pub(crate) use video::poster as video_poster;
use video::is_video;
mod interact;
