//! Opening the plugin's popup, switching tabs and views, fetching pages.
use super::*;

fn kind_trending(tab: Tab) -> Kind { if tab == Tab::Stickers { Kind::StickerTrending } else { Kind::Trending } }
fn kind_search(tab: Tab, q: &str) -> Kind { if tab == Tab::Stickers { Kind::StickerSearch(q.to_string()) } else { Kind::Search(q.to_string()) } }

impl State {
    pub(super) fn fetch(&mut self, kind: Kind, offset: u32) {
        if self.sample { return }
        let Some(tx) = self.tx.clone() else { return };
        self.ticket += 1;
        self.loading = true;
        self.error = None;
        self.kind = Some(kind.clone());
        giphy::fetch_page(tx, self.ticket, kind, offset);
    }
}

impl Plugin {
    /// Resets the popup to the GIF tab's home screen (sample mode uses generated GIFs and no network).
    pub(in crate::extras::gif_extras) fn picker_open(&mut self, app: &mut App, ctx: &egui::Context) {
        let st = &mut self.picker;
        st.query.clear(); st.focus = true; st.sel = None; st.debounce.reset(); st.error = None; st.scroll_to_sel = false;
        st.tab = Tab::Gifs; st.view = View::Home; st.gifs.clear(); st.total = 0; st.next_offset = 0; st.loading = false; st.kind = None;
        st.sample = app.client.is_none();
        st.thumbs.offline = st.sample;
        if st.tx.is_none() { let (tx, rx) = channel(); st.tx = Some(tx); st.rx = Some(rx); }
        st.thumbs.trim(&Default::default());
        if st.sample {
            if st.sample_all.is_empty() {
                let all = giphy::sample_gifs();
                for (g, b) in &all { st.thumbs.insert(&g.id, Rc::new(b.clone())); }
                st.sample_all = all.iter().map(|(g, _)| g.clone()).collect();
                // Stickers: the same pictures under their own ids and titles.
                for (i, (g, b)) in all.iter().enumerate() {
                    let id = format!("sample-stk-{i}");
                    st.thumbs.insert(&id, Rc::new(b.clone()));
                    st.sample_stk.push(Gif { id, title: format!("Sticker {}", g.title), ..g.clone() });
                }
                for (c, i) in giphy::sample_categories() {
                    st.thumbs.insert(&format!("cat:{}", c.name), Rc::new(all[i % all.len()].1.clone()));
                    st.sample_cats.push(c);
                }
            }
            st.trending = [Some((st.sample_all.clone(), st.sample_all.len() as u32, st.sample_all.len() as u32)), Some((st.sample_stk.clone(), st.sample_stk.len() as u32, st.sample_stk.len() as u32))];
            st.cats = Some(st.sample_cats.clone());
        } else {
            if st.cats.is_none() && !st.cats_loading {
                if let Some(tx) = st.tx.clone() { st.cats_loading = true; giphy::fetch_categories(tx); }
            }
            if st.trending[0].is_none() { st.fetch(Kind::Trending, 0); } // preview of the "Angesagte GIFs" tile
        }
        // Screenshot runs only (WEBOX_SHOT_GIF together with WEBOX_SHOT_GIF_EXTRAS=1).
        if app.shot.is_some() && app.client.is_none() && !self.picker.shot_done {
            if let Ok(mode) = std::env::var("WEBOX_SHOT_GIF") {
                self.picker.shot_done = true;
                self.picker.focus = false;
                match mode.as_str() {
                    "stickers" => self.set_tab(app, ctx, Tab::Stickers),
                    "emoji" => self.set_tab(app, ctx, Tab::Emojis),
                    "trending" => self.enter(app, View::Trending, ""),
                    "search" => self.enter(app, View::Search, "wow"),
                    "favs" | "favs-empty" => {
                        if mode == "favs" { let pick: Vec<Gif> = [1, 4, 6].iter().filter_map(|i| self.picker.sample_all.get(*i).cloned()).collect(); for g in &pick { self.favs(app).toggle(g, false); } }
                        self.enter(app, View::Favorites, "");
                    }
                    "upfavs" => { // an uploaded GIF kept as a favourite (temporary folder), shown in "Favoriten" beside a Giphy one
                        let dir = upload_stars::upload_fav_dir(app);
                        let (g0, g3) = (self.picker.sample_all.first().cloned(), self.picker.sample_all.get(3).map(|g| g.id.clone()));
                        if let (Some(g0), Some(b)) = (g0, g3.and_then(|id| self.picker.thumbs.get(&id))) {
                            if let Ok(f) = upload_favs::star(&dir, "Eigene Animation.gif", &b) { self.favs(app).toggle(&g0, false); self.favs(app).insert(f); }
                        }
                        self.enter(app, View::Favorites, "");
                    }
                    "home-favs" => { if let Some(g) = self.picker.sample_all.get(3).cloned() { self.favs(app).toggle(&g, false); } }
                    _ => {}
                }
                self.picker.focus = false;
            }
        }
    }

    /// Switches the tab: GIFs / Sticker start at their home screen, Emojis shows the embedded emoji picker.
    pub(in crate::extras::gif_extras) fn set_tab(&mut self, app: &mut App, ctx: &egui::Context, tab: Tab) {
        if self.picker.tab == tab { return }
        self.picker.tab = tab;
        self.picker.debounce.reset();
        if tab == Tab::Emojis { app.emoji_picker_reset(ctx); return }
        self.picker.focus = true;
        self.enter(app, View::Home, "");
    }

    /// Opens one view of the GIF / Sticker tab.
    pub(in crate::extras::gif_extras) fn enter(&mut self, app: &mut App, view: View, q: &str) {
        let favs: Vec<Gif> = if view == View::Favorites { let sticker = self.picker.tab == Tab::Stickers; self.favs(app).list(sticker).into_iter().map(|x| x.to_gif()).collect() } else { vec![] };
        let st = &mut self.picker;
        let sticker = st.tab == Tab::Stickers;
        let idx = tab_idx(st.tab);
        st.view = view; st.sel = None; st.scroll_to_sel = true; st.error = None; st.gifs.clear(); st.total = 0; st.next_offset = 0; st.loading = false;
        match view {
            View::Home => {
                st.query.clear(); st.kind = None;
                if st.trending[idx].is_none() { st.fetch(kind_trending(st.tab), 0); } // preview of the trending tile
            }
            View::Favorites => {
                st.query.clear(); st.kind = None;
                st.gifs = favs;
                st.total = st.gifs.len() as u32; st.next_offset = st.total;
            }
            View::Trending => {
                st.query.clear();
                if let Some((g, next, total)) = st.trending[idx].clone() { st.ticket += 1; st.gifs = g; st.next_offset = next; st.total = total; st.kind = Some(kind_trending(st.tab)); }
                else { st.fetch(kind_trending(st.tab), 0); }
            }
            View::Search => {
                st.query = q.to_string();
                if st.sample {
                    let pool = if sticker { &st.sample_stk } else { &st.sample_all };
                    st.gifs = pool.iter().filter(|g| g.title.to_lowercase().contains(&q.to_lowercase())).cloned().collect();
                    st.total = st.gifs.len() as u32; st.next_offset = st.total;
                } else { st.fetch(kind_search(st.tab, q), 0); }
            }
        }
    }
}
