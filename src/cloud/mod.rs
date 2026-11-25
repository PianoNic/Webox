//! Webex's own cloud protocol, in the order a session needs it:
//! service catalog (u2c) → device registration (wdm) → real-time socket (mercury) →
//! key-service handshake (encryption / kms ECDHE) → conversations and activities whose
//! text is end-to-end encrypted (JWE dir + A256GCM). Runs on background threads.
pub mod auth; // native sign-in (OAuth) and token refresh
pub mod jose;
pub mod secrets;
pub mod tls; // shared rustls config (bundled + system roots) and proxy handling for every HTTP agent and the websocket
pub mod richtext; // markdown → the HTML Webex sends (`object.content`)

mod activities;
pub mod features; // feature toggles + client policies (App::feature)
mod images;
pub mod locus;
pub mod media;
pub mod media_stats; // call statistics: packet counters, receiver reports and get_stats for the call window's panel
pub mod share_rx; // a screen share received in a call: decode, newest picture for the call window
pub mod video_rx; // camera video in a call: the far end's picture and our self view
mod sdp; // answer filtering and address-free summaries for the media leg
mod sdp_compat; // remote descriptions rewritten into what webrtc-rs accepts
mod ice_socket; // the call's own ICE socket (survives Windows' ICMP errors)
mod backoff; // 429 / 503: Retry-After and exponential backoff per host
mod report; // which job errors the user sees, in which words
mod mercury;
mod people;
mod sample;
mod space_ops;
mod header_plus;
mod presence_set;
pub use presence_set::{ACTIVE_TTL, NO_END_TTL, PRESENCE_FEATURE};
pub mod keepalive; // my own "Active" heartbeat
pub mod live_list; // the space list kept live from activities (read elsewhere, new conversations, changes elsewhere)
mod presence_sub; // presence of people outside the loaded spaces
pub mod typing; // typing indicators, both directions
mod polls; // periodic refreshes (running / upcoming meetings)
mod files; // uploads (share), downloads
pub use files::SendFiles;
mod msg_ops;
pub use msg_ops::quote_preview_html;
pub mod meetcall;
pub mod spaces_ext; // read state, notifications, moderators, teams (cloud/spaces_ext/)
pub use activities::REACTIONS;
use mercury::mercury;
pub use sample::{sample, sample_messages};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

pub const IDENTITY_ME: &str = "https://identity.webex.com/identity/scim/v1/Users/me";
pub const U2C_CATALOG: &str = "https://u2c-a.wbx2.com/u2c/api/v1/catalog?format=hostmap";
pub const ACTIVITY_PAGE: u32 = 50;
pub const CONVERSATION_PAGE: u32 = 100;
pub const KMS_TIMEOUT_SECS: u64 = 15;
/// Mercury ping interval in seconds (Webex pongs arrive every 22–25 s: this timer plus the round trip).
pub const MERCURY_PING_SECS: u64 = 22;

