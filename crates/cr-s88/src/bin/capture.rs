use cr_s88::{CaptureDirection, CaptureEvent, CaptureFixture, CaptureProtocol, list_input_ports};
use midir::{Ignore, MidiInput};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("validate") if args.len() == 3 => validate(&args[2]),
        Some("summarize") if args.len() == 3 => summarize(&args[2]),
        Some("analyze-usb") if args.len() == 3 => analyze_usb(&args[2]),
        Some("diff") if args.len() == 4 => diff(&args[2], &args[3]),
        Some("verify-template") if args.len() >= 3 => verify_template(&args[2], &args[3..]),
        Some("list") if args.len() == 2 => list(),
        Some("record") => record(&args[2..]),
        _ => {
            eprintln!(
                "usage:\n  cr-s88-capture list\n  cr-s88-capture validate FILE\n  cr-s88-capture summarize FILE\n  cr-s88-capture analyze-usb FILE\n  cr-s88-capture diff LEFT RIGHT\n  cr-s88-capture verify-template FILE [--channel N] [--cc-start N]\n  cr-s88-capture record --port MATCH --output FILE [--seconds N] [--firmware VERSION]"
            );
            std::process::exit(2);
        }
    }
}

fn list() -> anyhow::Result<()> {
    let ports = list_input_ports()?;
    if ports.is_empty() {
        eprintln!("event=input_ports status=empty count=0");
    } else {
        for name in ports {
            println!("{name}");
        }
    }
    Ok(())
}

fn validate(path: &str) -> anyhow::Result<()> {
    let fixture = CaptureFixture::load(path)?;
    println!("valid: {} events", fixture.events.len());
    Ok(())
}

fn summarize(path: &str) -> anyhow::Result<()> {
    let fixture = CaptureFixture::load(path)?;
    let inputs = fixture
        .events
        .iter()
        .filter(|event| event.direction == CaptureDirection::Input)
        .count();
    println!("device: {}", fixture.device);
    println!(
        "firmware: {}",
        fixture.firmware.as_deref().unwrap_or("unknown")
    );
    println!("protocol: {:?}", fixture.protocol);
    println!("hardware capture: {}", fixture.hardware_capture);
    println!("events: {} input / {} total", inputs, fixture.events.len());
    println!("duration: {} us", fixture.duration_us());
    Ok(())
}

fn analyze_usb(path: &str) -> anyhow::Result<()> {
    let fixture = CaptureFixture::load(path)?;
    let analysis = fixture.analyze_usb()?;
    let prefix = analysis
        .common_payload_prefix
        .iter()
        .take(32)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    println!("events: {}", analysis.events);
    println!(
        "directions: {} input / {} output",
        analysis.input_events, analysis.output_events
    );
    println!("transfer lengths: {:?}", analysis.lengths);
    println!(
        "little-endian length header matches: {} / {}",
        analysis.length_header_matches, analysis.events
    );
    println!("common payload prefix after header: {prefix}");
    Ok(())
}

fn diff(left: &str, right: &str) -> anyhow::Result<()> {
    let left = CaptureFixture::load(left)?;
    let right = CaptureFixture::load(right)?;
    let diff = left.diff(&right);
    if diff.is_empty() {
        println!("captures are identical");
        return Ok(());
    }
    for metadata in diff.metadata {
        println!("metadata: {metadata}");
    }
    for event in diff.events {
        println!(
            "event {} differs at byte offsets {:?}: {:?} -> {:?}",
            event.index,
            event.changed_byte_offsets,
            event.left.as_ref().map(|value| &value.bytes),
            event.right.as_ref().map(|value| &value.bytes)
        );
    }
    std::process::exit(1);
}

fn verify_template(path: &str, args: &[String]) -> anyhow::Result<()> {
    let mut channel = 1u8;
    let mut cc_start = 20u8;
    let mut index = 0;
    while index < args.len() {
        let name = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("{name} requires a value"))?;
        match name.as_str() {
            "--channel" => channel = value.parse()?,
            "--cc-start" => cc_start = value.parse()?,
            _ => anyhow::bail!("unknown verify-template option {name:?}"),
        }
        index += 2;
    }
    anyhow::ensure!((1..=16).contains(&channel), "--channel must be in 1..=16");
    anyhow::ensure!(cc_start <= 120, "--cc-start must leave room for eight CCs");
    let fixture = CaptureFixture::load(path)?;
    verify_template_fixture(&fixture, channel, cc_start)
}

fn verify_template_fixture(
    fixture: &CaptureFixture,
    channel: u8,
    cc_start: u8,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        fixture.hardware_capture,
        "template verification requires hardware_capture: true"
    );
    let status = 0xb0 | (channel - 1);
    let mut missing = Vec::new();
    for offset in 0..8u8 {
        let cc = cc_start + offset;
        let values: Vec<_> = fixture
            .events
            .iter()
            .filter(|event| event.direction == CaptureDirection::Input)
            .filter_map(|event| match event.bytes.as_slice() {
                [event_status, event_cc, value]
                    if *event_status == status && *event_cc == cc && *value <= 127 =>
                {
                    Some(*value)
                }
                _ => None,
            })
            .collect();
        let has_min = values.iter().any(|value| *value <= 2);
        let has_mid = values.iter().any(|value| (60..=68).contains(value));
        let has_max = values.iter().any(|value| *value >= 125);
        println!(
            "knob={} channel={} cc={} events={} min={} mid={} max={}",
            offset + 1,
            channel,
            cc,
            values.len(),
            has_min,
            has_mid,
            has_max
        );
        if !(has_min && has_mid && has_max) {
            missing.push(format!(
                "knob {} (CC {cc}): min={has_min} mid={has_mid} max={has_max}",
                offset + 1
            ));
        }
    }
    anyhow::ensure!(
        missing.is_empty(),
        "hardware template capture is incomplete: {}",
        missing.join("; ")
    );
    println!("verified eight-knob hardware template capture");
    Ok(())
}

