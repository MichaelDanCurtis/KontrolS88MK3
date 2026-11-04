#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
report=-
software_only=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --report) report=${2:?--report requires a path}; shift 2 ;;
    --software-only) software_only=1; shift ;;
    -h|--help)
      echo "usage: scripts/acceptance-preflight.sh [--software-only] [--report FILE]"
      exit 0
      ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if [ "$report" != - ] && [ -e "$report" ]; then
  echo "refusing to overwrite existing acceptance evidence: $report" >&2
  exit 1
fi

default_controlroom_bin="$repo_dir/target/debug/controlroom"
default_s88_capture_bin="$repo_dir/target/debug/cr-s88-capture"
default_midi_monitor_bin="$repo_dir/target/debug/cr-midi-monitor"
if [ -x "$repo_dir/bin/controlroom" ]; then
  default_controlroom_bin="$repo_dir/bin/controlroom"
fi
if [ -x "$repo_dir/bin/cr-s88-capture" ]; then
  default_s88_capture_bin="$repo_dir/bin/cr-s88-capture"
fi
if [ -x "$repo_dir/bin/cr-midi-monitor" ]; then
  default_midi_monitor_bin="$repo_dir/bin/cr-midi-monitor"
fi
controlroom_bin=${CONTROLROOM_BIN:-"$default_controlroom_bin"}
s88_capture_bin=${S88_CAPTURE_BIN:-"$default_s88_capture_bin"}
midi_monitor_bin=${MIDI_MONITOR_BIN:-"$default_midi_monitor_bin"}
s88_fixture="$repo_dir/crates/cr-s88/fixtures/standard-template-cc.synthetic.yaml"
if [ -f "$repo_dir/fixtures/s88-standard-template-cc.synthetic.yaml" ]; then
  s88_fixture="$repo_dir/fixtures/s88-standard-template-cc.synthetic.yaml"
fi
if [ ! -x "$controlroom_bin" ]; then
  if [ ! -f "$repo_dir/Cargo.toml" ]; then
    echo "Control Room binary is missing: $controlroom_bin" >&2
    exit 1
  fi
  cargo build --manifest-path "$repo_dir/Cargo.toml" --locked -p controlroom --bin controlroom
fi
if [ ! -x "$s88_capture_bin" ]; then
  if [ ! -f "$repo_dir/Cargo.toml" ]; then
    echo "S88 capture utility is missing: $s88_capture_bin" >&2
    exit 1
  fi
  cargo build --manifest-path "$repo_dir/Cargo.toml" --locked -p cr-s88 --bin cr-s88-capture
fi
if [ ! -x "$midi_monitor_bin" ]; then
  if [ ! -f "$repo_dir/Cargo.toml" ]; then
    echo "MIDI monitor utility is missing: $midi_monitor_bin" >&2
    exit 1
  fi
  cargo build --manifest-path "$repo_dir/Cargo.toml" --locked -p cr-midi --bin cr-midi-monitor
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/controlroom-acceptance-preflight.XXXXXX")
cleanup() {
  rm -rf -- "$work_dir"
}
trap cleanup EXIT HUP INT TERM
draft="$work_dir/report.md"
command_log="$work_dir/command.log"
software_failures=0
hardware_blocks=0

append() {
  printf '%s\n' "$*" >>"$draft"
}

run_software_check() {
  title=$1
  shift
  if "$@" >"$command_log" 2>&1; then
    result=PASS
  else
    result=FAIL
    software_failures=$((software_failures + 1))
  fi
  append "### $title — $result"
  append
  append '```text'
  sed -n '1,120p' "$command_log" >>"$draft"
  append '```'
  append
}

run_hardware_preflight() {
  title=$1
  shift
  if "$@" >"$command_log" 2>&1; then
    result=READY
  else
    result=BLOCKED
    hardware_blocks=$((hardware_blocks + 1))
  fi
  append "### $title — $result"
  append
  append '```text'
  sed -n '1,120p' "$command_log" >>"$draft"
  append '```'
  append
}

