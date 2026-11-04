use cr_s88::{CaptureDirection, CaptureEvent, CaptureFixture, CaptureProtocol, list_input_ports};
use midir::{Ignore, MidiInput};
use serde_json::Value;
use std::collections::BTreeSet;
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
        Some("import-usb") if args.len() >= 3 => import_usb(&args[2..]),
        _ => {
            eprintln!(
                "usage:\n  cr-s88-capture list\n  cr-s88-capture validate FILE\n  cr-s88-capture summarize FILE\n  cr-s88-capture analyze-usb FILE\n  cr-s88-capture diff LEFT RIGHT\n  cr-s88-capture verify-template FILE [--channel N] [--cc-start N]\n  cr-s88-capture record --port MATCH --output FILE [--seconds N] [--firmware VERSION]\n  cr-s88-capture import-usb INPUT.json --output FILE --device NAME [--firmware VERSION] [--source TEXT] [--hardware-capture] [--all-transfer-types]"
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

/// Field names tried, in priority order, when hunting for each mapped value in
/// a tshark `-T json` USB capture export. None of these are confirmed against
/// a real capture (see `docs/s88-protocol-notebook.md`); they are educated
/// guesses that are validated against the input's actual shape at import
/// time, with an actionable error if none match.
const TIME_FIELD_CANDIDATES: &[&str] = &[
    "frame.time_relative",
    "frame.time_epoch",
    "frame.time_delta_displayed",
    "frame.time_delta",
];

const DIRECTION_BIT_CANDIDATES: &[&str] = &[
    "usb.endpoint_address.direction",
    "usb.endpoint_address_direction",
];

const DIRECTION_ENDPOINT_CANDIDATES: &[&str] = &["usb.dst", "usb.src"];

const PAYLOAD_FIELD_CANDIDATES: &[&str] = &["usb.capdata", "usb.payload", "usb.data_fragment"];

const TRANSFER_TYPE_FIELD_CANDIDATES: &[&str] = &["usb.transfer_type"];

fn import_usb(args: &[String]) -> anyhow::Result<()> {
    let options = ImportUsbOptions::parse(args)?;
    let fixture = build_usb_fixture(&options)?;
    fixture.save(Path::new(&options.output))?;
    println!(
        "saved {} usb_bulk events to {}",
        fixture.events.len(),
        options.output
    );
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ImportUsbOptions {
    input: String,
    output: String,
    device: String,
    firmware: Option<String>,
    source: String,
    hardware_capture: bool,
    all_transfer_types: bool,
}

impl ImportUsbOptions {
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let input = args
            .first()
            .ok_or_else(|| anyhow::anyhow!("import-usb requires an input file"))?
            .clone();
        let mut output = None;
        let mut device = None;
        let mut firmware = None;
        let mut source = None;
        let mut hardware_capture = false;
        let mut all_transfer_types = false;
        let mut index = 1;
        while index < args.len() {
            let name = &args[index];
            match name.as_str() {
                "--hardware-capture" => {
                    hardware_capture = true;
                    index += 1;
                }
                "--all-transfer-types" => {
                    all_transfer_types = true;
                    index += 1;
                }
                "--output" | "--device" | "--firmware" | "--source" => {
                    let value = args
                        .get(index + 1)
                        .ok_or_else(|| anyhow::anyhow!("{name} requires a value"))?;
                    match name.as_str() {
                        "--output" if output.is_none() => output = Some(value.clone()),
                        "--device" if device.is_none() => device = Some(value.clone()),
                        "--firmware" if firmware.is_none() => firmware = Some(value.clone()),
                        "--source" if source.is_none() => source = Some(value.clone()),
                        _ => anyhow::bail!("duplicate import-usb option {name}"),
                    }
                    index += 2;
                }
                _ => anyhow::bail!("unknown import-usb option {name:?}"),
            }
        }
        let output = output.ok_or_else(|| anyhow::anyhow!("missing required --output"))?;
        let device = device.ok_or_else(|| anyhow::anyhow!("missing required --device"))?;
        let source = source.unwrap_or_else(|| format!("tshark USB capture export {input}"));
        Ok(Self {
            input,
            output,
            device,
            firmware,
            source,
            hardware_capture,
            all_transfer_types,
        })
    }
}

fn build_usb_fixture(options: &ImportUsbOptions) -> anyhow::Result<CaptureFixture> {
    let raw = std::fs::read_to_string(&options.input)
        .map_err(|error| anyhow::anyhow!("reading {}: {error}", options.input))?;
    let packets: Vec<Value> = serde_json::from_str(&raw).map_err(|error| {
        anyhow::anyhow!(
            "parsing {} as a tshark -T json export (expected a JSON array of packet objects): {error}",
            options.input
        )
    })?;
    build_usb_fixture_from_packets(&packets, options)
}

fn build_usb_fixture_from_packets(
    packets: &[Value],
    options: &ImportUsbOptions,
) -> anyhow::Result<CaptureFixture> {
    let first_packet = packets
        .first()
        .ok_or_else(|| anyhow::anyhow!("capture export contains no packets"))?;
    let first_layers = packet_layers(first_packet);

    if !options.all_transfer_types
        && find_field(first_layers, TRANSFER_TYPE_FIELD_CANDIDATES).is_none()
    {
        return Err(field_lookup_error(
            "transfer type",
            TRANSFER_TYPE_FIELD_CANDIDATES,
            first_layers,
        ));
    }
    if find_field(first_layers, TIME_FIELD_CANDIDATES).is_none() {
        return Err(field_lookup_error(
            "timestamp",
            TIME_FIELD_CANDIDATES,
            first_layers,
        ));
    }
    if find_field(first_layers, DIRECTION_BIT_CANDIDATES).is_none()
        && find_field(first_layers, DIRECTION_ENDPOINT_CANDIDATES).is_none()
    {
        let mut direction_candidates = DIRECTION_BIT_CANDIDATES.to_vec();
        direction_candidates.extend_from_slice(DIRECTION_ENDPOINT_CANDIDATES);
        return Err(field_lookup_error(
            "direction",
            &direction_candidates,
            first_layers,
        ));
    }
    if find_field(first_layers, PAYLOAD_FIELD_CANDIDATES).is_none() {
        return Err(field_lookup_error(
            "payload",
            PAYLOAD_FIELD_CANDIDATES,
            first_layers,
        ));
    }

    let mut raw_events: Vec<(f64, CaptureDirection, Vec<u8>)> = Vec::new();
    for (index, packet) in packets.iter().enumerate() {
        let layers = packet_layers(packet);

        if !options.all_transfer_types {
            match transfer_type_is_bulk(layers) {
                Some(true) => {}
                Some(false) | None => continue,
            }
        }

        let seconds = find_scalar(layers, TIME_FIELD_CANDIDATES)
            .and_then(|raw| raw.trim().parse::<f64>().ok())
            .ok_or_else(|| anyhow::anyhow!("packet {index}: could not parse a timestamp"))?;

        let direction = extract_direction(layers).ok_or_else(|| {
            anyhow::anyhow!("packet {index}: could not determine transfer direction")
        })?;

        let Some(payload_raw) = find_scalar(layers, PAYLOAD_FIELD_CANDIDATES) else {
            continue;
        };
        let payload_raw = payload_raw.trim();
        if payload_raw.is_empty() {
            continue;
        }
        let bytes = parse_hex_bytes(payload_raw).map_err(|error| {
            anyhow::anyhow!(
                "packet {index}: payload {payload_raw:?} is not colon-separated hex: {error}"
            )
        })?;
        if bytes.is_empty() {
            continue;
        }

        raw_events.push((seconds, direction, bytes));
    }

    anyhow::ensure!(
        !raw_events.is_empty(),
        "no usable USB bulk transfers with payload were found in {}",
        options.input
    );

    raw_events.sort_by(|left, right| {
        left.0
            .partial_cmp(&right.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let base = raw_events
        .first()
        .map(|(seconds, _, _)| *seconds)
        .unwrap_or(0.0);
    let events = raw_events
        .into_iter()
        .map(|(seconds, direction, bytes)| CaptureEvent {
            at_us: ((seconds - base) * 1_000_000.0).max(0.0).round() as u64,
            direction,
            bytes,
            note: None,
        })
        .collect();

    Ok(CaptureFixture {
        schema_version: 1,
        device: options.device.clone(),
        firmware: options.firmware.clone(),
        protocol: CaptureProtocol::UsbBulk,
        hardware_capture: options.hardware_capture,
        source: options.source.clone(),
        events,
    })
}

/// tshark's `-T json` export nests dissected fields under `_source.layers`.
/// Fall back to the packet itself so a differently-shaped export (or a test
/// fixture) is still searched rather than rejected outright.
fn packet_layers(packet: &Value) -> &Value {
    packet
        .get("_source")
        .and_then(|source| source.get("layers"))
        .unwrap_or(packet)
}

/// Depth-first search for a JSON object key exactly matching `name`, anywhere
/// under `value`. This deliberately does not assume a fixed nesting depth,
/// because tshark's JSON shape for USB dissection has not been verified
/// against a real capture.
fn find_field<'a>(value: &'a Value, candidates: &[&str]) -> Option<&'a Value> {
    for &candidate in candidates {
        if let Some(found) = find_field_named(value, candidate) {
            return Some(found);
        }
    }
    None
}

fn find_field_named<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => {
            if let Some(found) = map.get(name) {
                return Some(found);
            }
            map.values().find_map(|child| find_field_named(child, name))
        }
        Value::Array(items) => items.iter().find_map(|item| find_field_named(item, name)),
        _ => None,
    }
}

fn find_scalar(value: &Value, candidates: &[&str]) -> Option<String> {
    find_field(value, candidates).and_then(scalar_str)
}

/// Renders a tshark field value as a plain string. tshark's JSON export
/// usually gives scalars as strings, but this tolerates numbers, booleans and
/// single-element arrays too.
fn scalar_str(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Array(items) => items.first().and_then(scalar_str),
        _ => None,
    }
}

