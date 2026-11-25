#![allow(clippy::too_many_arguments)]
use crate::*;

impl App {
    pub(crate) fn new(a: system::Args) -> Self {
        let cfg = system::config_dir();
        system::host_log(&cfg, &a);
        if a.screenshot.is_none() { platform::updater::after_start(); } // leftovers of an update, the installer's version entry
        if a.sample || a.screenshot.is_some() { platform::updater::NO_AUTO_CHECK.store(true, std::sync::atomic::Ordering::Relaxed); }
        let mut settings = system::load_settings(&cfg);
        crate::ui::customize::migrate_legacy(&mut settings); // the old App-Seitenleiste checkboxes
        let (me, spaces, teams, people, meetings) = cloud::sample();
        let mut plugins = extras::plugins::Plugins::new(&cfg);
        if a.sample { plugins.use_memory_secrets(); } // demo runs never touch the OS keyring
        let repos = extras::plugin_repo::Repos::new(&cfg, plugins.install_root.clone(), a.sample);
        let mut app = App {
            plugins, repos, cfg, settings, client: None, status: "Offline (sample data)".into(),
            me, spaces, teams, people, meetings, messages: HashMap::new(), tab: Tab::Messaging, list_tab: ListTab::All,
            filter: Filter::None, conv_tab: ConvTab::Messages, selected: None, team_sel: None, search: String::new(), focus_search: false,
            drafts: HashMap::new(), flags: HashSet::new(), collapsed: HashSet::new(), list_hidden: false, history: vec![], hpos: 0,
            icons: Default::default(), toasts: vec![], plugin_log: vec![], new_space: None, dial: String::new(),
            show_shortcuts: false, pending_space: a.protocol_uri.as_deref().and_then(system::space_from_uri),
            shot: a.screenshot, frames: 0, applied: None, theme_cache: None, theme_sel: None, last_input: 0.0, last_pointer: None, textures_released: false, minimize: a.minimized,
            need_login: false, login_error: String::new(), email_input: String::new(), code_input: String::new(),
            pkce_verifier: String::new(), pending: None, refreshed: false,
            login_page: LoginPage::Start, config_open: false, meeting_input: String::new(), login_sized: false,
            webview: None, hwnd: None, autosignin: a.signin,
            cover: None, cover_bytes: None, photos: Default::default(),
            compose: Compose::New, rename_input: None, add_people: false, people_filter: String::new(), open_threads: HashSet::new(), confirm_leave: None, forward_text: None, rail_wide: false, tour: (0, 0.0), fonts_set: false, row_salt: String::new(), row_draft: false, call: Default::default(),
            sc: Default::default(),
            notify: Default::default(),
            msgx: Default::default(),
        };
        // One-time key fixes (ui/settings); offline sample / screenshot runs change them in memory only.
        if ui::settings::migrate(&mut app.settings, ui::chrome::now_ms()) && !a.sample && app.shot.is_none() { system::save_settings(&app.cfg, &app.settings); }
        // Sample: the Calling tab was "last seen" an hour ago, so the made-up missed call of 25 minutes ago shows the rail badge.
        if a.sample { app.settings[ui::settings::CALLS_SEEN_MS] = json!(ui::chrome::now_ms() - 3_600_000); }
        app.msgx_load(); // scheduled messages saved by an earlier run
        extras::gif_extras::register(); // built-in plugin behind the GIF picker's hook (off unless switched on in Settings > Plugins)
        app.rail_wide = app.settings["rail_wide"].as_bool().unwrap_or(false);
        // A "webexusername" in the local configuration is used as the sign-in email; otherwise the last email typed.
        app.email_input = [app.settings["webexusername"].as_str(), app.settings["email"].as_str()].into_iter().flatten()
            .find(|s| !s.is_empty()).unwrap_or("").into();
        for s in &app.spaces { app.messages.insert(s.id.clone(), cloud::sample_messages(&s.id)); }
        // Sample-only extras so the People / Meetings tabs have something to show (offline demo data).
        if a.sample {
            let me = app.me.clone();
            if let Some(s) = app.spaces.iter_mut().find(|s| s.id == "s1") { s.participants = std::iter::once(me).chain(app.people.iter().filter(|p| p.name != "Jonas Weber").cloned()).collect(); }
            app.meetings.push(Meeting { title: "Alex's Webex space: Sync".into(), start: "16:00".into(), link: "https://example.com/meet/s1".into() });
        }
        if a.sample { ui::sample_scale::scale(&mut app); } // WEBOX_SAMPLE_SCALE: an account-sized sample for memory checks
        if a.sample { app.photos.add(ui::sample_photos::sample_photos()); if std::env::var("WEBOX_SHOT_LATE").is_err() { app.photos.add_images(ui::sample_photos::sample_images()); } app.cover_bytes = Some(ui::sample_photos::sample_banner(0)); app.photos.add_images(vec![("cover:@default".into(), ui::sample_photos::sample_banner(1)), ("cover:jonas@example.com".into(), ui::sample_photos::sample_banner(2)), ("spacecover:s6".into(), ui::sample_photos::sample_banner(0)), ("spaceavatar:s8".into(), ui::sample_photos::sample_banner(1))]); for (k, v) in ui::sample_photos::sample_last_active(ui::chrome::now_ms()) { app.photos.last_active.insert(k, v); } }
        // Startup state: nothing selected in the list, the content area shows the "Create a space" pane; no space is
        // opened, so none is marked read. Screenshot mode opens the first sample space so --sample checks stay comparable.
        if app.shot.is_some() { if let Some(id) = app.spaces.first().map(|s| s.id.clone()) { app.open_space(id) } } else { app.new_space = Some(String::new()); }
        if a.sample { if let Ok(id) = std::env::var("WEBOX_SHOT_SPACE") { app.open_space(id) } } // screenshot checks of another sample chat
        let token = if a.sample { String::new() } else { app.settings["token"].as_str().unwrap_or("").to_string() };
        if !token.is_empty() && app.renewal_due() && app.try_refresh() {
            app.status = "Renewing your sign-in…".into();
        } else if !token.is_empty() {
            app.sign_in(token);
        } else if app.settings["sample"] != true {
            app.need_login = true;
        }
        if a.sample { app.need_login = false; }
        app
    }