#[derive(Clone, Debug, Default)]
pub struct Space {
    pub id: String, pub title: String, pub direct: bool, pub team: String, pub last: String,
    pub unread: bool, pub favorite: bool, pub public: bool, pub key_url: String, pub participants: Vec<Person>,
    pub kms_resource: String, pub hidden: bool,
    /// ISO time of the last activity I have seen in this space (conversation `lastSeenActivityDate`); the "Neue Nachrichten" line sits after it.
    pub last_seen: String,
    /// Read-only state "conversation closed by policy" ("This conversation is closed" + "This space is no longer active
    /// because communication between certain groups is prohibited by policy." + "Check for updates"): no composer, banner
    /// buttons disabled, no Profile tab. Set by `closed_direct` (rules 2 + 3, see there). See ui/closed.rs.
    pub closed: bool,
    /// Announcement state: a group space in announcement mode in which I am not a moderator; no composer.
    /// Set by `announcement_blocked`. See ui/closed.rs.
    pub announcement: bool,
    /// The space's meeting (locus) url, `locusUrl` of the conversation; empty until the space ever had a meeting.
    pub locus_url: String,
    /// A meeting runs in this group space right now (its locus is active with someone joined): the header shows "Beitreten".
    pub meeting: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Message {
    pub id: String, pub author: String, pub email: String, pub text: String, pub time: String, pub mine: bool,
    /// Day it was sent, "YYYY-MM-DD" (UTC), for the date separators.
    pub date: String,
    /// Thread root this message replies to (empty for top-level messages).
    pub parent: String, pub reactions: Vec<Reaction>, pub edited: bool,
    /// Image attachments (non-image files stay as "📎 name" lines in `text`).
    #[allow(dead_code)]
    pub images: Vec<Attachment>,
    /// Link-preview cards shown above the text; `image_key` is a key for `App::image`/`image_frame`.
    pub previews: Vec<LinkPreview>,
    /// Non-empty: a centred system line instead of a message (e.g. "Jonas Weber war nicht erreichbar."); `time` is appended.
    pub notice: String,
    /// The message's rich text as sent (`object.content`, decrypted HTML: <a href>, <br>, <b>, <i>, <code>, lists, quotes,
    /// mentions). This is drawn, not the plain `text` (which has lost line breaks and keeps `%20` decoded).
    /// Empty when the sender used plain text only.
    pub html: String,
    // The non-image files of a share (one "📎 name" line each stays in `text`), in that order; `key` names the
    // file for `Req::DownloadFile` (cloud/files/ keeps its scr).
    pub files: Vec<Attachment>,
    /// `published` in Unix ms (0 = unknown, e.g. offline samples): a message is grouped under the previous one only
    /// within 60 s of it (see ui/messages/ `grouped_in_time`).
    pub published_ms: i64,
    /// Non-empty on a collapsed join/leave row (see cloud/activities/ `collapse_join_leave`): the people it stands for,
    /// listed in its popover.
    pub group: Vec<Person>,
    /// `date` / `time` as received (UTC), once the UI has turned `date` / `time` into local ones on arrival
    /// (ui/messages `prepare_arrived`); `None` = not localised (offline samples, local copies).
    pub utc: Option<(String, String)>,
    /// Content revision for the message list's layout cache: a hash of what the list draws of this message, stamped on
    /// arrival; 0 = unknown (the list then hashes the content itself). Whatever edits a message in place sets it to 0.
    pub rev: u64,
    /// Non-zero: a deleted message a plugin keeps showing (`message.deleted`, extras/plugins/kept.rs): 1 = red text,
    /// 2 = red overlay, 3 = no highlight. Local only.
    pub deleted: u8,
    /// A kept deleted message the plugin asked to fold: the list shows "N gelöschte Nachrichten" for a run of them, a click
    /// unfolds them. Local only.
    pub collapsed: bool,
    /// Earlier texts of an edited message, oldest first, that a plugin attached (`message.edited`), each as spans (text,
    /// `kept::SPAN_*`): a plain earlier text is one `SPAN_OLD` span, drawn struck through; a diff colours added / removed
    /// parts. Drawn above the text. Local only.
    pub history: Vec<Vec<Span>>,
    /// The current text as spans with its added parts marked (a plugin's diff view): drawn in those colours when it is the
    /// text Webox shows. Local only.
    pub current: Vec<Span>,
    /// A plugin logs this message's edits: "(bearbeitet)" stands below it even without earlier texts drawn, and a click on
    /// it fires `message.edit_tag_clicked`. Local only.
    pub edit_tag: bool,
}
/// A piece of plugin-given text: (text, kind), kind one of `extras::plugins::kept::SPAN_*`.
pub type Span = (String, u8);
/// A link preview: page title, description, site name and a thumbnail (OpenGraph data).
#[derive(Clone, Debug, Default)]
pub struct LinkPreview { pub url: String, pub title: String, pub description: String, pub site: String, pub image_key: String }
/// An image attached to a message (shown inline). `key` is the cache key for `App::image(ctx, key)`; the
/// bytes arrive later as `Resp::Images`. `width`/`height` are the thumbnail's pixel size (0 if unknown).
#[allow(dead_code)] // read by the message view once it draws inline images
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attachment { pub name: String, pub mime: String, pub size: u64, pub key: String, pub width: u32, pub height: u32,
    /// Set for a GIF shared from Giphy: where it lives on Giphy's CDN.
    pub giphy: Option<Giphy> }
/// A shared Giphy GIF: the plain CDN URL (decrypted `scr.loc`), the Giphy id parsed from it and the title (the share's file name).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Giphy { pub id: String, pub url: String, pub title: String }
/// Who has read a space up to which message ("Gesehen von"). `person` is the participant id (as in `Person.id`) when it could
/// be resolved, else the actor's own id; `email` is lower-case. `activity` is the id of the last message that person
/// acknowledged, `time` its publish time (ISO) of the acknowledgement.
#[allow(dead_code)] // read by the message view for "Gesehen von"
#[derive(Clone, Debug, Default)]
pub struct ReadMark { pub person: String, pub email: String, pub activity: String, pub time: String }
#[derive(Clone, Debug, Default)]
pub struct Reaction { pub name: String, pub count: u32, pub mine: Option<String>,
    /// Who reacted, in the order the service lists them (the reactors popover).
    pub users: Vec<Reactor> }
/// One person behind a reaction: the participant id (uuid) and when they reacted (ISO, UTC; may be empty). `name` / `email`:
/// what the session knows about that uuid (the space's participants, actors of loaded activities, `Req::ReactorNames`); the
/// UI's people lists are keyed by the e-mail-style participant id, which a uuid never matches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reactor { pub id: String, pub published: String, pub name: String, pub email: String }
#[derive(Clone, Debug, Default)]
pub struct Person { pub id: String, pub name: String, pub email: String, pub status: String }
#[derive(Clone, Debug, Default)]
pub struct Team {
    pub id: String, pub name: String,
    /// The team's colour (`team.teamColor` from the conversation service), as (r, g, b); `None` if absent/unparseable.
    pub color: Option<(u8, u8, u8)>,
}

/// `teamColor` arrives as "#RRGGBB" or "#AARRGGBB" (alpha first, e.g. "#FE8216", "#FF2E8BE6").
pub fn parse_team_color(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.strip_prefix('#')?;
    let n = u32::from_str_radix(h, 16).ok()?;
    match h.len() { 6 | 8 => Some(((n >> 16) as u8, (n >> 8) as u8, n as u8)), _ => None }
}
#[derive(Clone, Debug, Default)]
pub struct Meeting { pub title: String, pub start: String, pub link: String }

/// What a `Req::Post` is: a new message, a reply in the thread of a root message, or an edit of one of my messages.
pub enum PostKind { New, Reply(String), Edit(String) }

pub enum Req {
    Spaces, Messages(String), Send(String, String), Meetings,
    /// (space, message)
    Delete(String, String), Ack(String, String),
    /// (space, message, reaction name); Unreact takes the reaction's own id
    React(String, String, String), Unreact(String, String),
    /// (space, message): the names of the people who reacted to it (read-only; their reaction activities name the actor). Asked
    /// by the reactors popover when a reactor isn't known; answered by folding the space again (`Resp::Messages`).
    ReactorNames(String, String),
    /// Load the original file behind an inline image thumbnail (`Attachment.key`); arrives as `Resp::Images` key `full:<key>`.
    FullImage(String),
    /// Load the page of activities before the oldest one known in this space (history); answered with `Resp::OlderMessages`.
    MessagesBefore(String),
    Favorite(String, bool), Hide(String), Leave(String), Rename(String, String), AddPerson(String, String),
    /// Calling (Locus): dial / hang up / decline / audit (see `locus.rs`).
    Call(locus::CallReq),
    // The one-to-one space with `who` (uuid / Person.id / e-mail): existing one or a new one, then
    // `text` in it; `invite` = an unknown e-mail address gets a Webex account (shouldCreateUsers). WRITE, user click only.
    Direct { who: String, text: String, invite: bool },
    // A group space from "Neue Nachricht" (several recipients, optional name), then `text`. WRITE, user click only.
    Group { title: Option<String>, who: Vec<String>, text: String },
    // People search in the directory (read-only); answered with `Resp::Directory`.
    DirectorySearch(String),
    /// Share a Giphy GIF in `space` (picked in the composer's GIF picker). Not implemented yet: the handler answers with an
    /// error toast. The intended payload is a `share` activity whose file item is
    /// `{objectType:"file", displayName: <title>, mimeType:"image/gif", url:"https://giphy.com", image:{width,height}, scr:{loc: url}}`
    /// (the received format is described in `cloud/images.rs`): `url` = Giphy `downsized_medium` media URL (https, *.giphy.com),
    /// `title` = Giphy title, `width`/`height` = pixel size of that rendition. The GIF is not encrypted and not uploaded.
    SendGif { space: String, url: String, title: String, width: u32, height: u32 },
    /// A message as Webex sends it (see `richtext.rs`): `markdown` interprets the markdown syntax (the composer's
    /// markdown mode, or its rich toolbar's markers), `mentions` are the `@name`s typed via the popup (ids = `Person.id`).
    Post { space: String, text: String, markdown: bool, mentions: Vec<crate::richtext::MentionRef>, kind: PostKind },
    /// Set MY presence (`status` as the UI names it: active / inactive / DoNotDisturb) for `ttl` seconds (-1 = no end); WRITE,
    /// only on the user's click in the status menu. See cloud/presence_set.rs.
    SetPresence { status: String, ttl: i64 },
    /// Set one of MY user feature toggles (`POST feature /features/users/<me>/user`); WRITE, only on the user's click in Settings.
    SetFeature { key: String, val: Value },
    /// Upload files (encrypted) and share them in a space with the composer's text (WRITE; only on the user's Send). Progress and
    /// the result are reported to `crate::app::files` under `upload` (see cloud/files/).
    SendFiles(SendFiles),
    /// Download + decrypt the file behind an attachment key into `path`, then open it / show it in its folder.
    DownloadFile { key: String, path: std::path::PathBuf, then: crate::app::files::Then },
    // cloud/msg_ops.rs
    /// Flag (`on`) or unflag (PUT on `flag_url`) a message (userApps flags). WRITE.
    Flag { space: String, msg: String, on: bool, flag_url: String },
    /// Load my flags (GET userApps /flags?state=flagged) -> `Resp::Flags`. Read-only.
    Flags,
    /// "Als ungelesen markieren" from a message (verb "unacknowledge"). WRITE.
    MarkUnread { space: String, msg: String },
    /// A message that quotes another one (`<blockquote class='messageQuote'>`), new or as a thread reply (`parent`). WRITE.
    PostQuote { space: String, text: String, markdown: bool, mentions: Vec<crate::richtext::MentionRef>, quoted_html: String, quoted_text: String, author: String, author_email: String, parent: Option<String> },
    // cloud/meetcall.rs
    /// Read the call history (janus userSessions).
    CallHistory,
    /// WRITE: delete call-history entries `(sessionId, endTime ms)` (janus markAsDeleted).
    DeleteCalls(Vec<(String, i64)>),
    /// Read the calendar's meetings (calendar service calendarEvents).
    Calendar,
    /// WRITE: schedule a meeting (scheduler dialog's "Ansetzen").
    Schedule(meetcall::ScheduleReq),
    /// Read the user's personal-room link ("Meeting starten").
    PersonalRoom,
    // Every further space / people / team action (see cloud/spaces_ext/); WRITE ops only on the user's click.
    SpaceOp(spaces_ext::SpaceOp),
    /// The sign-in was renewed: the running session takes the new bearer in place (requests and the next
    /// socket authorisation), and renews its key-service token from `kms` (refresh token, idbroker base),
    /// without re-registering the device or reconnecting.
    Token { access: String, kms: Option<(String, String)> },
    /// I am typing in this space (the UI sends at most one per 5 s, see cloud/typing.rs). WRITE, the user's own typing.
    Typing(String),
}
pub enum Resp {
    Status(String), Device(String), Me(Person), Spaces(Vec<Space>), Teams(Vec<Team>), People(Vec<Person>),
    Messages(String, Vec<Message>), /** the first page of a space could not be loaded (the list shows its retry item) */ MessagesFailed(String), Incoming(String, Message), Meetings(Vec<Meeting>), Error(String),
    /// The login was refused (expired or wrong token): the window goes back to the sign-in screen.
    AuthFailed(String),
    /// The cover picture for conversation headers (bytes of a PNG/JPEG from the avatar service).
    Cover(Vec<u8>),
    /// Profile pictures (PNG/JPEG bytes) keyed by person id, uuid and lower-case email. May arrive in several batches.
    Avatars(Vec<(String, Vec<u8>)>),
    /// Cover pictures (PNG/JPEG bytes) of other people, keyed like `Avatars` (person id, uuid, lower-case email). Sent when a
    /// direct space's messages are loaded, for the other participant, if that person chose a cover. The key `"@default"` holds
    /// the global default cover (device policy `cover-image-selection`), sent once after sign-in.
    Covers(Vec<(String, Vec<u8>)>),
    /// The complete current read marks of one space (everyone but me), re-sent whenever someone acknowledges live.
    ReadMarks(String, Vec<ReadMark>),
    /// The custom cover picture (PNG/JPEG) of a group space that has one (`(space id, bytes)`). Spaces without a custom cover
    /// have no banner; they are never sent. Sent when the space's messages load.
    SpaceCovers(Vec<(String, Vec<u8>)>),
    /// The round picture of a space that has one (`(space id, bytes)`), sent after the space list loads.
    SpaceAvatars(Vec<(String, Vec<u8>)>),
    /// Decrypted inline-image bytes (PNG/JPEG/…) keyed by `Attachment.key`.
    Images(Vec<(String, Vec<u8>)>),
    /// Presence as (person id or my email, status string for `presence_text()`, last-active time in Unix ms or 0).
    /// Sent once after the spaces load and again for every live update.
    Presence(Vec<(String, String, i64)>),
    /// The rest of a person's presence line: (person id or my email, end of the status in Unix ms (DND/meeting/out-of-office
    /// "until …"), custom-status / out-of-office text). Sent with every `Presence` snapshot and live update; an entry with
    /// `None` and an empty text clears what was known.
    PresenceDetail(Vec<(String, Option<i64>, String)>),
    /// History: the space's complete folded message list after an older page was merged in, how many messages that put in
    /// front of the previous first message (to keep the scroll position), and whether the start of the chat was reached.
    OlderMessages(String, Vec<Message>, usize, bool),
    /// A change of the current call (outgoing ringing, incoming, connected, ended).
    Call(locus::CallEvent),
    /// Our screen share in the call started (true) or ended (false).
    CallShare(bool),
    /// Mute in the call changed: the other side's microphone, ours as Locus has it, ours here.
    CallMute(locus::MuteInfo),
    /// The call's participants for the roster (sent when it changes).
    CallRoster(locus::controls::Roster),
    /// The other side shares its screen (their name), or stopped (None). The pictures: `share_rx::latest`.
    RemoteShare(Option<String>),
    /// Camera video in the call changed: ours (wanted / sending, which camera, size, rate) and the other side's camera on
    /// or off. The pictures: `video_rx::remote` / `video_rx::self_view`.
    CallVideo(locus::video::VideoInfo),
    /// Directory people for a search query (query, people, last batch).
    Directory(String, Vec<Person>, bool),
    /// All my flags: (space id or "" when the flag doesn't say, message id, flag url).
    Flags(Vec<(String, String, String)>),
    /// One flag changed: (space, message, Some(flag url) = flagged / None = unflagged).
    Flagged(String, String, Option<String>),
    CallHistory(Vec<meetcall::CallRecord>),
    Calendar(Vec<meetcall::CalEvent>),
    /// Scheduling result: the join link, or the server's reason.
    Scheduled(Result<String, String>),
    PersonalRoom(Result<String, String>),
    // Notification settings / moderators / team facts / full people list / team spaces (cloud/spaces_ext/).
    SpaceEvent(spaces_ext::SpaceEvent),
    /// One live change of the space list (cloud/live_list/ → app/live_list.rs).
    List(live_list::ListPatch),
    /// Someone started (`on`) / stopped typing in a space (cloud/typing.rs → ui/typing.rs).
    Typing { space: String, person: Person, on: bool },
}
enum Job { Ui(Req), Activity(Value), Presence(Value), Locus(Value),
    /// A Mercury `directory.search` event.
    Directory(Value),
    /// The socket is back after a drop (Unix ms when it went down): fetch what was missed (`Session::catchup`).
    Catchup(i64),
    /// A Mercury `status.start_typing` / `status.stop_typing` event (cloud/typing.rs).
    Typing(Value),
    /// An activity event's `data.conversation.registrationTTLMs`: (conversation, ms).
    TypingTtl(String, i64) }

/// Which `start()` is current. Background threads of an earlier session (socket, job loop) end when it
/// changes, so a sign-out / new sign-in never leaves a second socket of the same device behind.
static SESSION_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The bearer of the current session (renewed in place by `Req::Token`); the socket reads it on every
/// (re)connect.
static LIVE_TOKEN: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());
pub(super) fn session_current(gen: u64) -> bool { SESSION_GEN.load(std::sync::atomic::Ordering::SeqCst) == gen }
pub(super) fn live_token() -> String { LIVE_TOKEN.read().map(|t| t.clone()).unwrap_or_default() }
/// Seconds between re-POSTs of the usersub presence subscription: apheleia subscriptions live 10 min and are renewed
/// 1 min before they expire.
pub const PRESENCE_RENEW_SECS: u64 = 540;

