//! Composer actions: drafts, recent emoji, mentions, sending, formatting, the rich toolbar and the markdown legend.
use super::*;

impl App {
    pub(super) fn emoji_autocomplete_on(&self) -> bool { self.settings["emoji_autocomplete"].as_bool().unwrap_or(true) } // "Emoji-Autovervollständigung verwenden", default on
    pub(super) fn emoticons_on(&self) -> bool { self.settings["emoticons_to_emojis"].as_bool().unwrap_or(true) } // feature flag desktop-emoticons-to-emojis

    /// Height of the composer card (it grows with the text rows, the edit header, the chips and the rich-text toolbar).
    pub(crate) fn composer_height(&self, ctx: &egui::Context) -> f32 {
        let st = st_get(ctx);
        let rich = st.rich || self.settings["composer_rich_toolbar"].as_bool().unwrap_or(false);
        metrics(matches!(self.compose, Compose::Edit { .. }), rich, chips_height(&st), st.rows).2
    }

    /// The space's own composer while a message is edited in the overlay: one text row, no chips (they belong to the editor).
    pub(crate) fn composer_idle_height(&self) -> f32 {
        metrics(false, self.settings["composer_rich_toolbar"].as_bool().unwrap_or(false), 0.0, 1).2
    }

    pub(crate) fn save_drafts(&mut self) {
        if self.shot.is_some() { return; }
        // A draft put aside while editing is saved under its space while the edit text is in the editor - see `end_edit`.
        let stash = |k: &str| k.contains('\u{1}');
        let editing = match &self.compose { Compose::Edit { .. } => self.selected.clone(), _ => None };
        let m: serde_json::Map<String, serde_json::Value> = self.drafts.iter().filter(|(k, v)| !stash(k) && !v.trim().is_empty() && editing.as_deref() != Some(k.as_str())).map(|(k, v)| (k.clone(), json!(v)))
            .chain(editing.iter().filter_map(|s| self.drafts.get(&crate::app::actions::edit_stash_key(s)).filter(|v| !v.trim().is_empty()).map(|v| (s.clone(), json!(v))))).collect();
        self.settings["drafts"] = serde_json::Value::Object(m);
        system::save_settings(&self.cfg, &self.settings);
    }