    pub(crate) fn toast(&mut self, t: impl Into<String>, now: f64) { self.toasts.push((t.into(), now + 5.0)); }

    pub(crate) fn poll(&mut self, now: f64) {
        // The time zone changed (checked once a minute): every received message gets its local date / time again.
        {
            static CHECKED: std::sync::Mutex<f64> = std::sync::Mutex::new(f64::NEG_INFINITY);
            let due = CHECKED.lock().map(|mut c| { let d = now - *c >= 60.0 || now < *c; if d { *c = now; } d }).unwrap_or(false);
            if due && self.client.is_some() && crate::ui::messages::zone_changed() {
                for l in self.messages.values_mut() { for m in l.iter_mut() { crate::ui::messages::relocalise(m); } }
            }
        }
        let mut got = vec![];
        if let Some(c) = &self.client { while let Ok(r) = c.rx.try_recv() { got.push(r); } }
        let changed = !got.is_empty();
        crate::platform::memprobe::RESPS.fetch_add(got.len(), std::sync::atomic::Ordering::Relaxed); // WEBOX_PROBE: cloud answers
        for r in got {
            match r {
                Resp::Status(s) => self.status = s,
                Resp::Device(d) => { self.settings["device"] = json!(d); system::save_settings(&self.cfg, &self.settings); }
                Resp::Me(p) => self.me = p,
                Resp::Spaces(s) => {
                    self.spaces = s;
                    // A space asked for by a webexteams:// link / toast opens; otherwise the list arrives with nothing selected (startup state).
                    if let Some(id) = self.pending_space.take() { self.open_space(id); }
                    // The selected space is gone (left / deleted / hidden elsewhere): nothing selected -> the create-space pane.
                    if self.selected.as_ref().is_some_and(|id| !self.spaces.iter().any(|s| &s.id == id)) { self.selected = None; }
                }
                Resp::Teams(t) => self.teams = t,
                Resp::People(p) => self.people = p,
                Resp::Directory(q, people, done) => {
                    // People in no loaded space: their status is followed from now on (cloud/presence_sub.rs).
                    for p in people.iter().filter(|p| !self.people.iter().any(|x| x.id == p.id)) { self.photos.outside_status.insert(p.id.clone(), p.status.clone()); }
                    crate::ui::new_message::directory_results(&q, people, done)
                }
                Resp::Meetings(m) => self.meetings = m,
                Resp::Messages(id, mut m) => {
                    self.list_arrived(&id, &mut m); // plugins: message.deleted / message.edited, kept messages
                    crate::ui::messages::prepare_arrived(&mut m); // local date / time once, layout revision
                    let open = self.selected.as_deref() == Some(id.as_str());
                    if let Some(h) = self.photos.history.get_mut(&id) { h.first_failed = false; }
                    if self.plugins.subscribed("message.received") { self.plugins.note_space(&id, &m); }
                    self.messages.insert(id.clone(), m);
                    if open { self.acknowledge(&id); }
                }
                Resp::MessagesFailed(id) => { if !self.messages.contains_key(&id) { self.photos.history.entry(id).or_default().first_failed = true; } }
                Resp::Incoming(id, mut m) => {
                    crate::ui::messages::prepare_arrived(std::slice::from_mut(&mut m));
                    let mine = m.mine;
                    if !mine { crate::ui::typing::message_from(&id, &m.email); } // their message ends their typing
                    // Main window not in front: OS toast + taskbar flash instead of the in-window toast.
                    let os_toast = !mine && self.notify_incoming(&id, &m);
                    if !mine && self.selected.as_deref() != Some(&id) {
                        self.message_sound(&id);
                        let shown = self.plugins.on_message(&m.author, &m.text);
                        if !os_toast { self.toast(format!("{}: {}", m.author, shown), now); }
                    }
                    if let Some(i) = self.spaces.iter().position(|s| s.id == id) {
                        let mut s = self.spaces.remove(i);
                        s.unread = !mine && self.selected.as_deref() != Some(&id);
                        self.spaces.insert(0, s);
                    }
                    // A send reloads the whole list (`Resp::Messages`, which already holds the new message) and the live event of the
                    // same activity arrives as well: the same id must not be added twice.
                    let open = self.selected.as_deref() == Some(id.as_str()); // read receipt for a live message in the open chat
                    // Plugins: `message.received` with its space, once per id (`message_received` drops repeats).
                    let hook = (self.plugins.subscribed("message.received") && m.notice.is_empty()).then(|| {
                        let direct = self.spaces.iter().any(|s| s.id == id && s.direct);
                        (m.id.clone(), m.author.clone(), m.text.clone(), direct, !mine && mentions_me(&m, &self.me))
                    });
                    let list = self.messages.entry(id.clone()).or_default();
                    if !list.iter().any(|x| x.id == m.id) { list.push(m); }
                    if let Some((mid, author, text, direct, mentioned)) = hook {
                        self.plugins.note_space(&id, list);
                        self.plugins.message_received(&id, &mid, &author, &text, mine, direct, mentioned);
                    }
                    if open && !mine { self.acknowledge(&id); }
                }
                Resp::Error(e) => { self.call_failed(); self.toast(e, now) }
                Resp::Call(ev) => self.on_call_event(ev, now),
                Resp::CallShare(on) => { self.call.sharing = on; self.call.share_pending = false; }
                Resp::RemoteShare(who) => self.on_remote_share(who),
                Resp::CallVideo(v) => self.call.video = v,
                Resp::CallMute(m) => self.on_call_mute(m, now),
                Resp::CallRoster(r) => self.call.roster = r,
                Resp::Cover(b) => { self.cover_bytes = Some(b); self.cover = None; }
                Resp::Avatars(v) => self.photos.add(v),
                Resp::Images(v) => self.photos.add_images(v),
                Resp::SpaceAvatars(v) => self.photos.add_images(v.into_iter().map(|(k, b)| (format!("spaceavatar:{k}"), b)).collect()),
                Resp::SpaceCovers(v) => self.photos.add_images(v.into_iter().map(|(k, b)| (format!("spacecover:{k}"), b)).collect()),
                Resp::ReadMarks(space, v) => { self.photos.read_marks.insert(space, v); }
                Resp::Covers(v) => self.photos.add_images(v.into_iter().map(|(k, b)| (format!("cover:{k}"), b)).collect()),
                Resp::Presence(v) => for (id, status, last) in v {
                    if last > 0 { self.photos.last_active.insert(id.clone(), last); }
                    if self.photos.outside_status.contains_key(&id) { self.photos.outside_status.insert(id.clone(), status.clone()); }
                    for p in self.people.iter_mut().filter(|p| p.id == id) { p.status = status.clone(); }
                    for s in &mut self.spaces { for p in s.participants.iter_mut().filter(|p| p.id == id) { p.status = status.clone(); } }
                    if self.me.id == id || self.me.email.eq_ignore_ascii_case(&id) { cloud::keepalive::note_self_status(&status); self.me.status = status.clone(); }
                },
                // An older page arrived: the full list replaces the loaded one (reactions merged both ways).
                Resp::OlderMessages(space, mut list, prepended, reached) => {
                    self.list_arrived(&space, &mut list);
                    crate::ui::messages::prepare_arrived(&mut list);
                    let h = self.photos.history.entry(space.clone()).or_default();
                    h.requested = None; h.reached_start = reached; h.last_prepended = prepended; h.generation += 1;
                    if self.plugins.subscribed("message.received") { self.plugins.note_space(&space, &list); }
                    self.messages.insert(space, list);
                }
                // End time + custom/out-of-office text of a person's presence line (see presence.rs).
                Resp::PresenceDetail(v) => for (id, until, text) in v {
                    if until.is_none() && text.is_empty() { self.photos.presence_info.remove(&id); } else { self.photos.presence_info.insert(id, (until, text)); }
                },
                Resp::AuthFailed(e) => {
                    self.client = None;
                    if !self.try_refresh() { self.login_error = e; self.need_login = true; }
                }
                Resp::Flags(v) => self.on_flags(v),
                Resp::Flagged(space, msg, url) => self.on_flagged(space, msg, url),
                r @ (Resp::CallHistory(_) | Resp::Calendar(_) | Resp::Scheduled(_) | Resp::PersonalRoom(_)) => self.on_meetcall(r, now),
                Resp::SpaceEvent(e) => self.on_space_event(e), // ui/spaces_ext/
                Resp::List(p) => self.on_list_patch(p), // app/live_list.rs
                Resp::Typing { space, person, on } => crate::ui::typing::on_typing(space, person, on),
            }
        }
        self.plugins.sync_language(); // plugins' translations after a language change (extras/plugins/i18n.rs)
        let out = std::mem::take(&mut *self.plugins.out.lock().unwrap());
        for (plugin, t) in out.toasts { self.plugin_toast(plugin, t, now); }
        for t in out.sends { self.send_text(t, now); }
        for (space, t) in out.sends_to { self.post_to(space, t); }
        if out.kept_changed {
            for (space, list) in self.messages.iter_mut() { self.plugins.reapply_kept(space, list); crate::ui::messages::prepare_arrived(list); }
        }
        for (key, id, v) in out.settings_set {
            let r = self.plugins.list.iter().position(|e| e.key == key).ok_or_else(|| "no such plugin".to_string()).and_then(|i| self.plugins.set_setting(i, &id, v));
            if let Err(e) = r { self.plugin_log.push(format!("[{key}] settings.set {id}: {e}")); }
        }
        if let Some(t) = out.clipboard { crate::ui::copy_text(t); }
        if let Some(d) = out.dialog { self.plugins.dialog = Some(d); }
        // Plugin threads read spaces, people and messages from a copy (`spaces.list`, `space.history`, ...), kept only while a
        // plugin listens to `message.received`.
        self.plugins.feed(&self.spaces, &self.people, &self.messages, changed);
        self.ack_on_activation(); // the window came to the front with an unread chat open
        self.plugin_log.extend(out.logs);
        self.toasts.retain(|t| t.1 > now);
    }

