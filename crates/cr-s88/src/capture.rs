use crate::{InputTransport, S88Config, TransportStats};
use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const CAPTURE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureProtocol {
    Midi,
    UsbBulk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureEvent {
    /// Microseconds since the capture began. Events must be monotonic.
    pub at_us: u64,
    pub direction: CaptureDirection,
    pub bytes: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureFixture {
    pub schema_version: u16,
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
    pub protocol: CaptureProtocol,
    /// True only when the bytes came from a physical device or host trace.
    pub hardware_capture: bool,
    pub source: String,
    pub events: Vec<CaptureEvent>,
}

impl CaptureFixture {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let source = fs::read_to_string(path.as_ref())?;
        let fixture: Self = serde_norway::from_str(&source)?;
        fixture.validate()?;
        Ok(fixture)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        self.validate()?;
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let temporary = temporary_path(path);
        use std::io::Write;
        let mut file = fs::File::create(&temporary)?;
        file.write_all(serde_norway::to_string(self)?.as_bytes())?;
        file.sync_all()?;
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == CAPTURE_SCHEMA_VERSION,
            "unsupported capture schema version {}",
            self.schema_version
        );
        anyhow::ensure!(
            !self.device.trim().is_empty(),
            "capture device must not be empty"
        );
        anyhow::ensure!(
            !self.source.trim().is_empty(),
            "capture source must not be empty"
        );
        let mut previous = 0;
        for (index, event) in self.events.iter().enumerate() {
            anyhow::ensure!(!event.bytes.is_empty(), "event {index} has no bytes");
            anyhow::ensure!(
                index == 0 || event.at_us >= previous,
                "event {index} timestamp is not monotonic"
            );
            if self.protocol == CaptureProtocol::Midi {
                validate_midi_message(index, &event.bytes)?;
            }
            previous = event.at_us;
        }
        Ok(())
    }

    pub fn diff(&self, other: &Self) -> CaptureDiff {
        let mut metadata = Vec::new();
        if self.device != other.device {
            metadata.push(format!("device: {:?} != {:?}", self.device, other.device));
        }
        if self.firmware != other.firmware {
            metadata.push(format!(
                "firmware: {:?} != {:?}",
                self.firmware, other.firmware
            ));
        }
        if self.protocol != other.protocol {
            metadata.push(format!(
                "protocol: {:?} != {:?}",
                self.protocol, other.protocol
            ));
        }
        let count = self.events.len().max(other.events.len());
        let mut events = Vec::new();
        for index in 0..count {
            let left = self.events.get(index);
            let right = other.events.get(index);
            if left == right {
                continue;
            }
            let changed_byte_offsets = match (left, right) {
                (Some(left), Some(right)) => (0..left.bytes.len().max(right.bytes.len()))
                    .filter(|offset| left.bytes.get(*offset) != right.bytes.get(*offset))
                    .collect(),
                (Some(left), None) => (0..left.bytes.len()).collect(),
                (None, Some(right)) => (0..right.bytes.len()).collect(),
                (None, None) => Vec::new(),
            };
            events.push(EventDiff {
                index,
                left: left.cloned(),
                right: right.cloned(),
                changed_byte_offsets,
            });
        }
        CaptureDiff { metadata, events }
    }

    pub fn duration_us(&self) -> u64 {
        self.events.last().map_or(0, |event| event.at_us)
    }

    /// Summarize a captured USB bulk trace without interpreting or replaying
    /// any device command.
    pub fn analyze_usb(&self) -> anyhow::Result<UsbCaptureAnalysis> {
        anyhow::ensure!(
            self.protocol == CaptureProtocol::UsbBulk,
            "USB analysis requires a usb_bulk capture"
        );
        let mut lengths = BTreeMap::new();
        let mut input_events = 0;
        let mut output_events = 0;
        let mut length_header_matches = 0;
        let mut common_payload_prefix: Option<Vec<u8>> = None;
        for event in &self.events {
            *lengths.entry(event.bytes.len()).or_insert(0) += 1;
            match event.direction {
                CaptureDirection::Input => input_events += 1,
                CaptureDirection::Output => output_events += 1,
            }
            if event.bytes.len() >= 4 {
                let declared = u32::from_le_bytes([
                    event.bytes[0],
                    event.bytes[1],
                    event.bytes[2],
                    event.bytes[3],
                ]) as usize;
                if declared == event.bytes.len() - 4 {
                    length_header_matches += 1;
                }
                let payload = &event.bytes[4..];
                match &mut common_payload_prefix {
                    None => common_payload_prefix = Some(payload.to_vec()),
                    Some(prefix) => {
                        let shared = prefix
                            .iter()
                            .zip(payload)
                            .take_while(|(left, right)| left == right)
                            .count();
                        prefix.truncate(shared);
                    }
                }
            } else {
                common_payload_prefix = Some(Vec::new());
            }
        }
        Ok(UsbCaptureAnalysis {
            events: self.events.len(),
            input_events,
            output_events,
            lengths,
            length_header_matches,
            common_payload_prefix: common_payload_prefix.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbCaptureAnalysis {
    pub events: usize,
    pub input_events: usize,
    pub output_events: usize,
    pub lengths: BTreeMap<usize, usize>,
    /// Events whose first four little-endian bytes equal the remaining size.
    pub length_header_matches: usize,
    /// Bytes common to every transfer after the possible four-byte header.
    pub common_payload_prefix: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDiff {
    pub index: usize,
    pub left: Option<CaptureEvent>,
    pub right: Option<CaptureEvent>,
    pub changed_byte_offsets: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureDiff {
    pub metadata: Vec<String>,
    pub events: Vec<EventDiff>,
}

impl CaptureDiff {
    pub fn is_empty(&self) -> bool {
        self.metadata.is_empty() && self.events.is_empty()
    }
}

/// Replays input-direction events from a fixture through the normal adapter.
#[derive(Debug, Clone)]
pub struct CaptureReplayTransport {
    fixture: CaptureFixture,
    preserve_timing: bool,
}

impl CaptureReplayTransport {
    pub fn new(fixture: CaptureFixture) -> Self {
        Self {
            fixture,
            preserve_timing: false,
        }
    }

    pub fn preserving_timing(mut self, preserve: bool) -> Self {
        self.preserve_timing = preserve;
        self
    }
}

impl InputTransport for CaptureReplayTransport {
    fn run(
        self,
        _config: S88Config,
        events: Sender<Vec<u8>>,
        running: Arc<AtomicBool>,
        stats: Arc<TransportStats>,
    ) {
        let mut previous = 0;
        for event in self
            .fixture
            .events
            .iter()
            .filter(|event| event.direction == CaptureDirection::Input)
        {
            if !running.load(Ordering::Acquire) {
                break;
            }
            if self.preserve_timing {
                std::thread::sleep(Duration::from_micros(event.at_us.saturating_sub(previous)));
            }
            stats.record_received();
            if events.send(event.bytes.clone()).is_err() {
                break;
            }
            previous = event.at_us;
        }
    }
}

fn validate_midi_message(index: usize, bytes: &[u8]) -> anyhow::Result<()> {
    let status = bytes[0];
    anyhow::ensure!(status >= 0x80, "event {index} starts with a MIDI data byte");
    let valid_length = match status {
        0x80..=0x8f | 0x90..=0x9f | 0xa0..=0xaf | 0xb0..=0xbf | 0xe0..=0xef => bytes.len() == 3,
        0xc0..=0xdf => bytes.len() == 2,
        0xf0 => bytes.len() >= 2 && bytes.last() == Some(&0xf7),
        0xf1 | 0xf3 => bytes.len() == 2,
        0xf2 => bytes.len() == 3,
        0xf6 | 0xf8..=0xff => bytes.len() == 1,
        _ => false,
    };
    anyhow::ensure!(
        valid_length,
        "event {index} is not one complete MIDI message"
    );
    anyhow::ensure!(
        bytes[1..].iter().all(|byte| *byte < 0x80) || status == 0xf0,
        "event {index} contains an out-of-range MIDI data byte"
    );
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".tmp-{}", std::process::id()));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KnobBinding, S88Adapter};
    use cr_core::BindingId;

    fn fixture(value: u8) -> CaptureFixture {
        CaptureFixture {
            schema_version: CAPTURE_SCHEMA_VERSION,
            device: "Kontrol S88 MK3".into(),
            firmware: Some("unknown".into()),
            protocol: CaptureProtocol::Midi,
            hardware_capture: false,
            source: "unit test synthetic fixture".into(),
            events: vec![CaptureEvent {
                at_us: 12,
                direction: CaptureDirection::Input,
                bytes: vec![0xb0, 20, value],
                note: Some("knob 1".into()),
            }],
        }
    }

    #[test]
    fn yaml_round_trip_and_diff_report_changed_byte() {
        let left = fixture(64);
        let yaml = serde_norway::to_string(&left).unwrap();
        let decoded: CaptureFixture = serde_norway::from_str(&yaml).unwrap();
        assert_eq!(decoded, left);
        let diff = left.diff(&fixture(65));
        assert_eq!(diff.events[0].changed_byte_offsets, vec![2]);
    }

    #[test]
    fn rejects_non_monotonic_and_concatenated_midi_events() {
        let mut invalid = fixture(64);
        invalid.events.push(CaptureEvent {
            at_us: 1,
            direction: CaptureDirection::Input,
            bytes: vec![0xb0, 20, 64, 0xb0, 21, 65],
            note: None,
        });
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn fixture_replays_through_adapter() {
        let config = S88Config {
            enabled: true,
            port: "fixture".into(),
            channel: 1,
            queue_capacity: 8,
            reconnect_interval_ms: 1,
            knobs: vec![KnobBinding {
                id: BindingId::new("s88.knob.1"),
                cc: 20,
            }],
        };
        let adapter =
            S88Adapter::start_with_transport(config, CaptureReplayTransport::new(fixture(64)))
                .unwrap();
        let input = (0..100).find_map(|_| {
            let value = adapter.try_recv();
            if value.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            value
        });
        assert_eq!(input.unwrap().binding, BindingId::new("s88.knob.1"));
    }

    #[test]
    fn usb_analysis_reports_framing_without_interpreting_commands() {
        let fixture = CaptureFixture {
            schema_version: CAPTURE_SCHEMA_VERSION,
            device: "Kontrol S88 MK3".into(),
            firmware: Some("test".into()),
            protocol: CaptureProtocol::UsbBulk,
            hardware_capture: false,
            source: "synthetic framing test".into(),
            events: vec![
                CaptureEvent {
                    at_us: 0,
                    direction: CaptureDirection::Output,
                    bytes: vec![4, 0, 0, 0, 0x84, 0x00, 0x01, 0x02],
                    note: None,
                },
                CaptureEvent {
                    at_us: 1,
                    direction: CaptureDirection::Output,
                    bytes: vec![4, 0, 0, 0, 0x84, 0x00, 0x03, 0x04],
                    note: None,
                },
            ],
        };
        let analysis = fixture.analyze_usb().unwrap();
        assert_eq!(analysis.events, 2);
        assert_eq!(analysis.output_events, 2);
        assert_eq!(analysis.length_header_matches, 2);
        assert_eq!(analysis.common_payload_prefix, vec![0x84, 0x00]);
        assert_eq!(analysis.lengths.get(&8), Some(&2));
    }
}
