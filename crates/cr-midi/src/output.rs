//! Output sinks. `CaptureSink` makes the whole pipeline testable without
//! hardware; `MidirSink` talks to a real port.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MidiError {
    #[error("could not initialise midi backend: {0}")]
    Init(String),
    #[error("no midi output port matching {0:?}; available: {1:?}")]
    NoMatchingPort(String, Vec<String>),
    #[error("could not connect to midi port: {0}")]
    Connect(String),
    #[error("could not send midi bytes: {0}")]
    Send(String),
}

pub trait MidiSink {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError>;
}

/// Records everything sent. Used by tests and by `--dry-run`.
#[derive(Debug, Default)]
pub struct CaptureSink {
    pub messages: Vec<Vec<u8>>,
}

impl CaptureSink {
    pub fn new() -> Self {
        Self::default()
    }
    /// All captured bytes flattened, for byte-exact stream assertions.
    pub fn flat(&self) -> Vec<u8> {
        self.messages.iter().flatten().copied().collect()
    }
}

impl MidiSink for CaptureSink {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
        self.messages.push(bytes.to_vec());
        Ok(())
    }
}

/// A real MIDI output port.
pub struct MidirSink {
    conn: midir::MidiOutputConnection,
}

impl std::fmt::Debug for MidirSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidirSink")
            .field("conn", &"MidiOutputConnection")
            .finish()
    }
}

impl MidirSink {
    pub fn list_ports() -> Result<Vec<String>, MidiError> {
        let out =
            midir::MidiOutput::new("control-room").map_err(|e| MidiError::Init(e.to_string()))?;
        Ok(out
            .ports()
            .iter()
            .filter_map(|p| out.port_name(p).ok())
            .collect())
    }

    /// Open the first output port whose name contains `port_substring`
    /// (case-insensitive).
    pub fn open(port_substring: &str) -> Result<Self, MidiError> {
        let out =
            midir::MidiOutput::new("control-room").map_err(|e| MidiError::Init(e.to_string()))?;
        let needle = port_substring.to_lowercase();

        let ports = out.ports();
        let mut names = Vec::with_capacity(ports.len());
        let mut found = None;
        for p in &ports {
            let name = out.port_name(p).unwrap_or_default();
            if found.is_none() && name.to_lowercase().contains(&needle) {
                found = Some(p.clone());
            }
            names.push(name);
        }

        let port =
            found.ok_or_else(|| MidiError::NoMatchingPort(port_substring.to_string(), names))?;
        let conn = out
            .connect(&port, "control-room-out")
            .map_err(|e| MidiError::Connect(e.to_string()))?;
        Ok(Self { conn })
    }
}

impl MidiSink for MidirSink {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
        self.conn
            .send(bytes)
            .map_err(|e| MidiError::Send(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_sink_records_each_message_separately() {
        let mut s = CaptureSink::new();
        s.send(&[0xB0, 29, 64]).unwrap();
        s.send(&[0xB0, 29, 65]).unwrap();
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[0], vec![0xB0, 29, 64]);
    }

    #[test]
    fn capture_sink_flattens_for_stream_assertions() {
        let mut s = CaptureSink::new();
        s.send(&[0xB0, 1, 127]).unwrap();
        s.send(&[0xB0, 33, 127]).unwrap();
        assert_eq!(s.flat(), vec![0xB0, 1, 127, 0xB0, 33, 127]);
    }

    #[test]
    fn listing_ports_does_not_error_on_this_host() {
        // Does not assert on contents: a CI box may have zero MIDI ports.
        let _ = MidirSink::list_ports().expect("port enumeration should work");
    }

    #[test]
    fn opening_a_nonexistent_port_reports_available_ports() {
        let err = MidirSink::open("definitely-not-a-real-port-xyz").unwrap_err();
        assert!(matches!(err, MidiError::NoMatchingPort(_, _)));
    }
}
