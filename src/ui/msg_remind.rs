//! "Remind me in": a reminder on a message (the message menu's submenu). When it is due, a toast
//! "Reminder notification" with the author and the text comes up; a click opens the space. The reminders are kept on this
//! computer (settings `message_reminders`), so they survive a restart; the service's reminders are not used.
use super::menu;
use super::schedule_send::{now_ms, tomorrow_nine};
use crate::*;

/// A reminder on message `msg` of `space`, due at `due_ms` (UTC ms).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Reminder { pub space: String, pub msg: String, pub author: String, pub text: String, pub due_ms: i64 }

/// The submenu's entries: (id, English label, minutes from now; 0 = tomorrow at 9:00).
pub(crate) const CHOICES: [(&str, &str, i64); 4] = [("remind-20", "20 minutes", 20), ("remind-60", "1 hour", 60), ("remind-180", "3 hours", 180), ("remind-tomorrow", "Tomorrow at 9:00 AM", 0)];

/// The submenu ("Remind me in" as its title, then the choices).
pub(crate) fn entries() -> Vec<menu::Entry> {
    let mut v = vec![menu::Entry::Title(tr("Remind me in").into())];
    v.extend(CHOICES.iter().map(|(id, label, _)| menu::Item::new(id, tr(label)).entry()));
    v
}

/// When a choice is due, from `now` (ms).
pub(crate) fn due_of(choice: &str, now: i64) -> Option<i64> {
    CHOICES.iter().find(|c| c.0 == choice).map(|c| if c.2 == 0 { tomorrow_nine(now) } else { now + c.2 * 60_000 })
}

fn to_json(r: &Reminder) -> serde_json::Value { json!({ "space": r.space, "msg": r.msg, "author": r.author, "text": r.text, "due": r.due_ms }) }
fn from_json(v: &serde_json::Value) -> Option<Reminder> {
    let g = |k: &str| v[k].as_str().map(String::from);
    Some(Reminder { space: g("space")?, msg: g("msg")?, author: g("author").unwrap_or_default(), text: g("text").unwrap_or_default(), due_ms: v["due"].as_i64()? })
}

/// Splits `list` into the reminders due at `now` and the rest (still sorted).
pub(crate) fn take_due(list: &mut Vec<Reminder>, now: i64) -> Vec<Reminder> {
    let (due, keep): (Vec<Reminder>, Vec<Reminder>) = std::mem::take(list).into_iter().partition(|r| r.due_ms <= now);
    *list = keep;
    due
}

impl App {
    pub(crate) fn reminders_load(&mut self) {
        self.msgx.reminders = self.settings["message_reminders"].as_array().into_iter().flatten().filter_map(from_json).collect();
    }
    fn reminders_save(&mut self) {
        if self.shot.is_some() || self.client.is_none() { return; } // the offline sample and screenshots keep them in memory
        self.settings["message_reminders"] = json!(self.msgx.reminders.iter().map(to_json).collect::<Vec<_>>());
        system::save_settings(&self.cfg, &self.settings);
    }

    /// A pick of the "Remind me in" submenu on message `m` of `space` (one reminder per message: a new one replaces it).
    pub(crate) fn remind_pick(&mut self, space: &str, m: &Message, choice: &str, now: f64) {
        let Some(due) = due_of(choice, now_ms()) else { return };
        self.msgx.reminders.retain(|r| r.msg != m.id);
        let author = if m.mine { self.me.name.clone() } else { m.author.clone() };
        self.msgx.reminders.push(Reminder { space: space.into(), msg: m.id.clone(), author, text: m.text.clone(), due_ms: due });
        self.msgx.reminders.sort_by_key(|r| r.due_ms);
        self.reminders_save();
        let (_, _, _, h, mi) = super::schedule_send::local_parts(due);
        self.toast(crate::trq!("Reminder at %1", format!("{h:02}:{mi:02}")), now);
    }

    /// "Remove reminder".
    pub(crate) fn remind_remove(&mut self, msg: &str) {
        self.msgx.reminders.retain(|r| r.msg != msg);
        self.reminders_save();
    }

    /// Once per frame (msgx_tick): due reminders become toasts; the next one schedules a repaint.
    pub(crate) fn reminders_tick(&mut self, ctx: &egui::Context) {
        if self.msgx.reminders.is_empty() { return; }
        let now = now_ms();
        let due = take_due(&mut self.msgx.reminders, now);
        if !due.is_empty() {
            for r in &due { self.notify_message_reminder(&r.space, &r.author, &r.text); }
            self.reminders_save();
        }
        if let Some(next) = self.msgx.reminders.first() { ctx.request_repaint_after(std::time::Duration::from_millis((next.due_ms - now).clamp(500, 60_000) as u64)); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_and_due_reminders() {
        assert_eq!(due_of("remind-20", 1_000), Some(1_000 + 20 * 60_000));
        assert_eq!(due_of("remind-180", 0), Some(3 * 3_600_000));
        assert_eq!(due_of("remind-tomorrow", 5_000), Some(tomorrow_nine(5_000)));
        assert_eq!(due_of("nope", 0), None);
        let ids: Vec<String> = entries().iter().filter_map(|e| match e { menu::Entry::Item(i) => Some(i.label.clone()), _ => None }).collect();
        assert_eq!(ids, [tr("20 minutes"), tr("1 hour"), tr("3 hours"), tr("Tomorrow at 9:00 AM")]);
        let r = |m: &str, due| Reminder { space: "s".into(), msg: m.into(), author: "A".into(), text: "t".into(), due_ms: due };
        let mut l = vec![r("a", 10), r("b", 20), r("c", 30)];
        assert_eq!(take_due(&mut l, 20), [r("a", 10), r("b", 20)]);
        assert_eq!(l, [r("c", 30)]);
        assert_eq!(from_json(&to_json(&r("x", 5))), Some(r("x", 5)));
    }
}