/// Whether the renewal planned for `at` (Unix ms) is due at `now`. Wall-clock time, like the desktop client's expiry check:
/// time asleep counts. A clock set back by more than one period counts as due too, so the renewal never waits for hours.
pub fn presence_renew_due(at: i64, now: i64) -> bool { now >= at || at - now > PRESENCE_RENEW_SECS as i64 * 1000 }

pub struct Client { tx: Sender<Job>, pub rx: Receiver<Resp> }
impl Client { pub fn req(&self, r: Req) { let _ = self.tx.send(Job::Ui(r)); } }

struct Kms { cluster: String, kid: String, rsa: rsa::RsaPublicKey, eph: Option<(String, [u8; 32])> }

struct Session {
    agent: ureq::Agent, token: String, my_email: String, kms_token: String, kms_loop: Sender<String>, svc: HashMap<String, String>, device: String, user: String,
    kms: Option<Kms>, keys: HashMap<String, Vec<u8>>, kms_rx: Receiver<String>,
    /// Participant id (email-style) → Webex uuid, which the avatar and presence services want.
    uuids: HashMap<String, String>, avatars_done: std::collections::HashSet<String>,
    hist: activities::History,
    space_cover_urls: HashMap<String, String>, space_covers_done: std::collections::HashSet<String>, full_desc: HashMap<String, images::FullDesc>, read_marks: HashMap<String, HashMap<String, ReadMark>>, covers_done: std::collections::HashSet<String>, oracle_done: bool, img_jobs: Vec<images::ImgJob>, images_done: std::collections::HashSet<String>,
    spaces: HashMap<String, Space>, out: Sender<Resp>, cover: String, calls: locus::Calls,
    /// My own Active keepalive (cloud/keepalive.rs).
    keep: keepalive::Keepalive,
    /// Live list state (cloud/live_list/).
    live: live_list::Live,
    /// People followed outside the loaded spaces (cloud/presence_sub.rs).
    extra: presence_sub::Extra,
    /// Typing registration and conversation urls (cloud/typing.rs).
    typing: typing::TypingState,
    /// Whose presence subscription runs until when (cloud/people/subs.rs).
    subs: people::Subs,
}

