//! Space menus and dialogs, built with the shared popup toolkit (`ui/menu/`): the chat banner's gear menu, the list row's
//! "···" / right-click menu, and the rename / add-people / leave dialogs.
//!
//! Row menu of a 1:1 chat: "Mark as unread" / "Mark as read", "Add to favorites", "Add to Contacts",
//! "Open in new window", "Mute notifications", "Move to section", "Create a section", "Hide", "Leave".
//! A group space adds "Add to a team", "Add people...", "Copy space link", "Remove from team" in the same order.
//! The gear menu starts with "Start live translation" and "Mute notifications" (the App Hub entry is left out).
//! Entries whose function Webox cannot perform (contacts, sections, separate windows, live translation, the DM policy text,
//! meeting capabilities) are shown disabled, never as a silent no-op.
use crate::*;
use super::menu::{self, Align, Button, Entry, Item, Side};
use super::spaces_ext::{self as sx, Dlg};

impl App {
    /// The gear menu of the chat banner.
    pub(crate) fn gear_entries(&self, space: &Space) -> Vec<Entry> {
        let mut v = Vec::new();
        v.push(Item::new("translate", tr("Start live translation")).icon("translate-regular").disabled().entry());
        if space.direct {
            v.push(self.mute_item(space));
            v.push(Item::new("policy", tr("View Direct Message Policy")).icon("info-circle-regular").disabled().entry());
        } else {
            v.push(Item::new("notify", tr("Notifications...")).icon("alert-regular").entry());
        }
        v.push(Item::new("meeting", tr("Meeting capabilities")).icon("calendar-month-regular").disabled().entry());
        v.push(Item::new("section", tr("Move to section...")).icon("webex-teams-regular").disabled().entry());
        if !space.direct {
            v.push(Entry::Sep);
            v.push(Item::new("rename", tr("Edit space information")).icon("edit-regular").entry());
            v.push(Item::new("people", tr("Add people...")).icon("people-add-regular").shortcut("Ctrl+Shift+P").entry());
            v.push(Item::new("link", tr("Copy space link")).icon("link-regular").entry());
            v.push(self.team_item(space));
            if !self.moderated(&space.id) || self.i_moderate(&space.id) {
                v.push(Item::new("archive", tr("Archive space")).icon("archive-regular").entry());
            }
        }
        v.push(Entry::Sep);
        v.push(Item::new("leave", tr("Leave")).icon("exit-room-regular").danger().entry());
        v
    }

    fn mute_item(&self, space: &Space) -> Entry {
        if self.notify_of(&space.id).muted() { Item::new("mute", tr("Unmute notifications")).icon("alert-regular").entry() }
        else { Item::new("mute", tr("Mute notifications")).icon("alert-muted-regular").entry() }
    }
    fn team_item(&self, space: &Space) -> Entry {
        if space.team.is_empty() { Item::new("team-add", tr("Add to a team")).icon("webex-teams-regular").enabled(!self.teams.is_empty()).entry() }
        else { Item::new("team-remove", tr("Remove from team")).icon("webex-teams-remove-regular").entry() }
    }

    /// The "···" / right-click menu of a list row.
    pub(crate) fn row_entries(&self, space: &Space) -> Vec<Entry> {
        let mut v = vec![
            if space.unread { Item::new("read", tr("Mark as read")).icon("email-read-regular").entry() }
            else { Item::new("unread", tr("Mark as unread")).icon("mark-as-unread-regular").entry() },
            Item::new("favorite", if space.favorite { tr("Remove from favorites") } else { tr("Add to favorites") }).icon(if space.favorite { "favorite-filled" } else { "favorite-regular" }).entry(),
        ];
        if space.direct {
            v.push(Item::new("contact", tr("Add to Contacts")).icon("participant-add-regular").disabled().entry());
            v.push(Item::new("window", tr("Open in new window")).icon("pop-out-regular").disabled().entry());
            v.push(self.mute_item(space));
        } else {
            v.push(Item::new("window", tr("Open in new window")).icon("pop-out-regular").disabled().entry());
            v.push(Item::new("notify", tr("Notifications...")).icon("alert-regular").entry());
            v.push(Item::new("people", tr("Add people...")).icon("people-add-regular").entry());
            v.push(self.team_item(space));
            v.push(Item::new("link", tr("Copy space link")).icon("link-regular").entry());
        }
        v.push(Item::new("section", tr("Move to section")).icon("webex-teams-regular").disabled().entry());
        v.push(Item::new("section-new", tr("Create a section")).icon("plus-regular").disabled().entry());
        v.push(Entry::Sep);
        v.push(Item::new("hide", tr("Hide")).icon("hide-regular").entry());
        v.push(Item::new("leave", tr("Leave")).icon("exit-room-regular").danger().entry());
        v
    }

