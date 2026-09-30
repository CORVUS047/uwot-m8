//! The MIDI output the control-change bindings send to.

use midir::{MidiOutput, MidiOutputConnection};

use crate::cc::Message;

/// The name the app appears under in a patchbay.
const CLIENT: &str = "uwot-m8";

/// Ports whose name contains this are the M8's own, and are picked first.
const PREFERRED: &str = "m8";

/// Every MIDI output the system currently offers, in the order it offers them.
pub fn ports() -> Vec<String> {
    let Ok(midi) = MidiOutput::new(CLIENT) else { return Vec::new() };
    midi.ports().iter().filter_map(|port| midi.port_name(port).ok()).collect()
}

/// The port a fresh config should use: the M8's own if it is there.
pub fn default_port() -> Option<String> {
    let ports = ports();
    ports
        .iter()
        .find(|name| name.to_lowercase().contains(PREFERRED))
        .or_else(|| ports.first())
        .cloned()
}

/// A connection to one MIDI output, reopened on demand.
pub struct Port {
    connection: Option<MidiOutputConnection>,
    /// The name asked for, kept so the port can be reopened after it fails.
    wanted: String,
}

impl Port {
    /// Opens the port whose name matches `wanted` exactly.
    pub fn open(wanted: &str) -> Result<Self, String> {
        let midi = MidiOutput::new(CLIENT).map_err(|e| format!("no MIDI available: {e}"))?;
        let ports = midi.ports();
        let named: Vec<(usize, String)> = ports
            .iter()
            .enumerate()
            .filter_map(|(index, port)| Some((index, midi.port_name(port).ok()?)))
            .collect();

        let needle = wanted.to_lowercase();
        let found = named
            .iter()
            .find(|(_, name)| *name == wanted)
            .or_else(|| named.iter().find(|(_, name)| name.to_lowercase().contains(&needle)))
            .ok_or_else(|| format!("no MIDI port matching {wanted}"))?;

        let connection = midi
            .connect(&ports[found.0], CLIENT)
            .map_err(|e| format!("cannot open MIDI port {}: {e}", found.1))?;
        Ok(Self { connection: Some(connection), wanted: wanted.to_string() })
    }

    pub fn is_open(&self) -> bool {
        self.connection.is_some()
    }

    /// The port name this was opened for, for the settings menu to show.
    pub fn wanted(&self) -> &str {
        &self.wanted
    }

    /// Sends one control change.
    pub fn send(&mut self, message: Message) -> Result<(), String> {
        let Some(connection) = self.connection.as_mut() else {
            return Err("MIDI port is closed".into());
        };
        match connection.send(&message.bytes()) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.connection = None;
                Err(format!("MIDI send failed: {e}"))
            }
        }
    }
}
