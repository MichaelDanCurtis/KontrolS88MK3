#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
output_dir=
peak_firmware=
s88_firmware=
peak_port=Peak
s88_port="KONTROL S88 MK3"
channel=1
cc_start=20
run_live=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --output-dir) output_dir=${2:?--output-dir requires a path}; shift 2 ;;
    --peak-firmware) peak_firmware=${2:?--peak-firmware requires a value}; shift 2 ;;
    --s88-firmware) s88_firmware=${2:?--s88-firmware requires a value}; shift 2 ;;
    --peak-port) peak_port=${2:?--peak-port requires a value}; shift 2 ;;
    --s88-port) s88_port=${2:?--s88-port requires a value}; shift 2 ;;
    --channel) channel=${2:?--channel requires a value}; shift 2 ;;
    --cc-start) cc_start=${2:?--cc-start requires a value}; shift 2 ;;
    --run-live) run_live=1; shift ;;
    -h|--help)
      echo "usage: scripts/studio-acceptance.sh --output-dir DIR --peak-firmware VERSION --s88-firmware VERSION [--peak-port Peak] [--s88-port 'KONTROL S88 MK3'] [--channel 1] [--cc-start 20] [--run-live]"
      echo "--run-live explicitly authorizes the Peak MIDI sweep; without it, only input monitoring is performed."
      exit 0
      ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$output_dir" ] || [ -z "$peak_firmware" ] || [ -z "$s88_firmware" ]; then
  echo "--output-dir, --peak-firmware, and --s88-firmware are required" >&2
  exit 2
fi
if [ -e "$output_dir" ]; then
  echo "refusing to overwrite an existing studio evidence directory: $output_dir" >&2
  exit 1
fi

install -d -m0755 "$output_dir"
started=$(date -u +%Y-%m-%dT%H:%M:%SZ)

echo "Stage 1/4: non-transmitting configuration and hardware preflight"
"$script_dir/acceptance-preflight.sh" --report "$output_dir/preflight.md"

echo "Stage 2/4: independent-process OSC boundary smoke"
"$script_dir/osc-smoke.sh" >"$output_dir/osc-smoke.log" 2>&1

echo "Stage 3/4: Peak input evidence and optional live output sweep"
set -- \
  --firmware "$peak_firmware" \
  --port "$peak_port" \
  --channel "$channel" \
  --report "$output_dir/peak.md"
if [ "$run_live" -eq 1 ]; then
  set -- "$@" --run-live
fi
"$script_dir/peak-acceptance.sh" "$@"

echo "Stage 4/4: S88 eight-knob min/mid/max capture"
"$script_dir/s88-acceptance.sh" \
  --port "$s88_port" \
  --firmware "$s88_firmware" \
  --output "$output_dir/s88-standard-template.yaml" \
  --channel "$channel" \
  --cc-start "$cc_start"

cat >"$output_dir/SESSION.md" <<EOF
# Control Room connected-studio acceptance

- Started: $started
- Completed: $(date -u +%Y-%m-%dT%H:%M:%SZ)
- Peak firmware: $peak_firmware
- Peak port match: $peak_port
- S88 firmware: $s88_firmware
- S88 port match: $s88_port
- MIDI channel: $channel
- Peak live sweep requested: $run_live

## Machine evidence

| Artifact | Result |
|---|---|
| Unified preflight (preflight.md) | PASS |
| Independent OSC boundary (osc-smoke.log) | PASS |
| Peak capture/runtime report (peak.md) | RECORDED |
| S88 min/mid/max fixture (s88-standard-template.yaml) | VERIFIED |

## Direct observations

Fill these rows during the session. Machine evidence does not close them.

| Gate | Result | Notes |
|---|---|---|
| Peak cutoff, resonance, envelope, and LFO mappings are correct | NOT RECORDED | |
| Peak modulation has no audible stepping, jitter, or stuck value | NOT RECORDED | |
| Native editor remains usable through an extended patching session | NOT RECORDED | |
| S88 knob order and banking are correct | NOT RECORDED | |
| S88 pickup crosses without an output jump | NOT RECORDED | |
| Peak and S88 recover after unplug/replug | NOT RECORDED | |
| Intended external OSC client works bidirectionally | NOT RECORDED | |

The M1, M2/M2N/M2S/M6, M3, and M4 hardware gates close only after the relevant rows are completed from direct observation.
EOF

echo "Connected-studio machine evidence captured in: $output_dir"
echo "Complete the direct-observation rows in: $output_dir/SESSION.md"