    /// Runs a picked menu action for `space` (the space is made the current one first where a dialog needs it).
    pub(crate) fn space_action(&mut self, ctx: &egui::Context, action: &str, space: &Space, now: f64) {
        let current = |s: &mut Self| if s.selected.as_ref() != Some(&space.id) { s.open_space(space.id.clone()); };
        match action {
            "favorite" => self.set_favorite(&space.id, !space.favorite),
            "unread" => self.mark_space_unread(&space.id),
            "read" => self.mark_read(&space.id),
            "mute" => self.toggle_mute(&space.id, now),
            "notify" => sx::open_dialog(Dlg::Notify(space.id.clone(), sx::self_notify_index(self.notify_of(&space.id)))),
            "link" => { ctx.copy_text(sx::space_link(&space.id)); self.toast(tr("Link copied"), now); }
            "copy" => ctx.copy_text(format!("{} ({})", space.title, space.id)),
            "hide" => self.hide_space(&space.id),
            "rename" => { current(self); self.rename_input = Some(space.title.clone()); }
            "people" => { current(self); self.add_people = true; }
            "team-add" => sx::open_dialog(Dlg::TeamPick(space.id.clone(), self.teams.first().map(|t| t.id.clone()))),
            "team-remove" => sx::open_dialog(Dlg::TeamRemove(space.id.clone(), space.team.clone())),
            "archive" => sx::open_dialog(Dlg::ArchiveSpace(space.id.clone())),
            // A 1:1 cannot be left server side: "Leave" on a 1:1 hides it (the service answers a leave with HIDE).
            "leave" => if space.direct { self.hide_space(&space.id) } else { self.confirm_leave = Some(space.id.clone()); },
            _ => {} // disabled entries cannot be picked
        }
    }

