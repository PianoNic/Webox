//! The settings form of one plugin, generated from its schema (src/extras/plugin_settings) with the settings kit: switches as
//! check box rows, text and secrets as text fields, numbers as sliders with the value next to it, choices as combo boxes,
//! buttons as pill buttons. Switches and choices are saved at once; text when the field loses focus or on Enter; a slider when
//! it is let go. A value the schema refuses is not saved and the reason is shown under the field.
use super::{kit, PUi};
use crate::extras::plugin_settings::{schema::fmt_num, Kind, SecretAt};
use crate::*;
use serde_json::Value;

const FIELD_W: f32 = 360.0;

impl App {
    pub(super) fn plugin_form(&mut self, ui: &mut egui::Ui, t: &Th, s: &mut PUi, i: usize, now: f64) {
        let Some(schema) = self.plugins.list[i].settings.schema.clone() else { return };
        for sec in &schema.sections {
            let title = if sec.title.is_empty() { "Einstellungen" } else { &sec.title };
            kit::section(ui, t, title, &sec.description, false, |ui| {
                for f in &sec.fields {
                    ui.add_space(6.0);
                    self.plugin_field(ui, t, s, i, f, now);
                }
            });
        }
    }

    fn apply_setting(&mut self, s: &mut PUi, i: usize, id: &str, v: Value) {
        let k = format!("{}|{id}", self.plugins.list[i].key);
        match self.plugins.set_setting(i, id, v) {
            Ok(()) => { s.errors.remove(&k); s.edits.remove(&k); }
            Err(e) => { s.errors.insert(k, e); }
        }
    }