    pub(super) fn record_recent_emoji(&mut self, ch: &str) {
        let mut r: Vec<String> = self.settings["emoji_recents"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        r.retain(|x| x != ch);
        r.insert(0, ch.to_string());
        r.truncate(30);
        self.settings["emoji_recents"] = json!(r);
        if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); }
    }

    pub(super) fn mention_cands(&self, space: &Space, q: &str) -> Vec<mp::Cand> {
        let mut people = space.participants.clone();
        if space.direct { if let Some(p) = self.direct_person(space) { people.push(p); } }
        mp::candidates(q, !space.direct, &people, &self.people, &self.me)
    }

    /// Mentions: with sample data the message is stored with its `<spark-mention>` HTML; signed in they are sent via `Req::Post`.
    pub(super) fn send_with_mentions(&mut self, text: String, ms: &[Mention], now: f64) {
        let Some(id) = self.selected.clone() else { return };
        // Signed in: the mentions go to the data layer (`Req::Post`), which sends them as <spark-mention>.
        if self.client.is_some() {
            let refs = ms.iter().map(|m| crate::richtext::MentionRef { id: m.id.clone(), name: m.name.clone(), all: m.all }).collect();
            self.send_text_with(text, refs, now);
            return;
        }
        if !matches!(self.compose, Compose::New | Compose::Reply { .. }) { self.send_text(text, now); return; }
        let Some(text) = self.plugins.on_send(text.trim()) else { self.toast("A plugin stopped this message", now); return };
        let html = ct::to_content_html(&text, ms);
        let mode = std::mem::take(&mut self.compose);
        let list = self.messages.entry(id.clone()).or_default();
        let parent = if let Compose::Reply { root, .. } = mode { root } else { String::new() };
        let m = Message { id: format!("{id}-local{}", list.len()), author: self.me.name.clone(), email: self.me.email.clone(), text, html, time: "now".into(), date: system::today(), mine: true, parent, ..Default::default() };
        let at = if m.parent.is_empty() { list.len() } else { list.iter().rposition(|x| x.id == m.parent || x.parent == m.parent).map_or(list.len(), |i| i + 1) };
        list.insert(at, m);
    }

    /// Enter / the send pill / "Aktualisieren": emoticons first, then the @all confirmation, then the message.
    pub(super) fn composer_send(&mut self, st: &mut St, space: &Space, confirmed: bool, now: f64) {
        let id = space.id.clone();
        let mut text = self.drafts.get(&id).cloned().unwrap_or_default();
        if self.emoticons_on() { text = ct::parse_emoticons(&text); }
        // The "Neue Nachricht" pane sends through ui/new_message/ (opens / creates the direct space).
        if id == super::super::new_message::DRAFT_ID {
            if !text.trim().is_empty() { self.drafts.insert(super::super::new_message::SEND_KEY.into(), text); }
            st.mentions.clear(); st.emoji_q = None; st.ment_q = None;
            return;
        }
        let ms: Vec<Mention> = ct::live_mentions(&text, &st.mentions).into_iter().cloned().collect();
        let was_edit = matches!(self.compose, Compose::Edit { .. });
        let gif = self.gif_staged(&id);
        let quoted = !matches!(self.compose, Compose::Edit { .. }) && self.quote_waiting(&id);
        if text.trim().is_empty() && st.chips.is_empty() && !gif && !quoted { return; }
        if !confirmed && ms.iter().any(|m| m.all) && !space.direct { st.confirm_all = true; return; }
        self.gif_send_staged(&id, now); // a staged GIF goes out first (Req::SendGif)
        let quote = if quoted { self.quote_take(&id) } else { None }; // the waiting quote goes with this message
        if !st.chips.is_empty() {
            // The files and the text go out together as one share activity.
            let chips = std::mem::take(&mut st.chips);
            let refs: Vec<crate::richtext::MentionRef> = ms.iter().map(|m| crate::richtext::MentionRef { id: m.id.clone(), name: m.name.clone(), all: m.all }).collect();
            if let Some(q) = quote {
                // A quote and files at once: the text travels with the quote, the files follow as their own share.
                self.send_quoted(&id, text.trim().to_string(), refs, q);
                self.send_staged(&id, chips, String::new(), Vec::new(), now);
            } else {
                let text = self.plugins.on_send(text.trim()).unwrap_or_default();
                self.send_staged(&id, chips, text, refs, now);
            }
        } else if let Some(q) = quote {
            let refs = ms.iter().map(|m| crate::richtext::MentionRef { id: m.id.clone(), name: m.name.clone(), all: m.all }).collect();
            self.send_quoted(&id, text.trim().to_string(), refs, q);
        } else if !text.trim().is_empty() {
            if ms.is_empty() { self.send_text(text, now); } else { self.send_with_mentions(text, &ms, now); }
        }
        self.drafts.insert(id.clone(), String::new());
        if was_edit { self.end_edit(&id); } // hides the edit overlay; the space's own draft returns
        st.mentions.clear();
        st.emoji_q = None; st.ment_q = None;
        self.save_drafts();
    }

    /// Esc priority: cancel the mention search -> cancel edit mode -> cancel the thread reply.
    pub(super) fn composer_escape(&mut self, st: &mut St, id: &str) {
        if st.emoji_q.is_some() { st.emoji_cancel = st.emoji_q.as_ref().map(|q| q.0); return; }
        if st.ment_q.is_some() { st.ment_cancel = st.ment_q.as_ref().map(|q| q.0); return; }
        // Cancelling an edit hides the overlay and brings the space's own draft back.
        if matches!(self.compose, Compose::Edit { .. }) { self.end_edit(id); return; }
        self.compose = Compose::New;
    }

    pub(super) fn apply_format(&mut self, ctx: &egui::Context, id: &str, f: Format) {
        let d = self.drafts.entry(id.to_string()).or_default();
        let len = d.chars().count();
        let (a, b) = get_sel(ctx, len);
        let (t, na, nb) = ct::apply_format(d, a, b, f);
        *d = t;
        set_sel(ctx, na, nb);
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(CID)));
        ctx.request_repaint();
    }

    pub(super) fn toggle_markdown(&mut self, st: &mut St, now: f64) {
        st.md = !st.md;
        self.settings["markdown_mode"] = json!(st.md);
        if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); }
        self.toast(if st.md { "Markdown aktiviert" } else { "Markdown deaktiviert" }, now);
    }

    /// Esc closes an open popup first; returns true when it did.
    pub(super) fn escape_for_popup(&mut self, st: &mut St, em_open: bool, mn_open: bool) -> bool {
        if em_open { st.emoji_cancel = st.emoji_q.as_ref().map(|q| q.0); st.emoji_q = None; return true; }
        if mn_open { st.ment_cancel = st.ment_q.as_ref().map(|q| q.0); st.ment_q = None; st.ment_dir = None; return true; }
        false
    }

    pub(super) fn rich_toolbar(&mut self, ui: &mut egui::Ui, t: &Th, row: Rect, st: &mut St, id: &str) {
        let light = super::super::menu::light(t);
        let border = if light { hex(0xEDEDED) } else { hex(0x3B3B3B) }; // wx-default-border
        let ink = if light { hex(0x121212) } else { hex(0xF3F3F3) };
        let mut x = row.left();
        let cy = row.center().y;
        let groups: [&[(&'static str, &str, Format)]; 2] = [
            &[("bold-regular", "Fett (Strg+B)", Format::Bold), ("italic-regular", "Kursiv (Strg+I)", Format::Italic), ("underline-bold", "Unterstrichen (Strg+U)", Format::Underline), ("strikethrough-bold", "Durchgestrichen (Alt+K)", Format::Strike)],
            &[("heading-one-regular", "Überschrift 1 (Alt+Umschalt+1)", Format::Heading(1)), ("heading-two-regular", "Überschrift 2 (Alt+Umschalt+2)", Format::Heading(2)), ("heading-three-regular", "Überschrift 3 (Alt+Umschalt+3)", Format::Heading(3))],
        ];
        let mut pending: Option<Format> = None;
        for g in groups {
            let w = g.len() as f32 * 24.0 + 2.0;
            let pill = Rect::from_min_size(pos2(x, cy - 13.0), Vec2::new(w, 26.0));
            ui.painter().rect_stroke(pill, 10.0, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
            for (i, (icon, tip, f)) in g.iter().enumerate() {
                let r = Rect::from_min_size(pos2(pill.left() + 1.0 + i as f32 * 24.0, cy - 12.0), Vec2::splat(24.0));
                let resp = ui.interact(r, ui.id().with(("rich", *icon)), Sense::click());
                if resp.hovered() { ui.painter().rect_filled(r.shrink(1.0), 8.0, t.hover); }
                self.icons.paint(ui, icon, Rect::from_center_size(r.center(), Vec2::splat(16.0)), ink);
                if resp.on_hover_text(*tip).clicked() { pending = Some(*f); }
            }
            x += w + 5.0;
        }
        let single = |s: &mut Self, ui: &mut egui::Ui, x: &mut f32, icon: &'static str, tip: &str, size: f32| -> egui::Response {
            let r = Rect::from_min_size(pos2(*x, cy - size / 2.0), Vec2::splat(size));
            let resp = ui.interact(r, ui.id().with(("rich1", icon)), Sense::click());
            if resp.hovered() { ui.painter().circle_filled(r.center(), size / 2.0, t.hover); }
            s.icons.paint(ui, icon, Rect::from_center_size(r.center(), Vec2::splat(16.0)), ink);
            *x += size + 5.0;
            resp.on_hover_text(tip)
        };
        if single(self, ui, &mut x, "link-regular", "Link (Strg+K)", 24.0).clicked() { pending = Some(Format::Link); }
        let more = single(self, ui, &mut x, "more-regular", "Weitere Formatierungsoptionen", 28.0);
        if more.clicked() { super::super::menu::menu_toggle(ui.ctx(), "composer-more"); }
        x += 5.0;
        ui.painter().vline(x, (cy - 11.0)..=(cy + 11.0), Stroke::new(1.0_f32, border)); // 1 x 22 separator, 5 / 6 px spacers
        x += 6.0;
        if single(self, ui, &mut x, "markdown_16_em", if st.md { "Markdown deaktivieren (Alt+Umschalt+M)" } else { "Markdown einschalten (Alt+Umschalt+M)" }, 28.0).clicked() { st.legend = true; st.legend_anchor = Some(Rect::from_min_size(pos2(x - 33.0, cy - 14.0), Vec2::splat(28.0))); }
        // "Weitere" menu: Webox's own wording.
        let items: Vec<super::super::menu::Entry> = vec![
            super::super::menu::Item::new("sub", "Tiefgestellt").entry(), super::super::menu::Item::new("sup", "Hochgestellt").entry(), super::super::menu::Entry::Sep,
            super::super::menu::Item::new("code", "Inline-Code").entry(), super::super::menu::Entry::Sep,
            super::super::menu::Item::new("bullet", "Aufzählung").entry(), super::super::menu::Item::new("num", "Nummerierte Liste").entry(), super::super::menu::Entry::Sep,
            super::super::menu::Item::new("dedent", "Einzug verkleinern").entry(), super::super::menu::Item::new("indent", "Einzug vergrößern").entry(),
        ];
        if let Some(a) = super::super::menu::webex_menu(ui.ctx(), &mut self.icons, t, "composer-more", more.rect, super::super::menu::Side::Above, super::super::menu::Align::Left, true, &items) {
            pending = Some(match a.as_str() { "sub" => Format::Sub, "sup" => Format::Sup, "code" => Format::InlineCode, "bullet" => Format::BulletList, "num" => Format::NumberList, "dedent" => Format::Dedent, _ => Format::Indent });
        }
        if let Some(f) = pending { let ctx = ui.ctx().clone(); self.apply_format(&ctx, id, f); }
    }

    /// Markdown legend: 280 px wide popup above the M button; title, subtitle, divider, syntax list (scrolls above 200 px),
    /// divider, "Markdown" checkbox with the contextual warning. German strings from the language pack.
    pub(super) fn markdown_legend(&mut self, ctx: &egui::Context, t: &Th, st: &mut St, now: f64) {
        let light = super::super::menu::light(t);
        let (bg, border) = if light { (Color32::WHITE, alpha(Color32::BLACK, 0.2)) } else { (hex(0x121212), alpha(Color32::WHITE, 0.15)) };
        let div = if light { hex(0xEDEDED) } else { hex(0x3B3B3B) }; // wx-default-border
        // (kind, text): h = header, b = bold item, i = italic item, n = item, x = example header, e = example item, s = 4 px separator.
        let data: [(char, &str); 29] = [('h', "Schriftart"), ('b', "**Fett**"), ('i', "_Kursiv_"), ('n', "> Zitat"), ('s', ""),
            ('h', "Listen"), ('n', "1. Erster Punkt"), ('n', "2. Zweiter Punkt"), ('n', "3. Dritter Punkt"), ('n', "* Aufzählungspunkt 1"), ('n', "* Aufzählungspunkt 2"), ('s', ""),
            ('h', "Überschriften"), ('n', "# Überschrift 1"), ('n', "## Überschrift 2"), ('n', "### Überschrift 3"), ('n', "--- Horizontal line"), ('s', ""),
            ('h', "Links"), ('n', "[Linktext](Link)"), ('x', "Beispiel"), ('e', "[Cisco](www.cisco.com)"), ('s', ""),
            ('h', "Code"), ('n', "``` Sprache (optional)\nCODEBLOCK\n```"), ('x', "Beispiel"), ('e', "``` HTML\n<h1>Hallo Welt! </h1>\n```"), ('n', "\"Nicht formatierter Text\""), ('s', "")];
        let anchor = st.legend_anchor.unwrap_or(Rect::NOTHING);
        let mut inside = Rect::NOTHING;
        let area = egui::Area::new(egui::Id::new("md-legend")).fixed_pos(pos2(anchor.left().max(8.0), anchor.top() - 8.0)).pivot(egui::Align2::LEFT_BOTTOM).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::default().fill(bg).stroke(Stroke::new(1.0_f32, border)).corner_radius(8.0).show(ui, |ui| {
                ui.set_width(280.0);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let wrap = |ui: &mut egui::Ui, text: &str, font: FontId, top: f32, bottom: f32| {
                    ui.add_space(top);
                    ui.horizontal(|ui| { ui.add_space(16.0); ui.scope(|ui| { ui.set_max_width(248.0); ui.add(egui::Label::new(RichText::new(text).font(font).color(t.text1)).wrap()); }); });
                    ui.add_space(bottom);
                };
                wrap(ui, "Markdown-Formate", semibold(16.0), 12.0, 0.0);
                wrap(ui, "Sonderzeichen um den Text hinzufügen, um eine Formatierung anzuwenden", FontId::proportional(14.0), 4.0, 8.0);
                ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top() + 0.5, Stroke::new(1.0_f32, div));
                ui.add_space(1.0);
                crate::ui::widgets::vscroll().id_salt("md-legend-scroll").max_height(200.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    for (k, text) in data {
                        if k == 's' { ui.add_space(4.0); continue; }
                        let lines = text.lines().count().max(1) as f32;
                        let h = (lines * 17.0 + 10.0).max(24.0);
                        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
                        let lm = if k == 'h' || k == 'x' { 16.0 } else { 37.0 };
                        let (size, bold, italic) = (if k == 'h' { 16.0 } else { 14.0 }, k == 'h' || k == 'b', k == 'i' || k == 'e');
                        let mut rt = RichText::new(text).size(size).color(t.text1);
                        if bold { rt = rt.strong(); }
                        if italic { rt = rt.italics(); }
                        ui.painter().text(pos2(r.left() + lm, r.center().y), egui::Align2::LEFT_CENTER, "", FontId::proportional(size), t.text1);
                        let g = ui.painter().layout_no_wrap(text.to_string(), if bold { semibold(size) } else { FontId::proportional(size) }, t.text1);
                        let g = if lines > 1.0 { ui.painter().layout(text.to_string(), if bold { semibold(size) } else { FontId::proportional(size) }, t.text1, 240.0) } else { g };
                        ui.painter().galley(pos2(r.left() + lm, r.center().y - g.size().y / 2.0), g, t.text1);
                        let _ = rt;
                    }
                });
                ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top() + 0.5, Stroke::new(1.0_f32, div));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let mut md = st.md;
                    if ui.checkbox(&mut md, RichText::new("Markdown").strong().size(14.0).color(t.text1)).changed() { self.toggle_markdown(st, now); }
                });
                ui.add_space(2.0);
                let warn = if st.md { "Durch die Markdown-Deaktivierung werden Markdown-Formatierungen deaktiviert." } else { "Durch die Markdown-Aktivierung werden vorhandene Formatierungen entfernt." };
                ui.horizontal(|ui| { ui.add_space(36.0); ui.scope(|ui| { ui.set_max_width(232.0); ui.add(egui::Label::new(RichText::new(warn).size(14.0).color(t.text1)).wrap()); }); });
                ui.add_space(10.0);
                inside = ui.min_rect();
            });
        });
        // A press outside closes it (not on the M button, which toggles it itself).
        let pressed = ctx.input(|i| i.pointer.any_pressed()).then(|| ctx.input(|i| i.pointer.interact_pos())).flatten();
        if let Some(p) = pressed { if !area.response.rect.contains(p) && !anchor.contains(p) { st.legend = false; } }
        let _ = inside;
    }
}
