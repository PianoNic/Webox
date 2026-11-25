//! Presence for people outside the spaces the list was loaded with: people found in the directory search (search
//! results, "New message", and the profile cards opened from them) and the participants of a conversation that appeared
//! live (cloud/live_list/). They get a presence snapshot at once and join the usersub subscription, which the 540 s
//! renewal keeps (start.rs). Uses the usersub subscribe (people.rs `subscribe_presence`) and snapshot (`presence`).
use super::*;
use std::collections::HashSet;

/// People followed although no loaded space has them.
#[derive(Default)]
pub struct Extra { pub people: Vec<Person> }

/// The uuid presence services want for a person (`Person.id` may be e-mail style; then the remembered uuid).
fn uuid_key(uuids: &HashMap<String, String>, p: &Person) -> Option<String> {
    let is_uuid = |s: &str| s.len() == 36 && s.matches('-').count() == 4;
    if is_uuid(&p.id) { return Some(p.id.clone()) }
    uuids.get(&p.id).or_else(|| uuids.get(&p.email)).cloned()
}

/// Pure: who of `people` is not followed yet (by uuid), given the followed people.
pub fn not_followed(uuids: &HashMap<String, String>, followed: &[Person], people: &[Person]) -> Vec<Person> {
    let have: HashSet<String> = followed.iter().filter_map(|p| uuid_key(uuids, p)).collect();
    let mut seen = HashSet::new();
    people.iter().filter(|p| uuid_key(uuids, p).is_some_and(|u| !have.contains(&u) && seen.insert(u))).cloned().collect()
}

impl Session {
    /// Everyone followed besides the spaces' participants (for the renewal).
    pub(super) fn extra_people(&self) -> Vec<Person> { self.extra.people.clone() }

    /// Follows the people of `people` not followed yet: presence snapshot (sent to the UI) + usersub subscription.
    /// `except` = a space whose participants must not count as followed (it was just added). Returns the statuses.
    pub(super) fn subscribe_new(&mut self, people: &[Person], except: Option<&str>) -> Vec<(String, String, i64)> {
        let mut followed: Vec<Person> = self.me_person().into_iter().collect();
        followed.extend(self.spaces.values().filter(|s| Some(s.id.as_str()) != except).flat_map(|s| s.participants.iter().cloned()));
        followed.extend(self.extra.people.iter().cloned());
        let new: Vec<Person> = not_followed(&self.uuids, &followed, people).into_iter().filter(|p| !self.is_me(p)).collect();
        if new.is_empty() { return vec![] }
        dbg_conn(&format!("presence: following {} more people", new.len()));
        let statuses = self.presence(&new);
        self.subscribe_presence(&new);
        self.extra.people.extend(new);
        super::people::send_presence(&self.out, statuses.clone()); // main's presence bookkeeping: last status per contact
        statuses
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(id: &str, email: &str) -> Person { Person { id: id.into(), email: email.into(), ..Default::default() } }
    const U1: &str = "00000000-0000-4000-8000-000000000001";
    const U2: &str = "00000000-0000-4000-8000-000000000002";

    #[test]
    fn directory_hits_are_followed_once_and_renewed() {
        let (mut s, _rx) = super::super::live_list::tests::session();
        s.spaces.insert("sp".into(), Space { id: "sp".into(), participants: vec![p(U1, "ann@example.com")], ..Default::default() });
        // No service map in tests: the snapshot / subscribe requests fail without leaving the machine.
        s.subscribe_new(&[p(U1, "ann@example.com"), p(U2, "bob@example.com")], None);
        assert_eq!(s.extra_people().iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec![U2], "Ann is a space participant already");
        s.subscribe_new(&[p(U2, "bob@example.com")], None);
        assert_eq!(s.extra_people().len(), 1, "not twice");
        // A conversation that just appeared: its participants count as new although it is in the list now.
        s.subscribe_new(&[p(U1, "ann@example.com")], Some("sp"));
        assert_eq!(s.extra_people().len(), 2);
    }

    #[test]
    fn only_people_not_followed_yet() {
        let mut uuids = HashMap::new();
        uuids.insert("ann@example.com".to_string(), U1.to_string());
        let followed = vec![p("ann@example.com", "ann@example.com")];
        let found = vec![p(U1, "ann@example.com"), p(U2, "bob@example.com"), p(U2, "bob@example.com"), p("nobody", "")];
        let new = not_followed(&uuids, &followed, &found);
        assert_eq!(new.len(), 1, "Ann is followed (same uuid), Bob once, no uuid → skipped");
        assert_eq!(new[0].id, U2);
    }
}
