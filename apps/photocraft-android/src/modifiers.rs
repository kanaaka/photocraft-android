//! Modifier keys, tracked by the shell because the platform never reports them.
//!
//! # The gap
//!
//! winit 0.30.13's Android backend emits exactly six window events — `Focused`, `Resized`,
//! `ScaleFactorChanged`, `RedrawRequested`, `Touch` and `KeyboardInput`
//! (`src/platform_impl/android/mod.rs`) — and **never** `WindowEvent::ModifiersChanged`.
//! egui-winit updates its modifier state from that one event and nowhere else, and egui 0.36
//! carries modifiers as an *event* (`RawInput` has no `modifiers` field any more), so on Android
//! `ctx.input(|i| i.modifiers)` is the all-false default for the whole run:
//!
//! - Ctrl+A (Select All), Ctrl+S (Save), Ctrl+Z (Undo) and every other shortcut do nothing,
//! - Shift+click extends nothing, because the click arrives with no modifier set,
//! - Shift+[ and Shift+] never step the brush size, Alt keytips never come up.
//!
//! The *key events themselves* do arrive, so the modifiers can be tracked from them — which is
//! what a desktop backend does internally. This module does that and reports the state the way
//! egui expects: `Event::ModifiersChanged` (egui's `InputState` reads modifiers from that event
//! alone) plus the per-event `modifiers` copies that `Event::Key` and `Event::PointerButton`
//! carry.
//!
//! Call [`Tracked::apply`] from the shell's `eframe::App::raw_input_hook`, *before* forwarding to
//! the app's own hook, so the app sees the same frames it would on the desktop.
//!
//! # What it does not do
//!
//! The Super/Windows key is deliberately not tracked: on Windows and Linux egui's `command`
//! modifier is Ctrl and `mac_cmd` is documented as macOS-only, so there is no modifier here for
//! it to become. Alt is tracked and reported — it is what opens keytips — but egui's own note
//! that Alt+Shift is how macOS types special characters does not apply on Android.

/// The three modifiers that mean something on this platform, held between frames.
///
/// Between frames because the key press and the click are usually in *different* frames: the user
/// holds Shift down, and only then clicks the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tracked {
    alt: bool,
    ctrl: bool,
    shift: bool,
}

impl Tracked {
    /// Fold this frame's events into the tracked state and report it back into them.
    pub fn apply(&mut self, raw: &mut egui::RawInput) {
        let events = std::mem::take(&mut raw.events);
        let mut out = Vec::with_capacity(events.len() + 1);
        for mut event in events {
            self.observe(&event);
            // Every event that carries a copy gets the state as it stands at that point in the
            // batch, so a Shift press and a click in the same frame are ordered correctly.
            let modifiers = self.reported();
            match &mut event {
                egui::Event::Key { modifiers: m, .. } | egui::Event::PointerButton { modifiers: m, .. } => *m = modifiers,
                _ => {}
            }
            out.push(event);
        }
        // egui's `InputState` takes modifiers from this event and nowhere else, and a frame with no
        // key events in it — the vast majority — still has to report the state it is holding.
        out.push(egui::Event::ModifiersChanged(self.reported()));
        raw.events = out;
    }

    /// The state as egui wants to see it.
    fn reported(&self) -> egui::Modifiers {
        // On Windows and Linux the shortcut modifier is Ctrl, and `mac_cmd` must stay off there.
        egui::Modifiers { alt: self.alt, ctrl: self.ctrl, shift: self.shift, mac_cmd: false, command: self.ctrl }
    }