    pub(crate) fn space_dialogs(&mut self, ctx: &egui::Context, t: &Th, space: &Space) {
        if let Some(mut name) = self.rename_input.take() {
            let mut keep = true;
            let mut enter = false;
            let mut field = |ui: &mut egui::Ui| {
                ui.label(RichText::new(tr("Space name")).size(13.0));
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY).hint_text(tr("Space name")));
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) { enter = true; }
            };
            let res = menu::webex_dialog(ctx, t, "rename", tr("Edit space information"), "", Some(&mut field), &[Button::secondary("cancel", tr("Cancel")), Button::primary("save", tr("Save"))]);
            match res.as_deref() {
                Some("save") => { keep = false; }
                Some(_) => { keep = false; name.clear(); }
                None if enter => { keep = false; }
                None => {}
            }
            if keep { self.rename_input = Some(name); }
            else if !name.trim().is_empty() && name.trim() != space.title { self.rename_space(&space.id, name.trim()); }
        }
        if self.add_people {
            let mut picks: Vec<Person> = Vec::new();
            let q = self.people_filter.to_lowercase();
            let people: Vec<Person> = self.people.iter().filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q) || p.email.to_lowercase().contains(&q)).cloned().collect();
            let mut filter = std::mem::take(&mut self.people_filter);
            let mut body = |ui: &mut egui::Ui| {
                ui.add(egui::TextEdit::singleline(&mut filter).hint_text(tr("Name or email address")).desired_width(f32::INFINITY));
                ui.add_space(6.0);
                crate::ui::widgets::vscroll().max_height(260.0).show(ui, |ui| {
                    for p in &people {
                        let already = space.participants.iter().any(|x| x.id == p.id || (!p.email.is_empty() && x.email.eq_ignore_ascii_case(&p.email)));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}  {}", p.name, p.email)).size(14.0));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if already { ui.label(RichText::new(tr("In the space")).color(t.text2)); }
                                else if ui.button(tr("Add")).clicked() { picks.push(p.clone()); }
                            });
                        });
                    }
                });
            };
            let res = menu::webex_dialog(ctx, t, "add-people", &crate::trq!("Add people to “%1”", space.title), "", Some(&mut body), &[Button::primary("close", tr("Done"))]);
            self.people_filter = filter;
            for p in picks { self.add_person(&space.id, &p); with_people_added(&space.id, &p); }
            if res.is_some() { self.add_people = false; self.people_filter.clear(); }
        }
        if let Some(id) = self.confirm_leave.clone() {
            // A team's general space: leaving it leaves the team.
            let team = self.spaces.iter().find(|s| s.id == id).map(|s| s.team.clone()).filter(|tm| self.team_info(tm).is_some_and(|i| i.general == id));
            let body = if team.is_some() { tr("This is the general space for all team members. By leaving this space, you are also leaving the team. Leaving the team removes you from all of its spaces.") }
                else { tr("You will be removed from the space, and will need to be invited back.") };
            let res = menu::webex_dialog(ctx, t, "leave", tr("Leave this space?"), body, None,
                &[Button::secondary("cancel", tr("Cancel")), Button::danger("leave", tr("Leave"))]);
            match res.as_deref() {
                Some("leave") => { match team { Some(tm) => self.leave_team(&tm), None => self.leave_space(&id) } self.confirm_leave = None; }
                Some(_) => self.confirm_leave = None,
                None => {}
            }
        }
    }

    /// Draws the open gear / row menus (called once per frame from the messaging screen) and runs the picked action.
    pub(crate) fn space_menus(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        // Row "···" menu: target space + the button's rect were stored when it was opened.
        if let Some((sid, anchor)) = ctx.data(|d| d.get_temp::<(String, Rect)>(egui::Id::new("row-menu-target"))) {
            if let Some(space) = self.spaces.iter().find(|s| s.id == sid).cloned() {
                let plug = self.row_menu_plugins(ctx, &space);
                let mut entries = self.row_entries(&space);
                entries.extend(crate::ui::messages::plugin_entries(&plug));
                let entries = crate::ui::messages::tidy_menu(entries);
                if let Some(a) = menu::webex_menu(ctx, &mut self.icons, t, "row", anchor, Side::Right, Align::Left, false, &entries) {
                    match a.strip_prefix("plug:").and_then(|i| i.parse::<usize>().ok()).and_then(|i| plug.get(i)) {
                        Some(it) => self.plugins.space_menu_action(&it.plugin, &space.id, &it.id),
                        None => self.space_action(ctx, &a, &space, now),
                    }
                }
            }
        }
        self.spaces_ext_dialogs(ctx, t, now);
    }
}

impl App {
    /// The plugins' entries of the open row menu of `space` (`space.menu`): asked once per opening, kept while it stays open;
    /// none (and no plugin asked) while it is closed.
    fn row_menu_plugins(&self, ctx: &egui::Context, space: &Space) -> Vec<crate::extras::plugins::MenuItem> {
        let k = egui::Id::new("row-menu-plugins");
        if !menu::menu_is_open(ctx, "row") { ctx.data_mut(|d| d.remove::<(String, Vec<crate::extras::plugins::MenuItem>)>(k)); return vec![]; }
        if let Some((sid, items)) = ctx.data(|d| d.get_temp::<(String, Vec<crate::extras::plugins::MenuItem>)>(k)) { if sid == space.id { return items; } }
        let items = self.plugins.space_menu(&json!({ "space": space.id, "title": space.title, "direct": space.direct, "team": space.team }));
        ctx.data_mut(|d| d.insert_temp(k, (space.id.clone(), items.clone())));
        items
    }
}

/// Keeps a loaded full people list in step with an addition (the participants of `Space` are updated by `add_person`).
fn with_people_added(space: &str, p: &Person) {
    sx::with(|x| if let Some(l) = x.people.get_mut(space) { if !l.iter().any(|y| y.id == p.id) { l.push(p.clone()); } });
}