/// A direct chat's title: the other participant's name; when nobody else is left in `participants` (a ONE_ON_ONE chat
/// can have only me as participant), the conversation's own displayName; failing that "Untitled" ("Unbenannt").
/// Never empty, so the avatar never falls back to '#'.
pub(crate) fn direct_title(other: Option<&str>, own: &str, untitled: &str) -> String {
    [other.unwrap_or(""), own].iter().map(|s| s.trim()).find(|s| !s.is_empty()).unwrap_or(untitled).to_string()
}

/// Read-only "closed" state of a direct chat. Rules (first match wins, after the announcement rule):
///   2. a 1:1 conversation with the READ_ONLY flag (the conversation tag "READ_ONLY");
///   3. (feature-flagged by `left_company_rule`) a 1:1 conversation whose other participant was removed / deactivated,
///      i.e. nobody but me left in `participants`.
/// Closed meeting spaces, server-migration locks and meeting-participant restrictions are not modelled: Webox has
/// no meeting spaces and the data carries no migration lock.
pub(crate) fn closed_direct(direct: bool, read_only: bool, others: usize, left_company_rule: bool) -> bool { direct && (read_only || (left_company_rule && others == 0)) }
/// The rule-3 feature toggle of the signed-in account.
pub(crate) const LEFT_COMPANY_RULE: &str = "get-user-left-company-one-to-one-from-server-enabled";

/// Announcement state (rule 1, checked first): the conversation is in announcement mode (tag "ANNOUNCEMENT") and I am not
/// a moderator (`participants.items[].roomProperties.isModerator`). A direct chat is never treated as an announcement space.
pub(crate) fn announcement_blocked(direct: bool, announcement: bool, self_moderator: bool) -> bool { !direct && announcement && !self_moderator }

#[cfg(test)]
mod title_tests;

#[cfg(test)]
mod live;

mod start;
#[allow(unused_imports)]
pub(crate) use start::*;
mod session;
mod crypto;
mod lists;
