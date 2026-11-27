//! Keeps the data API's store (extras/data/) up to date from the window's state, only while a plugin uses it.
//!
//! Two inputs say what changed: the answers of the cloud thread (`data_marks`, one per `Resp` in `App::poll`) and a cheap
//! fingerprint of what the UI changes by itself (`data_fingerprint`: the open page and space, unread / favourite / hidden
//! flags, drafts, flags, settings, the call controls, toasts, plugins). A frame where neither changed does no store work.
//! Not followed live: typing indicators that expire without a "stopped typing" event (updated with the next change).
use crate::extras::data::build::{AppView, CallView, NotifView, SpaceExt, TeamExt};
use crate::extras::data::model::Kind;
use crate::extras::data::{Fingerprint, Marks, Source};
use crate::cloud::locus::CallState;
use crate::*;
use std::hash::{Hash, Hasher};

fn page(t: Tab) -> &'static str {
    match t {
        Tab::Messaging => "messaging", Tab::Meetings => "meetings", Tab::Calling => "calling", Tab::Whiteboards => "whiteboards",
        Tab::Contacts => "contacts", Tab::Teams => "teams", Tab::Voicemail => "voicemail", Tab::Settings => "settings",
    }
}
fn state_name(s: &CallState) -> &'static str {
    match s {
        CallState::Dialing => "dialing", CallState::Ringing => "ringing", CallState::Incoming => "incoming", CallState::Connecting => "connecting",
        CallState::Connected => "connected", CallState::Ended(_) => "ended",
    }
}
fn notify_name(n: crate::cloud::spaces_ext::Notify) -> &'static str {
    use crate::cloud::spaces_ext::Notify;
    match n { Notify::Global => "global", Notify::All => "all", Notify::Mentions => "mentions", Notify::Off => "off" }
}
fn hash(f: impl FnOnce(&mut std::collections::hash_map::DefaultHasher)) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    f(&mut h);
    h.finish()
}

impl App {
    /// What an answer of the cloud thread changes in the store.
    pub(crate) fn data_marks(&self, r: &Resp) -> Marks {
        use Kind::*;
        let msgs = |id: &str| Marks::of(&[Space, Person, Presence, Team, App]).space(id);
        match r {
            Resp::Status(_) | Resp::AuthFailed(_) => Marks::of(&[App]),
            Resp::Device(_) => Marks::of(&[Me]),
            Resp::Me(_) => { let mut m = Marks::of(&[Me, Person, Presence, Space, Team, App]); m.all_messages = true; m }
            Resp::Spaces(_) | Resp::List(_) => Marks::of(&[Space, Team, Person, Presence, App, Meeting, CallRecord, Call]),
            Resp::Teams(_) => Marks::of(&[Team]),
            Resp::People(_) | Resp::Directory(..) => Marks::of(&[Person, Presence, Space]),
            Resp::Meetings(_) | Resp::Calendar(_) => Marks::of(&[Meeting]),
            Resp::Messages(id, _) | Resp::OlderMessages(id, ..) | Resp::Incoming(id, _) | Resp::ReadMarks(id, _) => msgs(id),
            Resp::Flagged(space, ..) => msgs(space),
            Resp::Flags(_) => { let mut m = Marks::default(); m.all_messages = true; m }
            Resp::Error(_) | Resp::Call(_) | Resp::CallShare(_) | Resp::RemoteShare(_) | Resp::CallVideo(_) | Resp::CallMute(_) | Resp::CallRoster(_) =>
                Marks::of(&[Call, App, Person, Presence]),
            Resp::Avatars(_) | Resp::Covers(_) | Resp::SpaceAvatars(_) | Resp::SpaceCovers(_) => Marks::of(&[Person, Space]),
            // message thumbnails and link-preview pictures: mostly of the open space
            Resp::Images(_) => self.selected.as_deref().map_or_else(Marks::default, |s| Marks::default().space(s)),
            Resp::Presence(_) | Resp::PresenceDetail(_) => Marks::of(&[Presence, Me]),
            Resp::CallHistory(_) => Marks::of(&[CallRecord]),
            Resp::SpaceEvent(_) => Marks::of(&[Space, Team, Person, Presence]),
            Resp::Typing { space, .. } => { let _ = space; Marks::of(&[Space]) }
            Resp::Cover(_) => Marks::of(&[Person]),
            Resp::MessagesFailed(_) | Resp::Scheduled(_) | Resp::PersonalRoom(_) => Marks::default(),
        }
    }

