use anyhow::{Context, Result, bail};
use midir::{Ignore, MidiInput};
use std::time::Duration;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("list") if args.len() == 1 => list(),
        Some("monitor") => monitor(MonitorOptions::parse(&args[1..])?),
        _ => {
            eprintln!(
                "usage:\n  cr-midi-monitor list\n  cr-midi-monitor monitor --port MATCH [--seconds N]"
            );
            std::process::exit(2);
        }
    }
}

fn input_ports(client: &str) -> Result<(MidiInput, Vec<(midir::MidiInputPort, String)>)> {
    let mut input = MidiInput::new(client)?;
    input.ignore(Ignore::None);
    let ports = input
        .ports()
        .into_iter()
        .map(|port| {
            let name = input.port_name(&port)?;
            Ok((port, name))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((input, ports))
}

fn list() -> Result<()> {
    let (_input, ports) = input_ports("control-room-midi-monitor-list")?;
    if ports.is_empty() {
        eprintln!("event=input_ports status=empty count=0");
    } else {
        for (_, name) in ports {
            println!("{name}");
        }
    }
    Ok(())
}

fn monitor(options: MonitorOptions) -> Result<()> {
    let (input, ports) = input_ports("control-room-midi-monitor")?;
    let needle = options.port.to_lowercase();
    let mut matches: Vec<_> = ports
        .into_iter()
        .filter(|(_, name)| name.to_lowercase().contains(&needle))
        .collect();
    if matches.is_empty() {
        bail!("no MIDI input port matches {:?}", options.port);
    }
    if matches.len() != 1 {
        bail!(
            "MIDI input match {:?} is ambiguous; matches: {:?}",
            options.port,
            matches.iter().map(|(_, name)| name).collect::<Vec<_>>()
        );
    }
    let (port, name) = matches.remove(0);
    println!(
        "event=monitor status=started port={name:?} seconds={}",
        options.seconds
    );
    let connection = input
        .connect(
            &port,
            "control-room-midi-monitor-input",
            move |timestamp, bytes, _| print_event(timestamp, bytes),
            (),
        )
        .with_context(|| format!("opening MIDI input {name:?}"))?;
    std::thread::sleep(Duration::from_secs(options.seconds));
    drop(connection);
    println!("event=monitor status=stopped port={name:?}");
    Ok(())
}

fn print_event(timestamp: u64, bytes: &[u8]) {
    let encoded = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    match bytes {
        [status, cc, value] if status & 0xf0 == 0xb0 && *value <= 127 => println!(
            "event=midi_input at_us={timestamp} type=cc channel={} cc={} value={} bytes={encoded:?}",
            (status & 0x0f) + 1,
            cc,
            value
        ),
        _ => println!(
            "event=midi_input at_us={timestamp} type=raw length={} bytes={encoded:?}",
            bytes.len()
        ),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct MonitorOptions {
    port: String,
    seconds: u64,
}

impl MonitorOptions {
    fn parse(args: &[String]) -> Result<Self> {
        let mut port = None;
        let mut seconds = 30;
        let mut index = 0;
        while index < args.len() {
            let name = &args[index];
            let value = args
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .with_context(|| format!("{name} requires a value"))?;
            match name.as_str() {
                "--port" if port.is_none() => port = Some(value.clone()),
                "--seconds" => seconds = value.parse()?,
                "--port" => bail!("duplicate --port"),
                _ => bail!("unknown monitor option {name:?}"),
            }
            index += 2;
        }
        if seconds == 0 {
            bail!("--seconds must be greater than zero");
        }
        Ok(Self {
            port: port.context("missing required --port")?,
            seconds,
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
    fn monitor_options_are_strict() {
        assert_eq!(
            MonitorOptions::parse(&args(&["--port", "Peak"])).unwrap(),
            MonitorOptions {
                port: "Peak".into(),
                seconds: 30,
            }
        );
        assert!(MonitorOptions::parse(&args(&["--port", "Peak", "--seconds", "0"])).is_err());
        assert!(MonitorOptions::parse(&args(&["--wat", "x"])).is_err());
    }
}