    /// Update from one event.
    fn observe(&mut self, event: &egui::Event) {
        match event {
            // A modifier key going down or coming up. Physical keys: winit reports the left and
            // right keys separately, and either of them is "the" modifier.
            egui::Event::Key { key, pressed, .. } => match key {
                egui::Key::ShiftLeft | egui::Key::ShiftRight => self.shift = *pressed,
                egui::Key::ControlLeft | egui::Key::ControlRight => self.ctrl = *pressed,
                egui::Key::AltLeft | egui::Key::AltRight => self.alt = *pressed,
                _ => {}
            },
            // The platform reported them itself. It never does on Android, but egui-winit
            // synthesises one when the window loses focus, and that report is authoritative: it is
            // how a modifier that was held while the app went to the background stops being stuck.
            egui::Event::ModifiersChanged(reported) => {
                self.shift = reported.shift;
                self.ctrl = reported.ctrl;
                self.alt = reported.alt;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frame's events, with nothing else set.
    fn frame(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput { events, ..Default::default() }
    }

    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// A click with no modifiers — exactly what Android delivers, which is the bug.
    fn click() -> egui::Event {
        egui::Event::PointerButton {
            pos: egui::Pos2::ZERO,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn click_modifiers(raw: &egui::RawInput) -> egui::Modifiers {
        raw.events
            .iter()
            .find_map(|e| match e {
                egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .expect("the click survived")
    }

    fn reported(raw: &egui::RawInput) -> egui::Modifiers {
        raw.events
            .iter()
            .rev()
            .find_map(|e| match e {
                egui::Event::ModifiersChanged(m) => Some(*m),
                _ => None,
            })
            .expect("a ModifiersChanged event is always appended: egui's InputState reads modifiers from it alone")
    }

    /// The whole point: the key press and the click are normally in *different* frames.
    #[test]
    fn shift_held_in_one_frame_reaches_a_click_in_the_next() {
        let mut tracked = Tracked::default();
        let mut down = frame(vec![key(egui::Key::ShiftLeft, true)]);
        tracked.apply(&mut down);
        assert!(reported(&down).shift, "Shift down is reported");

        let mut later = frame(vec![click()]);
        tracked.apply(&mut later);
        assert!(click_modifiers(&later).shift, "a click in a frame with no key events still carries Shift");
    }

    #[test]
    fn ctrl_is_the_command_modifier_for_shortcuts() {
        let mut tracked = Tracked::default();
        let mut raw = frame(vec![key(egui::Key::ControlLeft, true), key(egui::Key::A, true)]);
        tracked.apply(&mut raw);
        let m = reported(&raw);
        assert!(m.ctrl, "Ctrl is held");
        assert!(m.command, "Ctrl must also be the command modifier on Android, or Ctrl+A is dead");
        assert!(!m.mac_cmd, "mac_cmd stays off outside macOS");
        // The Key event itself carries the state too, so a shortcut handler reading it sees Ctrl.
        let a = raw
            .events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { key: egui::Key::A, modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .expect("the A press survived");
        assert!(a.ctrl);
    }

    #[test]
    fn releasing_a_modifier_clears_it() {
        let mut tracked = Tracked::default();
        tracked.apply(&mut frame(vec![key(egui::Key::ShiftRight, true)]));
        assert!(tracked.reported().shift);
        tracked.apply(&mut frame(vec![key(egui::Key::ShiftRight, false)]));
        assert!(!tracked.reported().shift, "the right Shift releases what the left one set: either is 'the' modifier");
    }

    /// egui-winit synthesises one when the window loses focus. If the user held Ctrl and the app
    /// went to the background, that report has to win or Ctrl stays stuck down.
    #[test]
    fn a_platform_report_overrides_what_was_tracked() {
        let mut tracked = Tracked::default();
        tracked.apply(&mut frame(vec![key(egui::Key::ControlLeft, true)]));
        assert!(tracked.reported().ctrl);

        let cleared = egui::Modifiers { alt: false, ctrl: false, shift: false, mac_cmd: false, command: false };
        tracked.apply(&mut frame(vec![egui::Event::ModifiersChanged(cleared)]));
        assert!(!tracked.reported().ctrl, "the platform's own report wins");
    }

    /// Alt opens keytips, so it is tracked — but it must never leak into `command`.
    #[test]
    fn alt_is_tracked_without_becoming_command() {
        let mut tracked = Tracked::default();
        tracked.apply(&mut frame(vec![key(egui::Key::AltLeft, true)]));
        let m = tracked.reported();
        assert!(m.alt);
        assert!(!m.command, "Alt must not become the shortcut modifier");
    }
}
