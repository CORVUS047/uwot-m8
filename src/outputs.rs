//! The audio and MIDI sides of the app, which come and go with the M8 and so
//! are retried rather than treated as fatal.

use std::time::{Duration, Instant};

use crate::audio::{self, Audio, Leg};
use crate::cc::{self, Engine};
use crate::config::Config;
use crate::midi;

/// How long to leave it before trying a missing device again.
const RETRY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct Outputs {
    audio: Option<Audio>,
    /// The other way round: a device here played into the M8.
    send: Option<Audio>,
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
        let play_devices = (
            config.audio_input.as_deref(),
            config.audio_output.as_deref(),
        );
        let send_devices = (
            config.audio_send_input.as_deref(),
            config.audio_send_output.as_deref(),
        );
        // An application sent into the M8 is taken off this computer's own
        // output while the M8 is played back here, or it would be heard twice:
        // once straight from the application, once through the M8.
        let exclusive = config.audio;
        if Self::stale(&mut self.audio, config.audio, play_devices, false) {
            self.message = Some("audio device went away".into());
        }
        if Self::stale(&mut self.send, config.audio_send, send_devices, exclusive) {
            self.message = Some("what was being sent to the M8 went away".into());
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
        let wants_send = config.audio_send && self.send.is_none();
        let wants_midi = config.midi_port.is_some() && self.port.is_none();
        if !wants_audio && !wants_send && !wants_midi {
            return;
        }
        if self.last_retry.is_some_and(|at| at.elapsed() < RETRY) {
            return;
        }
        self.last_retry = Some(Instant::now());

        if wants_audio {
            match Audio::start(Leg::Play, play_devices.0, play_devices.1, false) {
                Ok(audio) => {
                    self.message = None;
                    self.audio = Some(audio);
                }
                Err(e) => self.message = Some(e),
            }
        }
        if wants_send {
            match Audio::start(Leg::Send, send_devices.0, send_devices.1, exclusive) {
                Ok(audio) => {
                    self.message = None;
                    self.send = Some(audio);
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

    /// Drops a passthrough that has failed, been turned off, or had its devices
    /// changed under it, saying whether it was a failure that took it.
    fn stale(
        leg: &mut Option<Audio>,
        wanted: bool,
        devices: (Option<&str>, Option<&str>),
        exclusive: bool,
    ) -> bool {
        let failed = leg.as_ref().is_some_and(Audio::failed);
        if failed
            || !wanted
            || leg
                .as_ref()
                .is_some_and(|audio| !audio.wants(devices.0, devices.1, exclusive))
        {
            *leg = None;
        }
        failed
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
                None => parts.push(format!(
                    "audio: waiting for {}",
                    config.audio_input.as_deref().unwrap_or("the M8")
                )),
            }
        }
        if config.audio_send {
            match &self.send {
                Some(send) => parts.push(format!("sending: {}", send.description())),
                // What is missing is far more often the thing being sent than
                // the M8, so name that when one was picked.
                None => parts.push(format!("sending: waiting for {}", send_source(config))),
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

/// What the send leg is listening to, named the way a person would.
fn send_source(config: &Config) -> &str {
    match &config.audio_send_input {
        Some(name) => name.strip_prefix(audio::APP_PREFIX).unwrap_or(name),
        None => config.audio_send_output.as_deref().unwrap_or("the M8"),
    }
}
