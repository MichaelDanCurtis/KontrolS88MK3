#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
default_capture_bin="$repo_dir/target/debug/cr-s88-capture"
if [ -x "$repo_dir/bin/cr-s88-capture" ]; then
  default_capture_bin="$repo_dir/bin/cr-s88-capture"
fi
capture_bin=${S88_CAPTURE_BIN:-"$default_capture_bin"}
port=
firmware=
output=
seconds=60
channel=1
cc_start=20

while [ "$#" -gt 0 ]; do
  case "$1" in
    --port) port=${2:?--port requires a value}; shift 2 ;;
    --firmware) firmware=${2:?--firmware requires a value}; shift 2 ;;
    --output) output=${2:?--output requires a path}; shift 2 ;;
    --seconds) seconds=${2:?--seconds requires a value}; shift 2 ;;
    --channel) channel=${2:?--channel requires a value}; shift 2 ;;
    --cc-start) cc_start=${2:?--cc-start requires a value}; shift 2 ;;
    -h|--help)
      echo "usage: scripts/s88-acceptance.sh --port MATCH --firmware VERSION --output FILE [--seconds 60] [--channel 1] [--cc-start 20]"
      exit 0
      ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$port" ] || [ -z "$firmware" ] || [ -z "$output" ]; then
  echo "--port, --firmware, and --output are required" >&2
  exit 2
fi
if [ -e "$output" ]; then
  echo "refusing to overwrite existing hardware capture: $output" >&2
  exit 1
fi
if [ ! -x "$capture_bin" ]; then
  if [ ! -f "$repo_dir/Cargo.toml" ]; then
    echo "S88 capture utility is missing: $capture_bin" >&2
    exit 1
  fi
  cargo build --manifest-path "$repo_dir/Cargo.toml" --locked -p cr-s88 --bin cr-s88-capture
fi

echo "Visible MIDI inputs:"
"$capture_bin" list
echo
echo "During the ${seconds}s capture, move knobs 1 through 8 to minimum, midpoint, and maximum."
echo "Use standard-template channel $channel and consecutive CCs $cc_start-$((cc_start + 7))."
echo "Capture starts in 3 seconds; no MIDI bytes will be transmitted."
sleep 3

"$capture_bin" record \
  --port "$port" \
  --output "$output" \
  --seconds "$seconds" \
  --firmware "$firmware"
"$capture_bin" summarize "$output"
"$capture_bin" verify-template "$output" --channel "$channel" --cc-start "$cc_start"

echo
echo "Capture evidence passed. The tactile gate still requires:"
echo "  1. Map all eight captured bindings in Control Room."
echo "  2. Confirm expected parameters move and bank ordering is correct."
echo "  3. Confirm pickup causes no output jump."
echo "  4. Unplug/replug the S88 and save reconnect counters."