fn collect_field_names(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                out.insert(key.clone());
                collect_field_names(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_field_names(item, out);
            }
        }
        _ => {}
    }
}

fn field_lookup_error(what: &str, candidates: &[&str], first_layers: &Value) -> anyhow::Error {
    let mut observed = BTreeSet::new();
    collect_field_names(first_layers, &mut observed);
    let observed = if observed.is_empty() {
        "(none)".to_string()
    } else {
        observed.into_iter().collect::<Vec<_>>().join(", ")
    };
    anyhow::anyhow!(
        "could not find a {what} field in the first packet.\n  tried: {}\n  fields present in the first packet: {observed}",
        candidates.join(", ")
    )
}

fn parse_int_maybe_hex(raw: &str) -> Option<i64> {
    if let Some(stripped) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        i64::from_str_radix(stripped, 16).ok()
    } else {
        raw.parse::<i64>().ok()
    }
}

fn transfer_type_is_bulk(layers: &Value) -> Option<bool> {
    let raw = find_scalar(layers, TRANSFER_TYPE_FIELD_CANDIDATES)?;
    let raw = raw.trim();
    if let Some(code) = parse_int_maybe_hex(raw) {
        return Some(code == 3);
    }
    let lowered = raw.to_ascii_lowercase();
    if lowered.contains("bulk") {
        return Some(true);
    }
    if lowered.contains("control") || lowered.contains("interrupt") || lowered.contains("iso") {
        return Some(false);
    }
    None
}

