//! The audio and MIDI sides of the app, which come and go with the M8 and so
//! are retried rather than treated as fatal.

use std::time::{Duration, Instant};

use crate::audio::Audio;
use crate::cc::{self, Engine};
use crate::config::Config;
use crate::midi;

/// How long to leave it before trying a missing device again.
const RETRY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct Outputs {
    audio: Option<Audio>,
    port: Option<midi::Port>,
    engine: Engine,
    /// What went wrong last, for the settings menu to show.
    message: Option<String>,
    last_retry: Option<Instant>,
}

impl Outputs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Brings the audio and the MIDI port into line with the settings.
    pub fn sync(&mut self, config: &Config) {
        if self.audio.as_ref().is_some_and(Audio::failed) {
            self.audio = None;
            self.message = Some("audio device went away".into());
        }
        if !config.audio {
            self.audio = None;
        }
        if self.port.as_ref().is_some_and(|port| !port.is_open()) {
            self.port = None;
            self.engine.clear();
        }
        match (&self.port, &config.midi_port) {
            (Some(port), Some(wanted)) if port.wanted() != wanted => self.port = None,
            (Some(_), None) => self.port = None,
            _ => {}
        }

        let wants_audio = config.audio && self.audio.is_none();
        let wants_midi = config.midi_port.is_some() && self.port.is_none();
        if !wants_audio && !wants_midi {
            return;
        }
        if self.last_retry.is_some_and(|at| at.elapsed() < RETRY) {
            return;
        }
        self.last_retry = Some(Instant::now());

        if wants_audio {
            match Audio::start() {
                Ok(audio) => {
                    self.message = None;
                    self.audio = Some(audio);
                }
                Err(e) => self.message = Some(e),
            }
        }
        if wants_midi {
            let wanted = config.midi_port.clone().unwrap_or_default();
            match midi::Port::open(&wanted) {
                Ok(port) => {
                    self.message = None;
                    self.port = Some(port);
                }
                Err(e) => self.message = Some(e),
            }
        }
    }

    /// Sends whatever a control-change binding's key going down calls for.
    pub fn press(&mut self, slot: usize, config: &Config) {
        let Some(binding) = config.cc.get(slot) else {
            return;
        };
        let message = self.engine.press(slot, binding, Instant::now());
        self.send(message);
    }

    /// And its key coming up, which for a gate is the way back.
    pub fn release(&mut self, slot: usize, config: &Config) {
        let Some(binding) = config.cc.get(slot) else {
            return;
        };
        let message = self.engine.release(slot, binding, Instant::now());
        self.send(message);
    }

    /// Lets every ramp in flight take another step.
    pub fn tick(&mut self) -> bool {
        if !self.engine.is_active() {
            return false;
        }
        for message in self.engine.tick(Instant::now()) {
            self.send(Some(message));
        }
        self.engine.is_active()
    }

    /// Lets go of the given slots, for when the app loses the keyboard.
    pub fn release_all(&mut self, slots: &[usize], config: &Config) {
        for slot in slots {
            self.release(*slot, config);
        }
    }

    fn send(&mut self, message: Option<cc::Message>) {
        let Some(message) = message else { return };
        let Some(port) = self.port.as_mut() else {
            return;
        };
        if let Err(e) = port.send(message) {
            self.message = Some(e);
        }
    }

    /// What the audio and MIDI are up to, for the settings menu's status line.
    pub fn status(&self, config: &Config) -> String {
        let mut parts = Vec::new();
        if config.audio {
            match &self.audio {
                Some(audio) => parts.push(format!("audio: {}", audio.description())),
                None => parts.push("audio: waiting for the M8".into()),
            }
        }
        if let Some(wanted) = &config.midi_port {
            match &self.port {
                Some(_) => parts.push(format!("MIDI: {wanted}")),
                None => parts.push(format!("MIDI: waiting for {wanted}")),
            }
        }
        if let Some(message) = &self.message {
            parts.push(message.clone());
        }
        parts.join("   ")
    }
}
