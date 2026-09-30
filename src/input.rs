//! SDL keyboard handling for the windowed frontend.

use sdl2::keyboard::{Mod, Scancode};

use uwot_m8::config::{Bound, Config};
use uwot_m8::keys::{self, Keyjazz};

/// What the main loop should do in response to a key event.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    NoteOn(u8, u8),
    NoteOff,
    /// A control-change binding's key went down or came up; which slot it was.
    CcPress(usize),
    CcRelease(usize),
    ToggleFullscreen,
    OpenSettings,
    Quit,
}

/// The name a key is known by in the config file.
pub fn key_name(scancode: Scancode) -> Option<String> {
    Some(match scancode {
        Scancode::Up => "Up".into(),
        Scancode::Down => "Down".into(),
        Scancode::Left => "Left".into(),
        Scancode::Right => "Right".into(),
        Scancode::Space => "Space".into(),
        Scancode::Return => "Enter".into(),
        Scancode::Escape => "Escape".into(),
        Scancode::Tab => "Tab".into(),
        Scancode::Backspace => "Backspace".into(),
        Scancode::Delete => "Delete".into(),
        Scancode::LShift => "LeftShift".into(),
        Scancode::RShift => "RightShift".into(),
        Scancode::LCtrl => "LeftCtrl".into(),
        Scancode::RCtrl => "RightCtrl".into(),
        Scancode::LAlt => "LeftAlt".into(),
        Scancode::RAlt => "RightAlt".into(),
        other => {
            let name = other.name();
            if name.chars().count() == 1 {
                name.to_lowercase()
            } else if is_function_key(name) {
                name.to_string()
            } else {
                return None;
            }
        }
    })
}

/// True for `F1` and up.
fn is_function_key(name: &str) -> bool {
    name.strip_prefix('F')
        .is_some_and(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
}

fn bound(config: &Config, scancode: Scancode) -> Option<Bound> {
    key_name(scancode).and_then(|name| config.bound(&name))
}

/// The keyjazz layout is defined by physical key position.
fn note_offset(scancode: Scancode) -> Option<u8> {
    let name = scancode.name();
    let mut chars = name.chars();
    let first = chars.next()?;
    chars
        .next()
        .is_none()
        .then(|| keys::note_offset(first))
        .flatten()
}

pub struct Input {
    /// Bitmask of currently held M8 buttons.
    held: u8,
    pub jazz: Keyjazz,
    /// Scancode of the key currently sounding a note, if any.
    sounding: Option<Scancode>,
    /// Keys holding a control-change slot down, so a release reaches its slot.
    cc_held: Vec<(Scancode, usize)>,
    /// The same two, for a controller, which is bound by name not scancode.
    pad_held: u8,
    pad_cc: Vec<(String, usize)>,
}

impl Input {
    pub fn new() -> Self {
        Self {
            held: 0,
            jazz: Keyjazz::default(),
            sounding: None,
            cc_held: Vec::new(),
            pad_held: 0,
            pad_cc: Vec::new(),
        }
    }

    /// Every M8 button currently held.
    pub fn keys(&self) -> u8 {
        self.held | self.pad_held
    }

    pub fn key_down(
        &mut self,
        config: &Config,
        scancode: Scancode,
        keymod: Mod,
        repeat: bool,
    ) -> Action {
        if repeat {
            return Action::None;
        }

        let alt = keymod.intersects(Mod::LALTMOD | Mod::RALTMOD);
        match scancode {
            Scancode::Return if alt => return Action::ToggleFullscreen,
            Scancode::F4 if alt => return Action::Quit,
            Scancode::Escape => return Action::OpenSettings,
            Scancode::Tab => {
                self.jazz.enabled = !self.jazz.enabled;
                if !self.jazz.enabled && self.sounding.take().is_some() {
                    return Action::NoteOff;
                }
                return Action::None;
            }
            _ => {}
        }

        if self.jazz.enabled {
            if let Some(offset) = note_offset(scancode) {
                self.sounding = Some(scancode);
                return Action::NoteOn(self.jazz.note(offset), self.jazz.velocity);
            }
            if is_keyjazz_setting(scancode) {
                self.adjust_keyjazz(scancode, alt);
                return Action::None;
            }
        }

        match bound(config, scancode) {
            Some(Bound::Cc(slot)) => {
                self.cc_held.push((scancode, slot));
                return Action::CcPress(slot);
            }
            Some(Bound::Button(button)) => self.held |= button,
            None => {}
        }
        Action::None
    }

    /// A controller button going down.
    pub fn pad_down(&mut self, config: &Config, name: &str) -> Action {
        match config.bound(name) {
            Some(Bound::Cc(slot)) => {
                self.pad_cc.push((name.to_string(), slot));
                Action::CcPress(slot)
            }
            Some(Bound::Button(button)) => {
                self.pad_held |= button;
                Action::None
            }
            None => Action::None,
        }
    }

    pub fn pad_up(&mut self, config: &Config, name: &str) -> Action {
        if let Some(at) = self.pad_cc.iter().position(|(held, _)| held == name) {
            let (_, slot) = self.pad_cc.remove(at);
            return Action::CcRelease(slot);
        }
        if let Some(Bound::Button(button)) = config.bound(name) {
            self.pad_held &= !button;
        }
        Action::None
    }

    pub fn key_up(&mut self, config: &Config, scancode: Scancode) -> Action {
        if self.sounding == Some(scancode) {
            self.sounding = None;
            return Action::NoteOff;
        }
        if let Some(at) = self.cc_held.iter().position(|(key, _)| *key == scancode) {
            let (_, slot) = self.cc_held.remove(at);
            return Action::CcRelease(slot);
        }
        if let Some(Bound::Button(button)) = bound(config, scancode) {
            self.held &= !button;
        }
        Action::None
    }

    /// Clears all held buttons, used when the window loses keyboard focus.
    pub fn release_all(&mut self) -> Vec<usize> {
        self.held = 0;
        self.pad_held = 0;
        self.cc_held
            .drain(..)
            .map(|(_, slot)| slot)
            .chain(self.pad_cc.drain(..).map(|(_, slot)| slot))
            .collect()
    }

    fn adjust_keyjazz(&mut self, scancode: Scancode, fine: bool) {
        match scancode {
            Scancode::KpDivide => self.jazz.octave_down(),
            Scancode::KpMultiply => self.jazz.octave_up(),
            Scancode::KpMinus => self.jazz.velocity_down(fine),
            Scancode::KpPlus => self.jazz.velocity_up(fine),
            _ => {}
        }
    }
}

fn is_keyjazz_setting(scancode: Scancode) -> bool {
    matches!(
        scancode,
        Scancode::KpDivide | Scancode::KpMultiply | Scancode::KpMinus | Scancode::KpPlus
    )
}