    /// A cheap summary of the state the UI changes without an answer of the cloud thread.
    pub(crate) fn data_fingerprint(&self) -> Fingerprint {
        let c = &self.call;
        Fingerprint {
            app: hash(|h| { (self.tab as u8).hash(h); self.selected.hash(h); self.team_sel.hash(h); self.status.hash(h); self.need_login.hash(h); self.client.is_some().hash(h); }),
            spaces: hash(|h| { self.spaces.len().hash(h); for s in &self.spaces { (s.unread, s.favorite, s.hidden).hash(h); } }),
            drafts: hash(|h| { self.drafts.len().hash(h); for (k, v) in &self.drafts { (k.len(), v.len()).hash(h); } }),
            flags: self.flags.len() as u64,
            settings: crate::extras::data::settings_generation(),
            call: hash(|h| {
                c.cur.as_ref().map(|x| (state_name(&x.state), x.since.to_bits(), x.ended_at.is_some())).hash(h);
                (c.muted, c.sharing, c.share_pending, c.remote_share.is_some(), c.video.wanted, c.video.sending, c.video.remote, &c.video.camera).hash(h);
            }),
            notify: self.notify.generation(),
            plugins: hash(|h| { for e in &self.plugins.list { (e.enabled, e.error.is_some(), e.version.len()).hash(h); } }),
        }
    }

    /// Updates the store (when a plugin uses it and something changed) and hands the subscriptions' changes to the plugins.
    pub(crate) fn data_frame(&mut self, marks: Marks, now: f64) {
        let hub = self.plugins.hub.clone();
        if !hub.active() { return; }
        let Some(m) = hub.plan(marks, self.data_fingerprint()) else { return };
        let dir = hub.directory();
        let now_ms = crate::i18n::now_ms();
        let has_photo = |k: &str| self.photos.has_photo(k);
        let has_image = |k: &str| self.photos.image_bytes(k).is_some();
        let space_ext = |id: &str| crate::ui::spaces_ext::with(|x| SpaceExt {
            notify: notify_name(x.notify.get(id).copied().unwrap_or_default()), locked: x.locked.contains(id),
            moderators: x.moderators.get(id).map(|s| s.iter().cloned().collect()).unwrap_or_default(), people: x.people.get(id).cloned(),
        });
        let team_ext = |id: &str| crate::ui::spaces_ext::with(|x| x.teams.get(id).map(|t| TeamExt { general: t.general.clone(), archived: t.archived }));
        let typers = |id: &str| crate::ui::typing::typers(id);
        let c = &self.call;
        let call = c.cur.as_ref().filter(|x| x.ended_at.is_none()).map(|x| CallView {
            id: format!("call:{}:{}", x.email, (x.since * 1000.0) as i64), state: state_name(&x.state), incoming: x.state == CallState::Incoming,
            name: x.name.clone(), email: x.email.clone(),
            started_ms: (x.state == CallState::Connected).then(|| now_ms - ((now - x.since).max(0.0) * 1000.0) as i64),
            roster: c.roster.clone(), muted: c.muted || c.mute.local, muted_by_host: c.mute.me.remote, unmute_allowed: c.mute.me.unmute_allowed,
            asked_to_unmute: c.mute.me.unmute_requested, video: c.video.wanted, video_sending: c.video.sending, sharing: c.sharing,
            camera: c.video.camera.clone(), video_size: c.video.size, video_fps: c.video.fps, remote_video: c.video.remote,
            remote_share: c.remote_share.clone(),
        });
        let notifications = self.notify.data_view(now_ms).into_iter()
            .map(|(id, kind, at_ms, space, title, body)| NotifView { id, kind, at_ms, space, title, body }).collect();
        let src = Source {
            me: &self.me, spaces: &self.spaces, teams: &self.teams, people: &self.people, meetings: &self.meetings, messages: &self.messages,
            settings: &self.settings, drafts: &self.drafts, flags: &self.flags, last_active: &self.photos.last_active,
            presence_info: &self.photos.presence_info, outside_status: &self.photos.outside_status, read_marks: &self.photos.read_marks,
            directory: &dir, has_photo: &has_photo, has_image: &has_image, space_ext: &space_ext, team_ext: &team_ext, typers: &typers,
            call, call_records: c.mc.records.as_deref(), calendar: c.mc.events.as_deref(), notifications,
            app: AppView { page: page(self.tab), selected: self.selected.clone(), team: self.team_sel.clone(), connected: self.client.is_some(),
                signed_in: !self.need_login, status: self.status.clone(), call_active: c.cur.as_ref().is_some_and(|x| x.ended_at.is_none()) },
            plugins: self.plugins.data_view(), my_cover: self.cover_bytes.is_some(), now_ms,
        };
        hub.apply(&m, &src);
        let ev = hub.take_events();
        if !ev.is_empty() { self.plugins.deliver_data(ev); }
    }
}