fn record(args: &[String]) -> anyhow::Result<()> {
    let options = RecordOptions::parse(args)?;
    let port_match = &options.port;
    let output = &options.output;

    let mut input = MidiInput::new("control-room-s88-capture")?;
    input.ignore(Ignore::None);
    let needle = port_match.to_lowercase();
    let matches: Vec<_> = input
        .ports()
        .into_iter()
        .filter_map(|port| {
            let name = input.port_name(&port).ok()?;
            name.to_lowercase()
                .contains(&needle)
                .then_some((port, name))
        })
        .collect();
    anyhow::ensure!(
        !matches.is_empty(),
        "no MIDI input port matches {port_match:?}"
    );
    anyhow::ensure!(
        matches.len() == 1,
        "MIDI input match {port_match:?} is ambiguous"
    );
    let (port, resolved_name) = matches.into_iter().next().unwrap();
    let started = Instant::now();
    let events = Arc::new(Mutex::new(Vec::new()));
    let callback_events = Arc::clone(&events);
    println!(
        "capturing {resolved_name:?} for {}s; input only, no bytes will be sent",
        options.seconds
    );
    let connection = input.connect(
        &port,
        "control-room-s88-capture-input",
        move |_timestamp, bytes, _| {
            callback_events.lock().unwrap().push(CaptureEvent {
                at_us: started.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
                direction: CaptureDirection::Input,
                bytes: bytes.to_vec(),
                note: None,
            });
        },
        (),
    )?;
    std::thread::sleep(Duration::from_secs(options.seconds));
    drop(connection);
    let events = Arc::try_unwrap(events).unwrap().into_inner().unwrap();
    let fixture = CaptureFixture {
        schema_version: 1,
        device: "Native Instruments Kontrol S88 MK3".into(),
        firmware: options.firmware,
        protocol: CaptureProtocol::Midi,
        hardware_capture: true,
        source: format!("midir input port {resolved_name}"),
        events,
    };
    fixture.save(Path::new(output))?;
    println!("saved {} events to {output}", fixture.events.len());
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct RecordOptions {
    port: String,
    output: String,
    seconds: u64,
    firmware: Option<String>,
}

impl RecordOptions {
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut port = None;
        let mut output = None;
        let mut seconds = 10;
        let mut firmware = None;
        let mut index = 0;
        while index < args.len() {
            let name = &args[index];
            let value = args
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| anyhow::anyhow!("{name} requires a value"))?;
            match name.as_str() {
                "--port" if port.is_none() => port = Some(value.clone()),
                "--output" if output.is_none() => output = Some(value.clone()),
                "--seconds" => seconds = value.parse::<u64>()?,
                "--firmware" if firmware.is_none() => firmware = Some(value.clone()),
                "--port" | "--output" | "--firmware" => {
                    anyhow::bail!("duplicate record option {name}")
                }
                _ => anyhow::bail!("unknown record option {name:?}"),
            }
            index += 2;
        }
        anyhow::ensure!(seconds > 0, "--seconds must be greater than zero");
        Ok(Self {
            port: port.ok_or_else(|| anyhow::anyhow!("missing required --port"))?,
            output: output.ok_or_else(|| anyhow::anyhow!("missing required --output"))?,
            seconds,
            firmware,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn record_options_are_strict_and_default_to_ten_seconds() {
        assert_eq!(
            RecordOptions::parse(&args(&["--port", "S88", "--output", "capture.yaml"])).unwrap(),
            RecordOptions {
                port: "S88".into(),
                output: "capture.yaml".into(),
                seconds: 10,
                firmware: None,
            }
        );
        assert!(RecordOptions::parse(&args(&["--port", "S88", "--wat", "x"])).is_err());
        assert!(RecordOptions::parse(&args(&["--port", "--output", "x"])).is_err());
    }

    #[test]
    fn template_verification_requires_hardware_and_all_three_positions() {
        let events = (0..8u8)
            .flat_map(|offset| [0u8, 64, 127].map(move |value| (offset, value)))
            .enumerate()
            .map(|(index, (offset, value))| CaptureEvent {
                at_us: index as u64,
                direction: CaptureDirection::Input,
                bytes: vec![0xb0, 20 + offset, value],
                note: None,
            })
            .collect();
        let mut fixture = CaptureFixture {
            schema_version: 1,
            device: "Native Instruments Kontrol S88 MK3".into(),
            firmware: Some("test".into()),
            protocol: CaptureProtocol::Midi,
            hardware_capture: true,
            source: "test hardware".into(),
            events,
        };
        assert!(verify_template_fixture(&fixture, 1, 20).is_ok());
        fixture.events.pop();
        assert!(verify_template_fixture(&fixture, 1, 20).is_err());
        fixture.hardware_capture = false;
        assert!(verify_template_fixture(&fixture, 1, 20).is_err());
    }
}
