//! Applies the live list changes of cloud/live_list/ to the window's copy of the space list (no reload). Each arrives as
//! a `Resp`, which already woke the window (start.rs forwards every answer with `ui::wake`).
use crate::*;
use crate::cloud::live_list::{notify_after, ListPatch};

impl App {
    pub(crate) fn on_list_patch(&mut self, p: ListPatch) {
        let find = |app: &mut App, id: &str| app.spaces.iter().position(|s| s.id == id);
        match p {
            // Read on another device of mine: the dot / bold row go, its notifications are withdrawn, the "New
            // messages" line of the next opening sits after the new last-seen time (ui/messages/list.rs).
            ListPatch::Read { space, last_seen, unread } => {
                if let Some(i) = find(self, &space) {
                    let s = &mut self.spaces[i];
                    s.last_seen = last_seen;
                    s.unread = unread;
                }
                if !unread { self.notify.withdraw_space(&space); }
            }
            ListPatch::Upsert(sp) => match find(self, &sp.id) {
                Some(i) => self.spaces[i] = sp,
                // Placed by its last activity like the list load (newest first); a message for it moves it up when it arrives.
                None => { let at = self.spaces.iter().position(|s| s.last < sp.last).unwrap_or(self.spaces.len()); self.spaces.insert(at, sp); }
            },
            ListPatch::Info(info) => crate::ui::spaces_ext::with(|x| {
                x.notify.insert(info.id.clone(), info.notify);
                if info.locked { x.locked.insert(info.id.clone()); } else { x.locked.remove(&info.id); }
                x.moderators.insert(info.id, info.moderators.into_iter().collect());
            }),
            ListPatch::Title { space, title } => if let Some(i) = find(self, &space) { self.spaces[i].title = title },
            ListPatch::Favorite(space, on) => if let Some(i) = find(self, &space) { self.spaces[i].favorite = on },
            ListPatch::Meeting { space, locus_url, active } => if let Some(i) = find(self, &space) {
                let s = &mut self.spaces[i];
                if !locus_url.is_empty() { s.locus_url = locus_url; }
                s.meeting = active;
            },
            ListPatch::Hidden(space, on) => if let Some(i) = find(self, &space) { self.spaces[i].hidden = on },
            ListPatch::Locked(space, on) => crate::ui::spaces_ext::with(|x| { if on { x.locked.insert(space); } else { x.locked.remove(&space); } }),
            ListPatch::NotifyTags { space, add, tags } => crate::ui::spaces_ext::with(|x| {
                let now = x.notify.get(&space).copied().unwrap_or_default();
                x.notify.insert(space, notify_after(now, add, &tags));
            }),
            ListPatch::Moderator { space, ids, on } => crate::ui::spaces_ext::with(|x| {
                let set = x.moderators.entry(space).or_default();
                for id in ids { if on { set.insert(id); } else { set.remove(&id); } }
            }),
            // Left / removed elsewhere: gone from the list; an open one closes to the start pane (as `Resp::Spaces` does).
            ListPatch::Remove(space) => {
                self.spaces.retain(|s| s.id != space);
                if self.selected.as_deref() == Some(space.as_str()) { self.selected = None; }
                self.notify.withdraw_space(&space);
            }
        }
    }
}