fn parse_direction_bit(value: &Value) -> Option<CaptureDirection> {
    let raw = scalar_str(value)?;
    let raw = raw.trim();
    if let Some(code) = parse_int_maybe_hex(raw) {
        return Some(if code != 0 {
            CaptureDirection::Input
        } else {
            CaptureDirection::Output
        });
    }
    match raw.to_ascii_lowercase().as_str() {
        "in" => Some(CaptureDirection::Input),
        "out" => Some(CaptureDirection::Output),
        _ => None,
    }
}

/// Maps host<->device direction onto [`CaptureDirection`]: host->device (OUT)
/// is `Output`, device->host (IN) is `Input`. Tries the endpoint direction bit
/// first, then falls back to the `usb.src`/`usb.dst` "host" convention.
fn extract_direction(layers: &Value) -> Option<CaptureDirection> {
    for &candidate in DIRECTION_BIT_CANDIDATES {
        if let Some(value) = find_field_named(layers, candidate)
            && let Some(direction) = parse_direction_bit(value)
        {
            return Some(direction);
        }
    }
    for &candidate in DIRECTION_ENDPOINT_CANDIDATES {
        if let Some(value) = find_field_named(layers, candidate)
            && let Some(raw) = scalar_str(value)
        {
            let is_host = raw.trim().eq_ignore_ascii_case("host");
            return Some(match (candidate, is_host) {
                ("usb.dst", true) => CaptureDirection::Input,
                ("usb.dst", false) => CaptureDirection::Output,
                (_, true) => CaptureDirection::Output,
                (_, false) => CaptureDirection::Input,
            });
        }
    }
    None
}

