#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
rack_app=${VCV_RACK_APP:-/Applications/VCV Rack 2 Pro.app}
rack_binary="$rack_app/Contents/MacOS/Rack"
rack_sdk=${RACK_DIR:-"$repo_dir/target/vcv-sdk/Rack-SDK"}
plugin_dir="$repo_dir/integrations/vcv-control-room"
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/controlroom-vcv-verify.XXXXXX")
controlroom_pid=
rack_pid=

cleanup() {
  if [ -n "$rack_pid" ]; then
    kill -TERM "$rack_pid" 2>/dev/null || true
    wait "$rack_pid" 2>/dev/null || true
  fi
  if [ -n "$controlroom_pid" ]; then
    kill -INT "$controlroom_pid" 2>/dev/null || true
    wait "$controlroom_pid" 2>/dev/null || true
  fi
  rm -rf -- "$work_dir"
}
trap cleanup EXIT HUP INT TERM

if [ ! -x "$rack_binary" ]; then
  echo "VCV Rack executable not found: $rack_binary" >&2
  exit 1
fi
if [ ! -f "$rack_sdk/plugin.mk" ]; then
  echo "Rack SDK not found: $rack_sdk" >&2
  echo "Run scripts/install-vcv-sdk.sh first." >&2
  exit 1
fi
if command -v lsof >/dev/null 2>&1 && lsof -nP -iUDP:9100 -iUDP:9101 2>/dev/null | grep -q 'UDP'; then
  echo "VCV bridge verification needs exclusive loopback UDP ports 9100 and 9101." >&2
  echo "Close any running Bridge 8 session, then run this verifier again." >&2
  exit 2
fi

cargo build --manifest-path "$repo_dir/Cargo.toml" --locked -p controlroom --bin controlroom >/dev/null
(
  cd "$plugin_dir"
  if [ "$rack_sdk" = "$repo_dir/target/vcv-sdk/Rack-SDK" ]; then
    make -j4 RACK_DIR=../../target/vcv-sdk/Rack-SDK dist >/dev/null
  else
    make -j4 RACK_DIR="$rack_sdk" dist >/dev/null
  fi
)

rack_user="$work_dir/rack-user"
mkdir -p "$rack_user/plugins-mac-arm64"
  install -m0644 "$plugin_dir/dist/ControlRoomBridge-2.1.0-mac-arm64.vcvplugin" \
  "$rack_user/plugins-mac-arm64/"

"$repo_dir/target/debug/controlroom" --dry-run \
  --listen 127.0.0.1:17784 \
  --state-file "$work_dir/controlroom.state.yaml" \
  "$repo_dir/config/vcv-bridge.yaml" >"$work_dir/controlroom.log" 2>&1 &
controlroom_pid=$!

attempt=0
until curl -fsS http://127.0.0.1:17784/api/v1/state >/dev/null 2>&1; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 100 ] || ! kill -0 "$controlroom_pid" 2>/dev/null; then
    cat "$work_dir/controlroom.log" >&2
    echo "Control Room did not become ready" >&2
    exit 1
  fi
  sleep 0.05
done

fifo="$work_dir/rack.stdin"
mkfifo "$fifo"
"$rack_binary" -u "$rack_user" -h "$plugin_dir/tests/bridge-smoke.vcv" \
  <"$fifo" >"$work_dir/rack.stdout" 2>&1 &
rack_pid=$!
exec 9>"$fifo"

attempt=0
until grep -q "ControlRoomBridge Bridge8 started" "$rack_user/log.txt" 2>/dev/null; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 100 ] || ! kill -0 "$rack_pid" 2>/dev/null; then
    cat "$rack_user/log.txt" >&2 || true
    echo "Rack did not instantiate Bridge8" >&2
    exit 1
  fi
  sleep 0.05
done

python3 "$repo_dir/scripts/osc-probe.py" send \
  --target 127.0.0.1:9100 --address /controlroom/in/1 --value 0.75 >/dev/null
python3 "$repo_dir/scripts/osc-probe.py" check-state \
  --url http://127.0.0.1:17784/api/v1/state \
  --node rack-return-1 --expected 0.75 --tolerance 0.02 --timeout 5 >/dev/null

sleep 0.2
printf '\n' >&9
exec 9>&-
wait "$rack_pid"
rack_pid=

stop_line=$(grep "ControlRoomBridge Bridge8 stopped:" "$rack_user/log.txt" | tail -n 1)
case "$stop_line" in
  *"received=0"*|*"errors=0"*)
    if printf '%s\n' "$stop_line" | grep -q "received=0"; then
      echo "Rack bridge did not receive Control Room output: $stop_line" >&2
      exit 1
    fi
    ;;
  *)
    echo "Rack bridge reported errors or no shutdown counters: $stop_line" >&2
    exit 1
    ;;
esac

kill -INT "$controlroom_pid"
wait "$controlroom_pid"
controlroom_pid=
grep -q "event=osc status=stopped.*received=[1-9].*send_errors=0" "$work_dir/controlroom.log"

echo "VCV Bridge 8 verification passed"
echo "$stop_line"
