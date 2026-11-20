//! Screenshot-mode input scripts (never active in a normal run): real pointer / wheel events injected into egui's raw input,
//! so scroll checks go through the same path as a mouse.
//!
//! `WEBOX_SHOT_POINTER=x,y` keeps the pointer at that point from frame 2 on.
//! `WEBOX_SHOT_WHEEL=from-to:dy[;from-to:dy...]` sends a wheel step of `dy` points (positive = up, as egui) every frame in
//! that frame range (inclusive), at the pointer position.
//! `WEBOX_SHOT_MIDDLE=frame:x,y[;frame:x,y...]` moves the pointer to x,y and clicks the middle button there at that frame.
use eframe::egui;
use egui::{Event, MouseWheelUnit, PointerButton, Pos2, RawInput, Vec2};

/// `(from, to, dy)` ranges of `WEBOX_SHOT_WHEEL`.
pub(crate) fn parse_wheel(s: &str) -> Vec<(u32, u32, f32)> {
    s.split(';').filter_map(|part| {
        let (range, dy) = part.split_once(':')?;
        let (a, b) = range.split_once('-').unwrap_or((range, range));
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?, dy.trim().parse().ok()?))
    }).collect()
}

fn parse_pos(s: &str) -> Option<Pos2> {
    let (x, y) = s.split_once(',')?;
    Some(Pos2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Adds this frame's scripted events to `raw` (`frame` = the number the coming update will count).
pub(crate) fn inject(frame: u32, raw: &mut RawInput) {
    let pointer = std::env::var("WEBOX_SHOT_POINTER").ok().and_then(|v| parse_pos(&v));
    if let Some(p) = pointer { if frame >= 2 { raw.events.push(Event::PointerMoved(p)); } }
    if let Ok(v) = std::env::var("WEBOX_SHOT_MIDDLE") {
        for part in v.split(';') {
            let Some((f, pos)) = part.split_once(':') else { continue };
            if f.trim().parse::<u32>().ok() != Some(frame) { continue }
            let Some(p) = parse_pos(pos) else { continue };
            raw.events.push(Event::PointerMoved(p));
            raw.events.push(Event::PointerButton { pos: p, button: PointerButton::Middle, pressed: true, modifiers: Default::default() });
            raw.events.push(Event::PointerButton { pos: p, button: PointerButton::Middle, pressed: false, modifiers: Default::default() });
        }
    }
    // WEBOX_SHOT_CLICK=frame:x,y[:ctrl|shift|ctrlshift|slow][;...]: a left click there, with those keys held, at that frame.
    // `slow` presses at that frame and releases 3 frames later (a real click spans frames: press and release are separate passes).
    if let Ok(v) = std::env::var("WEBOX_SHOT_CLICK") {
        // The screenshot window never gets the OS focus, and egui's `Response::has_focus` needs it: a clicked window is focused.
        raw.focused = true;
        for part in v.split(';') {
            let mut it = part.split(':');
            let (Some(f), Some(pos)) = (it.next(), it.next()) else { continue };
            let Some(f) = f.trim().parse::<u32>().ok() else { continue };
            let k = it.next().unwrap_or("");
            let slow = k.contains("slow");
            let (press, release) = (frame == f, if slow { frame == f + 3 } else { frame == f });
            if !press && !release && !(slow && frame > f && frame < f + 3) { continue }
            let Some(p) = parse_pos(pos) else { continue };
            let modifiers = egui::Modifiers { ctrl: k.contains("ctrl"), command: k.contains("ctrl"), shift: k.contains("shift"), ..Default::default() };
            raw.modifiers = modifiers;
            raw.events.push(Event::PointerMoved(p));
            if press { raw.events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers }); }
            if release { raw.events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers }); }
        }
    }
    // WEBOX_SHOT_KEY=frame:Name[;...]: press + release of that key (egui `Key::from_name`, e.g. Tab, Enter) at that frame;
    // `frame:text=abc` types that text
    if let Ok(v) = std::env::var("WEBOX_SHOT_KEY") {
        raw.focused = true;
        for part in v.split(';') {
            let Some((f, name)) = part.split_once(':') else { continue };
            if f.trim().parse::<u32>().ok() != Some(frame) { continue }
            if let Some(t) = name.strip_prefix("text=") { raw.events.push(Event::Text(t.to_string())); continue } // typed text
            let Some(key) = egui::Key::from_name(name.trim()) else { continue };
            for pressed in [true, false] {
                raw.events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() });
            }
        }
    }
    if let Ok(v) = std::env::var("WEBOX_SHOT_WHEEL") {
        for (a, b, dy) in parse_wheel(&v) {
            if frame >= a && frame <= b { raw.events.push(Event::MouseWheel { unit: MouseWheelUnit::Point, delta: Vec2::new(0.0, dy), modifiers: Default::default() }); }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wheel_script() {
        assert_eq!(super::parse_wheel("50-60:-20;70:5"), vec![(50, 60, -20.0), (70, 70, 5.0)]);
        assert!(super::parse_wheel("x").is_empty());
    }
}
