//! The composer's first frame and space switch: drafts and markdown mode restored, screenshot presets (WEBOX_SHOT_COMPOSER).
use super::*;

impl App {
    /// First frame / space switch (part of `composer`, draw.rs).
    pub(super) fn composer_restore(&mut self, ctx: &egui::Context, st: &mut St, id: &String, cid: egui::Id, shot: &Option<String>, now: f64) {
        // Not switched here yet: follow the account's toggle (it arrives with the device registration).
        if st.loaded && self.settings["markdown_mode"].is_null() { st.md = self.markdown_on(); }
        if !st.loaded {
            st.loaded = true;
            if let Some(o) = self.settings["drafts"].as_object() { for (k, v) in o { if let Some(s) = v.as_str() { self.drafts.entry(k.clone()).or_insert_with(|| s.to_string()); } } }
            st.md = self.markdown_on(); // default on (actions.rs markdown_on)
            st.rich = self.settings["composer_rich_toolbar"].as_bool().unwrap_or(false);
            st.space = id.clone();
            if let Some(k) = &shot {
                let d = self.drafts.entry(id.clone()).or_default();
                match k.as_str() {
                    "emoji" => { *d = "Prost :so".into(); }
                    "emoji2" => { *d = "Prost :beer".into(); }
                    "sob" => { *d = "Oh :sob".into(); }
                    "onlyemoji" => { *d = "😆".into(); }
                    "mention" => { *d = "Hallo @".into(); }
                    "emojitext" => { *d = "Hallo 😀 Welt 👍🏽 🇨🇭 👨‍👩‍👧 x".into(); }
                    "mention2" => { *d = "Hallo @to".into(); }
                    "legend" => { st.legend = true; }
                    "markdown" => { st.md = true; *d = "# Titel\n**fett** und _kursiv_\n1. Eins\n2. Zwei".into(); }
                    "rich" => { st.rich = true; *d = "Text".into(); }
                    "edit" => { *d = "Nachricht die ich bearbeite".into(); self.compose = Compose::Edit { msg: "x".into() }; }
                    "attach" | "upload" => {
                        st.chips = vec![Chip { name: "Quartalsbericht.pdf".into(), size: 1_200_000, path: "Quartalsbericht.pdf".into(), image: false }];
                        self.stage_bytes(&mut st.chips, "foto.png", &super::super::sample_photos::sample_banner(1), false, now);
                        if k == "upload" { let chips = std::mem::take(&mut st.chips); self.send_staged(&id, chips, "Hier die Unterlagen".into(), vec![], now); }
                    }
                    "mentionpill" => { st.mentions = vec![Mention { id: "mara@example.com".into(), name: "Mara Lind".into(), all: false }]; *d = "Hallo @Mara Lind bitte prüfen".into(); }
                    _ => {}
                }
                ctx.memory_mut(|m| m.request_focus(cid));
                let n = self.drafts.get(id).map_or(0, |d| d.chars().count());
                set_sel(&ctx, n, n);
            }
        }
        if st.space != *id {
            let old = std::mem::replace(&mut st.space, id.clone());
            if !old.is_empty() { self.save_drafts(); }
            st.mentions.clear(); st.chips.clear(); st.emoji_q = None; st.ment_q = None; st.legend = false; st.confirm_all = false; st.ment_dir = None;
        }
    }
}
