#!/bin/sh
set -eu

archive=${1:-}
if [ -z "$archive" ]; then
  echo "usage: scripts/verify-macos-release.sh ARCHIVE.tar.gz" >&2
  exit 2
fi
if [ ! -f "$archive" ]; then
  echo "release archive does not exist: $archive" >&2
  exit 1
fi

archive_dir=$(CDPATH= cd -- "$(dirname -- "$archive")" && pwd)
archive="$archive_dir/$(basename -- "$archive")"
checksum="$archive.sha256"
if [ ! -f "$checksum" ]; then
  echo "release checksum does not exist: $checksum" >&2
  exit 1
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/controlroom-release-verify.XXXXXX")
cleanup() {
  rm -rf -- "$work_dir"
}
trap cleanup EXIT HUP INT TERM

(
  cd "$archive_dir"
  shasum -a 256 -c "$(basename -- "$checksum")"
)

entries="$work_dir/entries.txt"
tar -tzf "$archive" >"$entries"
top_level=$(sed -n '1s,/.*,,p' "$entries")
if [ -z "$top_level" ]; then
  echo "release archive is empty" >&2
  exit 1
fi
if ! awk -v top="$top_level" '
  index($0, "/../") || $0 ~ /^\.\.\// || $0 ~ /^\// { exit 1 }
  $0 != top && index($0, top "/") != 1 { exit 1 }
' "$entries"
then
  echo "release archive contains an unsafe or unexpected path" >&2
  exit 1
fi

tar -xzf "$archive" -C "$work_dir"
release_root="$work_dir/$top_level"
for path in \
  "RELEASE-MANIFEST.txt" \
  "Control Room.app/Contents/MacOS/ControlRoomNative" \
  "bin/controlroom" \
  "bin/cr-midi-monitor" \
  "bin/cr-s88-capture" \
  "config/lfo-cutoff.yaml" \
  "deploy/controlroom.service" \
  "fixtures/s88-standard-template-cc.synthetic.yaml" \
  "integrations/vcv-control-room/plugin.json" \
  "integrations/vcv-control-room/src/ControlRoomBridge.cpp" \
  "integrations/vcv-control-room/tests/bridge-smoke.vcv" \
  "scripts/acceptance-preflight.sh" \
  "scripts/install-vcv-sdk.sh" \
  "scripts/osc-smoke.sh" \
  "scripts/verify-vcv-bridge.sh" \
  "scripts/rack-agent-probe.py"
do
  if [ ! -e "$release_root/$path" ]; then
    echo "release payload is missing: $path" >&2
    exit 1
  fi
done

codesign --verify --deep --strict "$release_root/Control Room.app"
"$release_root/bin/controlroom" --version
clang++ -std=c++17 -pthread -Wall -Wextra -Werror \
  "$release_root/integrations/vcv-control-room/src/OscBridge.cpp" \
  "$release_root/integrations/vcv-control-room/tests/osc_codec_test.cpp" \
  -o "$work_dir/vcv-osc-codec-test"
"$work_dir/vcv-osc-codec-test"
"$release_root/scripts/acceptance-preflight.sh" --software-only >/dev/null
"$release_root/scripts/osc-smoke.sh" >/dev/null
echo "macOS release verification passed: $archive"
