//! Older messages at the top of the list: state, request, and the offline sample's simulated pages.
//! Real clients: App::history_state / App::load_older_messages (photos.rs); the cloud thread collects every request of one
//! step (`Session::older_messages`, up to HISTORY_MAX_REQUESTS until HISTORY_PAGE visible rows) and answers once with
//! Resp::OlderMessages, which replaces the list in one go. Offline (--sample) the pages are simulated here.
use super::*;

/// The offline sample's history: (pages, messages per page, chunks per page). Default: SAMPLE_PAGES pages of 6, one update
/// each. Screenshot repro: `WEBOX_SHOT_HIST=pages,size,chunks` (e.g. `4,20,4`) delivers every page in `chunks` separate list
/// updates 150 ms apart, each with a date change and a collapsed join/leave row, like a slow server with several requests.
pub(super) fn sample_hist_cfg() -> (u32, u32, u32) {
    let parse = |v: String| { let n: Vec<u32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect(); (n.len() == 3 && n.iter().all(|x| *x > 0)).then(|| (n[0], n[1], n[2])) };
    std::env::var("WEBOX_SHOT_HIST").ok().and_then(parse).unwrap_or((SAMPLE_PAGES, 6, 1))
}

/// One simulated older page (oldest first): `size` messages over two days, with one collapsed join/leave row when the page
/// is longer than 8 (repro of rows above the viewport whose kind and height depend on their neighbours).
pub(super) fn sample_page(space: &str, page: u32, size: u32, me: &Person) -> Vec<Message> {
    let days = (system::days_ago(4 + 2 * page as i64), system::days_ago(3 + 2 * page as i64));
    (0..size).map(|i| {
        let mine = i % 3 == 2;
        let date = if i < size / 2 { days.0.clone() } else { days.1.clone() };
        let time = format!("{:02}:{:02}", 8 + (i % 12), (10 + i * 7) % 60);
        if size > 8 && i == 3 {
            let group = vec![Person { name: "Lena Brandt".into(), email: "lena@example.com".into(), ..Default::default() }, Person { name: "Tom Berger".into(), email: "tom@example.com".into(), ..Default::default() }];
            return Message { id: format!("{space}-old{page}-{i}"), notice: crate::trq!("{{0}} people joined this space.", 2), group, date, time, ..Default::default() };
        }
        let text = if i % 4 == 1 { format!("Ältere Nachricht {}-{}\nmit einer zweiten Zeile", page + 1, i + 1) } else { format!("Ältere Nachricht {}-{}", page + 1, i + 1) };
        Message { id: format!("{space}-old{page}-{i}"), author: if mine { me.name.clone() } else { "Mara Lind".into() }, email: if mine { me.email.clone() } else { "mara@example.com".into() },
            text, time, mine, date, ..Default::default() }
    }).collect()
}

impl App {
    /// (loading older messages, the start of the conversation is loaded) of a space.
    pub(super) fn history(&self, ctx: &egui::Context, id: &str) -> (bool, bool) {
        if self.client.is_some() { let h = self.history_state(id); return (h.loading, h.reached_start); }
        let now = ctx.input(|i| i.time);
        let (until, pages) = ctx.data(|d| d.get_temp::<(f64, u32)>(Id::new(("msg-hist", id)))).unwrap_or((0.0, 0));
        (until > 0.0, pages >= sample_hist_cfg().0 && until <= now)
    }

    /// How many older pages have arrived in this space (real: the history generation, sample: the page count).
    pub(super) fn history_generation(&self, ctx: &egui::Context, id: &str) -> u32 {
        if self.client.is_some() { return self.history_state(id).generation; }
        ctx.data(|d| d.get_temp::<u32>(Id::new(("msg-hist-updates", id)))).unwrap_or(0)
    }

    /// Asks for the page before the oldest loaded message (once: ignored while a request is running).
    pub(super) fn request_older(&mut self, ctx: &egui::Context, id: &str) {
        if self.client.is_some() { self.load_older_messages(id); return; } // no-op while loading / after the start
        let now = ctx.input(|i| i.time);
        let k = Id::new(("msg-hist", id));
        let (until, pages) = ctx.data(|d| d.get_temp::<(f64, u32)>(k)).unwrap_or((0.0, 0));
        if until > 0.0 || pages >= sample_hist_cfg().0 { return; }
        ctx.data_mut(|d| d.insert_temp(k, (now + 0.6, pages)));
    }

    /// Offline sample: delivers the simulated page (or its next chunk) when its time has come.
    pub(super) fn sample_older_tick(&mut self, ctx: &egui::Context, id: &str) {
        if self.client.is_some() { return; }
        let k = Id::new(("msg-hist", id));
        let Some((until, pages)) = ctx.data(|d| d.get_temp::<(f64, u32)>(k)) else { return };
        if until <= 0.0 { return; }
        let now = ctx.input(|i| i.time);
        if until > now { ctx.request_repaint_after(std::time::Duration::from_secs_f64((until - now).min(0.1))); return; }
        let (_, size, chunks) = sample_hist_cfg();
        let kc = Id::new(("msg-hist-chunk", id));
        let chunk = ctx.data(|d| d.get_temp::<u32>(kc)).unwrap_or(0);
        let page = sample_page(id, pages, size, &self.me);
        // Chunk c = the c-th slice from the end of the page (the newest older messages come first, as the server pages back).
        let per = size.div_ceil(chunks) as usize;
        let hi = page.len().saturating_sub(chunk as usize * per);
        let lo = hi.saturating_sub(per);
        let part: Vec<Message> = page[lo..hi].to_vec();
        let list = self.messages.entry(id.to_string()).or_default();
        list.splice(0..0, part);
        ctx.data_mut(|d| { let u = d.get_temp_mut_or_default::<u32>(Id::new(("msg-hist-updates", id))); *u += 1; });
        if chunk + 1 < chunks && lo > 0 {
            ctx.data_mut(|d| { d.insert_temp(kc, chunk + 1); d.insert_temp(k, (now + 0.15, pages)); });
        } else {
            ctx.data_mut(|d| { d.insert_temp(kc, 0u32); d.insert_temp(k, (0.0, pages + 1)); });
        }
        ctx.request_repaint();
    }
}