    /// A new list of `space` replaces the loaded one: the plugins see what was deleted or edited in between and what they
    /// keep is drawn into it (extras/plugins/kept.rs). Nothing happens without such a plugin.
    fn list_arrived(&mut self, space: &str, list: &mut Vec<cloud::Message>) {
        let (name, team) = self.spaces.iter().find(|s| s.id == space).map(|s| (s.title.clone(), s.team.clone())).unwrap_or_default();
        let team_name = self.teams.iter().find(|t| !team.is_empty() && t.id == team).map(|t| t.name.clone()).unwrap_or_default();
        let at = crate::extras::plugins::kept::Where { space, name: &name, team: &team, team_name: &team_name };
        self.plugins.list_arrived(at, self.messages.get(space).map(Vec::as_slice), list);
    }

    pub(crate) fn go(&mut self, tab: Tab, space: Option<String>) {
        self.history.truncate(self.hpos + 1);
        if self.history.last() != Some(&(tab, space.clone())) { self.history.push((tab, space)); }
        self.hpos = self.history.len() - 1;
    }

    pub(crate) fn set_tab(&mut self, t: Tab) { self.tab = t; if t == Tab::Messaging || t == Tab::Teams { self.team_sel = None } /* the Teams rail entry returns to the team list */ self.go(t, self.selected.clone()); }

