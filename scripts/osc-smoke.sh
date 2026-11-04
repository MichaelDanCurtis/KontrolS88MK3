#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
default_controlroom_bin="$repo_dir/target/debug/controlroom"
if [ -x "$repo_dir/bin/controlroom" ]; then
  default_controlroom_bin="$repo_dir/bin/controlroom"
fi
controlroom_bin=${CONTROLROOM_BIN:-"$default_controlroom_bin"}
probe="$script_dir/osc-probe.py"

if [ ! -x "$controlroom_bin" ]; then
  if [ ! -f "$repo_dir/Cargo.toml" ]; then
    echo "Control Room binary is missing: $controlroom_bin" >&2
    exit 1
  fi
  cargo build --manifest-path "$repo_dir/Cargo.toml" --locked -p controlroom --bin controlroom
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/controlroom-osc-smoke.XXXXXX")
runtime_pid=
listener_pid=
cleanup() {
  if [ -n "$runtime_pid" ] && kill -0 "$runtime_pid" 2>/dev/null; then
    kill -INT "$runtime_pid" 2>/dev/null || true
    wait "$runtime_pid" 2>/dev/null || true
  fi
  if [ -n "$listener_pid" ] && kill -0 "$listener_pid" 2>/dev/null; then
    kill "$listener_pid" 2>/dev/null || true
    wait "$listener_pid" 2>/dev/null || true
  fi
  rm -rf -- "$work_dir"
}
trap cleanup EXIT HUP INT TERM

"$probe" self-test
"$probe" listen \
  --bind 127.0.0.1:9001 \
  --address /control/lfo \
  --count 5 \
  --timeout 10 >"$work_dir/listener.log" 2>&1 &
listener_pid=$!

"$controlroom_bin" \
  --dry-run \
  --listen 127.0.0.1:17781 \
  --state-file "$work_dir/active.state.yaml" \
  "$repo_dir/config/osc-bridge.yaml" >"$work_dir/runtime.log" 2>&1 &
runtime_pid=$!

"$repo_dir/deploy/controlroom-healthcheck" http://127.0.0.1:17781/api/v1/health
"$probe" send --target 127.0.0.1:9000 --address /control/cutoff --value 95.25
"$probe" check-state \
  --url http://127.0.0.1:17781/api/v1/state \
  --node remote-cutoff \
  --expected 0.75 \
  --tolerance 0.01 \
  --timeout 5

wait "$listener_pid"
listener_pid=
kill -INT "$runtime_pid"
wait "$runtime_pid"
runtime_pid=

grep -q '"event": "received"' "$work_dir/listener.log"
grep -q 'event=osc status=stopped' "$work_dir/runtime.log"
cat "$work_dir/listener.log"
grep 'event=osc' "$work_dir/runtime.log"
echo "OSC external-process smoke passed"
