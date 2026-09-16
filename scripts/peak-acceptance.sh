#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
default_controlroom_bin="$repo_dir/target/debug/controlroom"
default_monitor_bin="$repo_dir/target/debug/cr-midi-monitor"
if [ -x "$repo_dir/bin/controlroom" ]; then
  default_controlroom_bin="$repo_dir/bin/controlroom"
fi
if [ -x "$repo_dir/bin/cr-midi-monitor" ]; then
  default_monitor_bin="$repo_dir/bin/cr-midi-monitor"
fi
controlroom_bin=${CONTROLROOM_BIN:-"$default_controlroom_bin"}
monitor_bin=${MIDI_MONITOR_BIN:-"$default_monitor_bin"}
port=Peak
firmware=
channel=1
report=
monitor_seconds=45
live_seconds=20
run_live=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --port) port=${2:?--port requires a value}; shift 2 ;;
    --firmware) firmware=${2:?--firmware requires a value}; shift 2 ;;
    --channel) channel=${2:?--channel requires a value}; shift 2 ;;
    --report) report=${2:?--report requires a path}; shift 2 ;;
    --monitor-seconds) monitor_seconds=${2:?--monitor-seconds requires a value}; shift 2 ;;
    --live-seconds) live_seconds=${2:?--live-seconds requires a value}; shift 2 ;;
    --run-live) run_live=1; shift ;;
    -h|--help)
      echo "usage: scripts/peak-acceptance.sh --firmware VERSION --report FILE [--port Peak] [--channel 1] [--monitor-seconds 45] [--run-live] [--live-seconds 20]"
      exit 0
      ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$firmware" ] || [ -z "$report" ]; then
  echo "--firmware and --report are required" >&2
  exit 2
fi
if [ -e "$report" ]; then
  echo "refusing to overwrite existing Peak evidence: $report" >&2
  exit 1
fi
if [ ! -x "$controlroom_bin" ]; then
  if [ ! -f "$repo_dir/Cargo.toml" ]; then
    echo "Control Room binary is missing: $controlroom_bin" >&2
    exit 1
  fi
  cargo build --manifest-path "$repo_dir/Cargo.toml" --locked -p controlroom --bin controlroom
fi
if [ ! -x "$monitor_bin" ]; then
  if [ ! -f "$repo_dir/Cargo.toml" ]; then
    echo "MIDI monitor utility is missing: $monitor_bin" >&2
    exit 1
  fi
  cargo build --manifest-path "$repo_dir/Cargo.toml" --locked -p cr-midi --bin cr-midi-monitor
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/controlroom-peak-acceptance.XXXXXX")
runtime_pid=
cleanup() {
  if [ -n "$runtime_pid" ] && kill -0 "$runtime_pid" 2>/dev/null; then
    kill -INT "$runtime_pid" 2>/dev/null || true
    wait "$runtime_pid" 2>/dev/null || true
  fi
  rm -rf -- "$work_dir"
}
trap cleanup EXIT HUP INT TERM

preflight_status=PASS
if ! "$controlroom_bin" --preflight "$repo_dir/config/lfo-cutoff.yaml" \
  >"$work_dir/preflight.log" 2>&1
then
  preflight_status=FAIL
fi

monitor_status=NOT_RUN
if [ "$preflight_status" = PASS ]; then
  echo "For the next ${monitor_seconds}s, move these Peak controls one at a time through their range:"
  echo "cutoff, resonance, amp envelope attack/decay/sustain/release, and LFO 1 rate."
  echo "Monitoring starts in 3 seconds; no MIDI bytes will be transmitted."
  sleep 3
  if "$monitor_bin" monitor --port "$port" --seconds "$monitor_seconds" \
    >"$work_dir/monitor.log" 2>&1
  then
    monitor_status=CAPTURED
  else
    monitor_status=FAIL
  fi
else
  printf '%s\n' "monitor skipped because preflight failed" >"$work_dir/monitor.log"
fi

live_status=NOT_RUN
if [ "$run_live" -eq 1 ] && [ "$preflight_status" = PASS ]; then
  echo "Starting the live 0.3 Hz cutoff sweep for ${live_seconds}s. Listen for stepping or jitter."
  "$controlroom_bin" --no-server "$repo_dir/config/lfo-cutoff.yaml" \
    >"$work_dir/live.log" 2>&1 &
  runtime_pid=$!
  sleep "$live_seconds"
  kill -INT "$runtime_pid"
  if wait "$runtime_pid"; then
    live_status=COMPLETED
  else
    live_status=FAIL
  fi
  runtime_pid=
else
  printf '%s\n' "live sweep not requested" >"$work_dir/live.log"
fi

install -d -m0755 "$(dirname -- "$report")"
{
  echo "# Peak hardware acceptance"
  echo
  echo "- Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- Firmware: $firmware"
  echo "- Port match: $port"
  echo "- MIDI channel: $channel"
  echo "- Connectivity preflight: $preflight_status"
  echo "- Input monitor: $monitor_status"
  echo "- Live sweep execution: $live_status"
  echo
  echo "## Connectivity preflight"
  echo
  echo '```text'
  sed -n '1,160p' "$work_dir/preflight.log"
  echo '```'
  echo
  echo "## Peak control monitor"
  echo
  echo '```text'
  sed -n '1,500p' "$work_dir/monitor.log"
  echo '```'
  echo
  echo "## Live runtime log"
  echo
  echo '```text'
  sed -n '1,240p' "$work_dir/live.log"
  echo '```'
  echo
  echo "## Human observations"
  echo
  echo "| Check | Result | Observed |"
  echo "|---|---|---|"
  echo "| Port and channel correct | NOT RECORDED | |"
  echo "| CC 29 moves filter cutoff | NOT RECORDED | |"
  echo "| 0.3 Hz sweep period is approximately 3.3 seconds | NOT RECORDED | |"
  echo "| Configured 20–90 range is audible | NOT RECORDED | |"
  echo "| No stuck or jittery values | NOT RECORDED | |"
  echo "| No audible stepping from rate limiting | NOT RECORDED | |"
  echo "| CC 79 resonance confirmed | NOT RECORDED | |"
  echo "| CC 86–89 amp envelope confirmed | NOT RECORDED | |"
  echo "| CC 30 LFO 1 rate confirmed | NOT RECORDED | |"
  echo
  echo "Do not change the Peak pack's verification status until each relevant row is filled from direct observation."
} >"$work_dir/report.md"
install -m0644 "$work_dir/report.md" "$report"
echo "$report"

if [ "$preflight_status" = FAIL ] || [ "$monitor_status" = FAIL ] || [ "$live_status" = FAIL ]; then
  exit 1
fi