    pub(crate) fn open_space(&mut self, id: String) {
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == id) { s.unread = false; }
        if let Some(c) = &self.client { c.req(Req::Messages(id.clone())); }
        self.selected = Some(id.clone());
        self.new_space = None; // opening a space leaves the "Create a space" pane (also the startup one)
        self.conv_tab = ConvTab::Messages;
        if self.tab != Tab::Teams { self.tab = Tab::Messaging; }
        self.go(self.tab, Some(id));
    }

    /// Whether a message of someone else in `id` contains `needle` (lower case: my name, or "@all"). The caller lowers it once:
    /// the list filter asks for every space each frame.
    pub(crate) fn mentions_of(&self, id: &str, needle: &str) -> bool {
        self.messages.get(id).is_some_and(|m| m.iter().any(|m| !m.mine && m.text.to_lowercase().contains(needle)))
    }

    pub(crate) fn visible(&self) -> Vec<Space> {
        self.visible_idx().into_iter().map(|i| self.spaces[i].clone()).collect()
    }

    /// `visible()` as indices into `self.spaces` (the list draws only the rows on screen, so it needn't clone every space,
    /// participants included, each frame).
    pub(crate) fn visible_idx(&self) -> Vec<usize> {
        let q = self.search.to_lowercase();
        let me = if matches!(self.filter, Filter::Notifications | Filter::MentionMe) { self.me.name.to_lowercase() } else { String::new() };
        self.spaces.iter().enumerate().filter(|(_, s)| {
            let tab = match self.list_tab { ListTab::All => true, ListTab::Direct => s.direct, ListTab::Spaces => !s.direct, ListTab::Public => s.public };
            let f = match self.filter {
                Filter::None => true,
                Filter::Notifications => s.unread || self.mentions_of(&s.id, &me),
                Filter::Drafts => self.drafts.get(&s.id).is_some_and(|d| !d.trim().is_empty()),
                Filter::Unread => s.unread,
                Filter::Favorites => s.favorite,
                Filter::MentionMe => self.mentions_of(&s.id, &me),
                Filter::MentionAll => self.mentions_of(&s.id, "@all"),
                Filter::Flags => self.flags.iter().any(|f| f.0 == s.id),
            };
            !s.hidden && tab && f && (q.is_empty() || s.title.to_lowercase().contains(&q)) && self.team_sel.as_ref().is_none_or(|t| &s.team == t)
        }).map(|(i, _)| i).collect()
    }

    pub(crate) fn act(&mut self, a: Act, ctx: &egui::Context) {
        let step = |app: &mut App, d: i32| {
            let v = app.visible();
            let i = v.iter().position(|s| Some(&s.id) == app.selected.as_ref()).map_or(0, |i| (i as i32 + d).clamp(0, v.len() as i32 - 1) as usize);
            if let Some(s) = v.get(i) { app.open_space(s.id.clone()) }
        };
        let hist = |app: &mut App, d: i32| {
            let i = app.hpos as i32 + d;
            if i < 0 || i as usize >= app.history.len() { return; }
            app.hpos = i as usize;
            let (t, s) = app.history[app.hpos].clone();
            app.tab = t;
            // a space from the history is opened like a click on it (messages loaded, read), without a new history entry
            if let Some(s) = s.filter(|s| app.selected.as_ref() != Some(s) && app.spaces.iter().any(|x| &x.id == s)) {
                if let Some(x) = app.spaces.iter_mut().find(|x| x.id == s) { x.unread = false; }
                if let Some(c) = &app.client { c.req(Req::Messages(s.clone())); }
                app.selected = Some(s);
                app.conv_tab = ConvTab::Messages;
            }
        };
        match a {
            Act::Tab(t) => self.set_tab(t),
            Act::Search => self.focus_search = true,
            Act::Prefs => self.set_tab(Tab::Settings),
            Act::Shortcuts => self.show_shortcuts = !self.show_shortcuts,
            Act::Prev => step(self, -1),
            Act::Next => step(self, 1),
            Act::Back => hist(self, -1),
            Act::Forward => hist(self, 1),
            Act::CreateSpace => { self.set_tab(Tab::Messaging); self.new_space = Some(String::new()) }
            Act::Contact => self.set_tab(Tab::Contacts),
            Act::Quit => { self.notify.quitting = true; ctx.send_viewport_cmd(egui::ViewportCommand::Close) } // quit, not hide to tray
            Act::ZoomIn => self.sc_zoom(ctx, 0.1), // persisted
            Act::ZoomOut => self.sc_zoom(ctx, -0.1),
            Act::List(l) => { self.set_tab(Tab::Messaging); self.list_tab = l }
            Act::Filter(f) => { self.set_tab(Tab::Messaging); self.filter = f }
            Act::Favorite => if let Some((id, on)) = self.spaces.iter().find(|s| Some(&s.id) == self.selected.as_ref()).map(|s| (s.id.clone(), !s.favorite)) { self.set_favorite(&id, on) },
            Act::ToggleRail => { self.rail_wide = !self.rail_wide; self.settings["rail_wide"] = json!(self.rail_wide); system::save_settings(&self.cfg, &self.settings); }
            Act::AddPeople => self.add_people = self.selected.is_some(),
            Act::LeaveSpace => self.confirm_leave = self.selected.clone(),
            Act::Reload => if let (Some(c), Some(id)) = (&self.client, &self.selected) { c.req(Req::Messages(id.clone())); c.req(Req::Spaces) },
            Act::Composer(id) => self.sc.composer_events.push(id),
            Act::CopyInfo => if let Some(s) = self.spaces.iter().find(|s| Some(&s.id) == self.selected.as_ref()) { ctx.copy_text(format!("{} ({})", s.title, s.id)) },
        }
    }

    pub(crate) fn apply_theme(&mut self, ctx: &egui::Context) -> Th {
        if !self.fonts_set {
            self.fonts_set = true;
            // main()'s creator already installed them before the first frame (also kept across sign-out, which rebuilds App):
            // only load them (14 MB from disk) when they are missing.
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) { set_fonts(ctx); }
        }
        // Resolved once per change, not every frame: the theme's tokens and the design plugins' palette / accents / decorations
        // (calls across the plugin ABI that allocate). Again when the theme, a plugin or its settings, my name or the minute
        // changes (plugins may follow the clock), or when egui replaced the visuals (it does on a system light / dark switch);
        // high contrast follows the system colours, so it is resolved every frame.
        let id = self.theme_id(ctx);
        let key = (id, self.plugins.generation, (ctx.input(|i| i.time) / 60.0) as i64, self.me.name.clone());
        if id != ThemeId::HighContrast {
            if let Some((k, t)) = &self.theme_cache {
                if *k == key && Some(ctx.style().visuals.selection.stroke.color) == self.theme_sel { return *t; }
            }
        }
        let t = self.resolve_theme(ctx, id);
        self.theme_cache = Some((key, t));
        t
    }

    fn resolve_theme(&mut self, ctx: &egui::Context, id: ThemeId) -> Th {
        let light_mode = !id.is_dark();
        let mut t = Th::of(id);
        // Design plugins (plugin API palette) paint over the selected theme's colours.
        if let Some(p) = self.plugins.palette() {
            let set = |c: &mut Color32, v: u32| if let Some(x) = rgba(v) { *c = x };
            for c in [&mut t.conv, &mut t.conv_b, &mut t.card, &mut t.card_b, &mut t.composer] { set(c, p.bg_primary) }
            for c in [&mut t.win, &mut t.win_b, &mut t.top] { set(c, p.bg_secondary) }
            for c in [&mut t.selected, &mut t.hover, &mut t.search, &mut t.rail_sel] { set(c, p.bg_tertiary) }
            set(&mut t.text1, p.text_primary); set(&mut t.text2, p.text_secondary);
            for c in [&mut t.accent, &mut t.focus, &mut t.unread, &mut t.banner_b] { set(c, p.accent) }
            set(&mut t.link, p.hyperlink);
        }
        // Accent themes (plugin-api 1.3): accent roles only, computed by the plugin for this base theme (ui/theme/accents.rs).
        let info = crate::ui::theme::accents::info(&t);
        // Decorations (plugin-api 1.4): washes, host-drawn decorations and effects (ui/theme/decor.rs, fx.rs).
        crate::ui::theme::decor::set(self.plugins.decor(&info).map(|d| crate::ui::theme::decor::from_api(&d)), &self.me.name);
        let acc = self.plugins.accents(&info).map(|a| crate::ui::theme::accents::apply(&mut t, &a));
        let sel = acc.as_ref().and_then(|a| a.selection).unwrap_or(t.accent);
        let (ov, grad) = acc.map_or_else(Default::default, |a| (a.overrides, a.gradient));
        crate::ui::theme::set_accent_overrides(ov, grad);
        crate::icons::set_art_ink(t.text1);
        if self.applied != Some(t) || ctx.style().visuals.selection.stroke.color != sel {
            let mut v = if light_mode { egui::Visuals::light() } else { egui::Visuals::dark() };
            v.panel_fill = t.win; v.window_fill = t.card; v.extreme_bg_color = t.composer; v.faint_bg_color = t.card;
            v.hyperlink_color = t.link; v.override_text_color = Some(t.text1);
            v.selection.bg_fill = alpha(sel, 0.4); v.selection.stroke = Stroke::new(1.0_f32, sel);
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, t.sep);
            v.widgets.inactive.weak_bg_fill = t.selected; v.widgets.inactive.bg_fill = t.selected; v.widgets.inactive.bg_stroke = Stroke::NONE;
            v.widgets.hovered.weak_bg_fill = t.pill; v.widgets.hovered.bg_fill = t.pill;
            v.widgets.active.weak_bg_fill = t.accent; v.widgets.active.bg_fill = t.accent;
            for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] { w.corner_radius = 16.0.into(); }
            ctx.set_visuals(v);
            self.applied = Some(t);
        }
        self.theme_sel = Some(sel);
        t
    }
}

mod signin;
mod live_list;
pub mod actions;
pub mod files; // upload / download state shared by the UI and the cloud thread

/// Whether `m` @-mentions me: a person mention with my id in its rich text, else (plain text only) my name in the text, as
/// `App::mentions_of` reads it.
fn mentions_me(m: &crate::cloud::Message, me: &crate::cloud::Person) -> bool {
    if !m.html.is_empty() { return !me.id.is_empty() && m.html.contains(&format!("data-object-id=\"{}\"", me.id)); }
    !me.name.is_empty() && m.text.to_lowercase().contains(&me.name.to_lowercase())
}