    fn plugin_field(&mut self, ui: &mut egui::Ui, t: &Th, s: &mut PUi, i: usize, f: &crate::extras::plugin_settings::Field, now: f64) {
        let e = &self.plugins.list[i];
        let (key, builtin) = (e.key.clone(), e.builtin);
        let k = format!("{key}|{}", f.id);
        let cur = e.settings.values().get(&f.id).cloned().unwrap_or(Value::Null);
        let label = |ui: &mut egui::Ui| {
            ui.label(RichText::new(&f.label).size(kit::BODY).color(t.text1));
            if !f.description.is_empty() { kit::text_block(ui, &f.description, kit::SECOND, t.text2, 0.0); }
            ui.add_space(4.0);
        };
        match &f.kind {
            Kind::Bool { .. } => {
                let mut v = cur.as_bool().unwrap_or(false);
                if kit::check(ui, &mut self.icons, t, &f.label, &mut v, true, false) { self.apply_setting(s, i, &f.id, Value::Bool(v)); }
                if !f.description.is_empty() { kit::check_desc(ui, t, &f.description); }
            }
            Kind::Text { multiline, .. } => {
                label(ui);
                let buf = s.edits.entry(k.clone()).or_insert_with(|| cur.as_str().unwrap_or_default().to_string());
                let r = if *multiline {
                    let k2 = kit::tok(t);
                    egui::Frame::default().fill(k2.input_bg).stroke(Stroke::new(1.0, k2.outline)).corner_radius(8.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                        ui.add(egui::TextEdit::multiline(buf).frame(false).desired_rows(4).desired_width(ui.available_width().min(FIELD_W + 200.0)).font(FontId::proportional(kit::BODY)))
                    }).inner
                } else { kit::text_field(ui, t, buf, "", FIELD_W) };
                let commit = r.lost_focus() || (!*multiline && r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
                if commit && Some(buf.as_str()) != cur.as_str() { let v = buf.clone(); self.apply_setting(s, i, &f.id, Value::String(v)); }
            }
            Kind::Secret => {
                label(ui);
                let at = self.plugins.secret_at(i, &f.id);
                let buf = s.edits.entry(k.clone()).or_default();
                let mut submit = false;
                ui.horizontal(|ui| {
                    let r = secret_field(ui, t, buf, if at == SecretAt::Unset { "Wert eingeben" } else { "•••••••• (gespeichert; zum Ändern neu eingeben)" });
                    submit = !buf.is_empty() && (r.lost_focus() || (r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter))));
                    if at != SecretAt::Unset { ui.add_space(8.0); if kit::pill(ui, t, "Entfernen", false, true).clicked() { buf.clear(); submit = true; } }
                });
                if submit { let v = buf.clone(); self.apply_setting(s, i, &f.id, Value::String(v)); s.edits.insert(k.clone(), String::new()); }
                let note = match self.plugins.secret_at(i, &f.id) {
                    SecretAt::Unset => "Nicht gesetzt.",
                    SecretAt::Keyring => if cfg!(windows) { "Gespeichert in der Windows-Anmeldeinformationsverwaltung." } else { "Gespeichert im Schlüsselbund." },
                    SecretAt::File => "Gespeichert in der Einstellungsdatei, NICHT verschlüsselt: kein Schlüsselbund verfügbar.",
                };
                ui.add_space(2.0);
                kit::text_block(ui, note, kit::SECOND, if note.contains("NICHT") { t.tk("listitem-textmissed") } else { t.text2 }, 0.0);
            }
            Kind::Number { min, max, step, .. } => {
                label(ui);
                let stored = cur.as_f64().unwrap_or(*min);
                let mut v: f32 = s.edits.get(&k).and_then(|x| x.parse().ok()).unwrap_or(stored as f32);
                let steps = if *step > 0.0 && (max - min) / step <= 1000.0 { ((max - min) / step).round() as usize + 1 } else { 0 };
                ui.horizontal(|ui| {
                    if kit::slider(ui, t, &mut v, *min as f32, *max as f32, steps, FIELD_W - 60.0) { s.edits.insert(k.clone(), v.to_string()); }
                    ui.add_space(8.0);
                    ui.label(RichText::new(fmt_num(snap(v as f64, *min, *step))).size(kit::BODY).color(t.text1));
                });
                if s.edits.contains_key(&k) && !ui.input(|i| i.pointer.any_down()) {
                    let x = snap(v as f64, *min, *step).clamp(*min, *max);
                    if (x - stored).abs() > f64::EPSILON { self.apply_setting(s, i, &f.id, serde_json::json!(x)); } else { s.edits.remove(&k); }
                }
            }
            Kind::Choice { options, .. } => {
                label(ui);
                let labels: Vec<String> = options.iter().map(|o| o.1.clone()).collect();
                let at = options.iter().position(|o| Some(o.0.as_str()) == cur.as_str()).unwrap_or(0);
                if let Some(n) = kit::combo(ui, &mut self.icons, t, &format!("plugin-{k}"), 260.0, &labels, at) {
                    let v = options[n].0.clone();
                    self.apply_setting(s, i, &f.id, Value::String(v));
                }
            }
            Kind::Button => {
                if kit::pill(ui, t, &f.label, true, true).clicked() {
                    let msg = if builtin { self.builtin_settings_action(&key, &f.id) } else { self.plugins.settings_action(i, &f.id) };
                    if let Some(m) = msg.filter(|m| !m.is_empty()) { self.toast(m, now); }
                }
                if !f.description.is_empty() { ui.add_space(2.0); kit::text_block(ui, &f.description, kit::SECOND, t.text2, 0.0); }
            }
        }
        if let Some(err) = s.errors.get(&k) { ui.add_space(2.0); kit::text_block(ui, err, kit::SECOND, t.tk("listitem-textmissed"), 0.0); }
    }

    /// A settings button of a built-in plugin.
    fn builtin_settings_action(&mut self, key: &str, id: &str) -> Option<String> {
        match key {
            k if k == crate::extras::gif_extras::KEY => crate::extras::gif_extras::settings_action(self, id),
            _ => None,
        }
    }
}

fn snap(v: f64, min: f64, step: f64) -> f64 { if step > 0.0 { min + ((v - min) / step).round() * step } else { v } }

/// kit::text_field with a masked input.
fn secret_field(ui: &mut egui::Ui, t: &Th, text: &mut String, hint: &str) -> egui::Response {
    let k = kit::tok(t);
    let (r, _) = ui.allocate_exact_size(Vec2::new(FIELD_W.min(ui.available_width()), 32.0), Sense::hover());
    ui.painter().rect_filled(r, 8.0, k.input_bg);
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(Vec2::new(12.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let resp = c.add(egui::TextEdit::singleline(text).password(true).frame(false).margin(Vec2::ZERO).hint_text(RichText::new(hint).color(t.text2).size(kit::BODY))
        .font(FontId::proportional(kit::BODY)).desired_width(r.width() - 24.0));
    ui.painter().rect_stroke(r, 8.0, Stroke::new(if resp.has_focus() { 2.0 } else { 1.0 }, if resp.has_focus() { k.focus } else if resp.hovered() { k.outline_hover } else { k.outline }), egui::StrokeKind::Inside);
    resp
}