generated=$(date -u +%Y-%m-%dT%H:%M:%SZ)
revision=$(git -C "$repo_dir" rev-parse --short HEAD 2>/dev/null || echo unknown)
append "# Control Room acceptance preflight"
append
append "- Generated: $generated"
append "- Host: $(uname -srm)"
append "- Revision: $revision"
append "- Mode: $([ "$software_only" -eq 1 ] && echo software-only || echo hardware-readiness)"
append
append "This report is evidence of configuration and connectivity readiness only. It never converts an audible, tactile, reboot, or display observation into a PASS."
append
append "## Software configuration checks"
append

for patch in \
  config/lfo-cutoff.yaml \
  config/dual-peak.yaml \
  config/osc-bridge.yaml \
  config/vcv-bridge.yaml \
  config/s88-takeover.yaml \
  deploy/controlroom.yaml
do
  run_software_check "$patch" "$controlroom_bin" --check "$repo_dir/$patch"
done

run_software_check "S88 synthetic capture fixture" \
  "$s88_capture_bin" validate "$s88_fixture"

if [ "$software_only" -eq 0 ]; then
  append "## Visible MIDI endpoints"
  append
  run_software_check "MIDI output inventory" "$controlroom_bin" --list-ports
  run_software_check "MIDI input inventory" "$s88_capture_bin" list
  run_software_check "General MIDI monitor inventory" "$midi_monitor_bin" list

  append "## Hardware connectivity preflight"
  append
  run_hardware_preflight "Peak flagship patch" \
    "$controlroom_bin" --preflight "$repo_dir/config/lfo-cutoff.yaml"
  run_hardware_preflight "OSC bridge patch" \
    "$controlroom_bin" --preflight "$repo_dir/config/osc-bridge.yaml"
  run_hardware_preflight "S88 takeover patch" \
    "$controlroom_bin" --preflight "$repo_dir/config/s88-takeover.yaml"
  run_hardware_preflight "Appliance patch" \
    "$controlroom_bin" --preflight "$repo_dir/deploy/controlroom.yaml"
fi

append "## Human-observed completion gates"
append
append "| Gate | Result | Required evidence |"
append "|---|---|---|"
append "| Peak cutoff sweep and musical feel | NOT RUN | Firmware/channel, audible 0.3 Hz sweep, range, jitter/stepping notes |"
append "| Native/browser connected-instrument session | NOT RUN | Extended session notes, disconnect/reconnect behavior, screenshot |"
append "| Intended OSC client interoperability | NOT RUN | Client/version, bidirectional addresses and observed values |"
append "| S88 eight-knob template and pickup | NOT RUN | 24 min/mid/max events, binding map, no-jump observations |"
append "| S88 unplug/replug | NOT RUN | Runtime reconnect counters and observation |"
append "| Raspberry Pi unattended boot | NOT RUN | Boot ID, service status, health, mDNS, restart/reconnect results |"
append "| MK3 display/framebuffer research | NOT RUN | Reviewed physical capture and safe rendered test image |"
append
append "## Result"
append
if [ "$software_failures" -gt 0 ]; then
  append "FAIL — $software_failures software prerequisite(s) failed."
elif [ "$software_only" -eq 0 ] && [ "$hardware_blocks" -gt 0 ]; then
  append "BLOCKED — software prerequisites passed; $hardware_blocks hardware preflight(s) are not ready on this host."
else
  append "READY — all checks requested by this run passed. Human-observed gates above remain authoritative."
fi

if [ "$report" = - ]; then
  cat "$draft"
else
  install -d -m0755 "$(dirname -- "$report")"
  install -m0644 "$draft" "$report"
  echo "$report"
fi

if [ "$software_failures" -gt 0 ]; then
  exit 1
fi
if [ "$software_only" -eq 0 ] && [ "$hardware_blocks" -gt 0 ]; then
  exit 2
fi