fn parse_hex_bytes(raw: &str) -> anyhow::Result<Vec<u8>> {
    raw.split(':')
        .filter(|part| !part.is_empty())
        .map(|part| {
            u8::from_str_radix(part, 16)
                .map_err(|error| anyhow::anyhow!("{part:?} is not a hex byte: {error}"))
        })
        .collect()
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

    const WELL_FORMED_USB_JSON: &str = r#"[
      {
        "_source": {
          "layers": {
            "frame": { "frame.time_relative": "0.000000" },
            "usb": {
              "usb.transfer_type": "0x03",
              "usb.endpoint_address_tree": { "usb.endpoint_address.direction": "0" },
              "usb.capdata": "84:00:00:60"
            }
          }
        }
      },
      {
        "_source": {
          "layers": {
            "frame": { "frame.time_relative": "0.001500" },
            "usb": {
              "usb.transfer_type": "0x03",
              "usb.endpoint_address_tree": { "usb.endpoint_address.direction": "1" },
              "usb.capdata": "01:02:03"
            }
          }
        }
      }
    ]"#;

    fn import_options(all_transfer_types: bool, hardware_capture: bool) -> ImportUsbOptions {
        ImportUsbOptions {
            input: "capture.json".into(),
            output: "out.yaml".into(),
            device: "Kontrol S88 MK3".into(),
            firmware: None,
            source: "test capture".into(),
            hardware_capture,
            all_transfer_types,
        }
    }

    fn packets_from(json: &str) -> Vec<Value> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn import_usb_options_require_output_and_device_and_default_source() {
        let parsed = ImportUsbOptions::parse(&args(&[
            "capture.json",
            "--output",
            "out.yaml",
            "--device",
            "Kontrol S88 MK3",
        ]))
        .unwrap();
        assert_eq!(parsed.output, "out.yaml");
        assert_eq!(parsed.device, "Kontrol S88 MK3");
        assert!(!parsed.hardware_capture);
        assert!(!parsed.all_transfer_types);
        assert_eq!(parsed.source, "tshark USB capture export capture.json");

        assert!(ImportUsbOptions::parse(&args(&["capture.json", "--output", "out.yaml"])).is_err());

        let with_flags = ImportUsbOptions::parse(&args(&[
            "capture.json",
            "--output",
            "out.yaml",
            "--device",
            "S88",
            "--hardware-capture",
            "--all-transfer-types",
        ]))
        .unwrap();
        assert!(with_flags.hardware_capture);
        assert!(with_flags.all_transfer_types);
    }

    #[test]
    fn import_usb_maps_time_direction_and_bytes() {
        let packets = packets_from(WELL_FORMED_USB_JSON);
        let fixture =
            build_usb_fixture_from_packets(&packets, &import_options(false, false)).unwrap();
        assert_eq!(fixture.protocol, CaptureProtocol::UsbBulk);
        assert_eq!(fixture.events.len(), 2);
        assert_eq!(fixture.events[0].at_us, 0);
        assert_eq!(fixture.events[0].bytes, vec![0x84, 0x00, 0x00, 0x60]);
        assert_eq!(fixture.events[1].at_us, 1500);
        assert_eq!(fixture.events[1].bytes, vec![0x01, 0x02, 0x03]);
        fixture.validate().unwrap();
    }

    #[test]
    fn direction_maps_host_to_device_as_output_and_device_to_host_as_input() {
        let packets = packets_from(WELL_FORMED_USB_JSON);
        let fixture =
            build_usb_fixture_from_packets(&packets, &import_options(false, false)).unwrap();
        assert_eq!(fixture.events[0].direction, CaptureDirection::Output);
        assert_eq!(fixture.events[1].direction, CaptureDirection::Input);

        // Same mapping via the usb.src/usb.dst "host" fallback, when no
        // endpoint direction bit is present.
        let src_dst_json = r#"[
          { "_source": { "layers": { "frame": { "frame.time_relative": "0.0" },
              "usb": { "usb.transfer_type": "0x03", "usb.src": "host", "usb.dst": "1.6.1", "usb.capdata": "aa:bb" } } } },
          { "_source": { "layers": { "frame": { "frame.time_relative": "0.1" },
              "usb": { "usb.transfer_type": "0x03", "usb.src": "1.6.1", "usb.dst": "host", "usb.capdata": "cc:dd" } } } }
        ]"#;
        let packets = packets_from(src_dst_json);
        let fixture =
            build_usb_fixture_from_packets(&packets, &import_options(false, false)).unwrap();
        assert_eq!(fixture.events[0].direction, CaptureDirection::Output);
        assert_eq!(fixture.events[1].direction, CaptureDirection::Input);
    }

    #[test]
    fn non_bulk_transfers_are_filtered_out_by_default_but_kept_with_the_escape_hatch() {
        let json = r#"[
          { "_source": { "layers": { "frame": { "frame.time_relative": "0.0" },
              "usb": { "usb.transfer_type": "0x03",
                       "usb.endpoint_address_tree": { "usb.endpoint_address.direction": "0" },
                       "usb.capdata": "01:02" } } } },
          { "_source": { "layers": { "frame": { "frame.time_relative": "0.001" },
              "usb": { "usb.transfer_type": "0x02",
                       "usb.endpoint_address_tree": { "usb.endpoint_address.direction": "0" },
                       "usb.capdata": "03:04" } } } }
        ]"#;
        let packets = packets_from(json);
        let default_filtered =
            build_usb_fixture_from_packets(&packets, &import_options(false, false)).unwrap();
        assert_eq!(default_filtered.events.len(), 1);
        assert_eq!(default_filtered.events[0].bytes, vec![0x01, 0x02]);

        let all_kept =
            build_usb_fixture_from_packets(&packets, &import_options(true, false)).unwrap();
        assert_eq!(all_kept.events.len(), 2);
    }

    #[test]
    fn packets_with_no_payload_are_skipped() {
        let json = r#"[
          { "_source": { "layers": { "frame": { "frame.time_relative": "0.0" },
              "usb": { "usb.transfer_type": "0x03",
                       "usb.endpoint_address_tree": { "usb.endpoint_address.direction": "0" },
                       "usb.capdata": "01:02" } } } },
          { "_source": { "layers": { "frame": { "frame.time_relative": "0.001" },
              "usb": { "usb.transfer_type": "0x03",
                       "usb.endpoint_address_tree": { "usb.endpoint_address.direction": "0" },
                       "usb.capdata": "" } } } }
        ]"#;
        let packets = packets_from(json);
        let fixture =
            build_usb_fixture_from_packets(&packets, &import_options(false, false)).unwrap();
        assert_eq!(fixture.events.len(), 1);
    }

    #[test]
    fn unrecognized_field_shape_produces_an_actionable_error() {
        let json = r#"[ { "foo": "bar", "baz": 1 } ]"#;
        let packets = packets_from(json);
        // Bypass the transfer-type check so the failure is about the
        // timestamp candidates specifically.
        let error = build_usb_fixture_from_packets(&packets, &import_options(true, false))
            .unwrap_err()
            .to_string();
        for candidate in TIME_FIELD_CANDIDATES {
            assert!(
                error.contains(candidate),
                "error {error:?} should mention tried candidate {candidate:?}"
            );
        }
        assert!(
            error.contains("foo"),
            "error {error:?} should list observed field `foo`"
        );
        assert!(
            error.contains("baz"),
            "error {error:?} should list observed field `baz`"
        );
    }

    #[test]
    fn import_usb_output_validates_and_round_trips_through_save_and_load() {
        let packets = packets_from(WELL_FORMED_USB_JSON);
        let fixture =
            build_usb_fixture_from_packets(&packets, &import_options(false, false)).unwrap();
        fixture.validate().unwrap();
        let path = std::env::temp_dir().join(format!(
            "cr-s88-import-usb-roundtrip-{}.yaml",
            std::process::id()
        ));
        fixture.save(&path).unwrap();
        let loaded = CaptureFixture::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(loaded, fixture);
    }

    #[test]
    fn hardware_capture_defaults_to_false_and_requires_the_flag() {
        let packets = packets_from(WELL_FORMED_USB_JSON);
        let without_flag =
            build_usb_fixture_from_packets(&packets, &import_options(false, false)).unwrap();
        assert!(!without_flag.hardware_capture);
        let with_flag =
            build_usb_fixture_from_packets(&packets, &import_options(false, true)).unwrap();
        assert!(with_flag.hardware_capture);
    }
}
