#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
output_dir=${1:-"$repo_dir/target/dist"}
version=$(awk '
  /^name = "controlroom"$/ { package = 1; next }
  package && /^version = / { gsub(/[" ]/, "", $3); print $3; exit }
' "$repo_dir/crates/controlroom/Cargo.toml")
architecture=$(uname -m)
bundle_name="controlroom-$version-macos-$architecture"
archive="$output_dir/$bundle_name.tar.gz"
checksum="$archive.sha256"

if [ -z "$version" ]; then
  echo "could not determine controlroom package version" >&2
  exit 1
fi
if [ -e "$archive" ] || [ -e "$checksum" ]; then
  echo "release artifact already exists: $archive" >&2
  exit 1
fi

work_dir=$(mktemp -d "$repo_dir/target/controlroom-package.XXXXXX")
cleanup() {
  rm -rf -- "$work_dir"
}
trap cleanup EXIT HUP INT TERM
stage="$work_dir/$bundle_name"

cargo build --manifest-path "$repo_dir/Cargo.toml" --locked --release \
  -p controlroom --bin controlroom \
  -p cr-midi --bin cr-midi-monitor \
  -p cr-s88 --bin cr-s88-capture
install -d -m0755 \
  "$stage/bin" \
  "$stage/config" \
  "$stage/deploy" \
  "$stage/docs" \
  "$stage/fixtures" \
  "$stage/integrations/vcv-control-room/res" \
  "$stage/integrations/vcv-control-room/src" \
  "$stage/integrations/vcv-control-room/tests" \
  "$stage/scripts" \
  "$output_dir"
install -m0755 "$repo_dir/target/release/controlroom" "$stage/bin/controlroom"
install -m0755 "$repo_dir/target/release/cr-midi-monitor" "$stage/bin/cr-midi-monitor"
install -m0755 "$repo_dir/target/release/cr-s88-capture" "$stage/bin/cr-s88-capture"
"$script_dir/build-native-app.sh" "$stage/Control Room.app" >/dev/null
install -m0644 "$repo_dir/README.md" "$stage/README.md"
install -m0644 "$repo_dir/config/lfo-cutoff.yaml" "$stage/config/lfo-cutoff.yaml"
install -m0644 "$repo_dir/config/dual-peak.yaml" "$stage/config/dual-peak.yaml"
install -m0644 "$repo_dir/config/osc-bridge.yaml" "$stage/config/osc-bridge.yaml"
install -m0644 "$repo_dir/config/vcv-bridge.yaml" "$stage/config/vcv-bridge.yaml"
install -m0644 "$repo_dir/config/s88-takeover.yaml" "$stage/config/s88-takeover.yaml"
install -m0644 "$repo_dir/docs/ROADMAP.md" "$stage/docs/ROADMAP.md"
install -m0644 "$repo_dir/docs/s88-protocol-notebook.md" "$stage/docs/s88-protocol-notebook.md"
install -m0644 \
  "$repo_dir/crates/cr-s88/fixtures/standard-template-cc.synthetic.yaml" \
  "$stage/fixtures/s88-standard-template-cc.synthetic.yaml"
for asset in Makefile README.md plugin.json; do
  install -m0644 \
    "$repo_dir/integrations/vcv-control-room/$asset" \
    "$stage/integrations/vcv-control-room/$asset"
done
for source in "$repo_dir"/integrations/vcv-control-room/src/*; do
  install -m0644 "$source" "$stage/integrations/vcv-control-room/src/$(basename "$source")"
done
install -m0644 \
  "$repo_dir/integrations/vcv-control-room/res/Bridge8.svg" \
  "$stage/integrations/vcv-control-room/res/Bridge8.svg"
install -m0644 \
  "$repo_dir/integrations/vcv-control-room/tests/osc_codec_test.cpp" \
  "$stage/integrations/vcv-control-room/tests/osc_codec_test.cpp"
install -m0644 \
  "$repo_dir/integrations/vcv-control-room/tests/bridge-smoke.vcv" \
  "$stage/integrations/vcv-control-room/tests/bridge-smoke.vcv"
for asset in "$repo_dir"/deploy/*; do
  install -m0644 "$asset" "$stage/deploy/$(basename "$asset")"
done
chmod 0755 "$stage/deploy/controlroom-healthcheck"
for helper in \
  acceptance-preflight.sh \
  appliance-acceptance.sh \
  install-vcv-sdk.sh \
  install.sh \
  osc-probe.py \
  osc-smoke.sh \
  peak-acceptance.sh \
  rack-agent-probe.py \
  s88-acceptance.sh \
  studio-acceptance.sh \
  uninstall.sh \
  upgrade.sh \
  verify-vcv-bridge.sh
do
  install -m0755 "$repo_dir/scripts/$helper" "$stage/scripts/$helper"
done

revision=$(git -C "$repo_dir" rev-parse HEAD 2>/dev/null || echo unknown)
if [ -z "$(git -C "$repo_dir" status --porcelain 2>/dev/null)" ]; then
  worktree=clean
else
  worktree=dirty
fi
{
  echo "Control Room $version"
  echo "Revision: $revision"
  echo "Worktree: $worktree"
  echo "Platform: macOS $architecture"
  echo "Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$stage/RELEASE-MANIFEST.txt"

codesign --verify --deep --strict "$stage/Control Room.app"
"$stage/bin/controlroom" --check "$stage/config/lfo-cutoff.yaml" >/dev/null
"$stage/bin/cr-s88-capture" validate \
  "$stage/fixtures/s88-standard-template-cc.synthetic.yaml" >/dev/null
PYTHONDONTWRITEBYTECODE=1 "$stage/scripts/osc-probe.py" self-test >/dev/null
"$stage/scripts/acceptance-preflight.sh" --software-only >/dev/null
tar -C "$work_dir" -czf "$archive" "$bundle_name"
(
  cd "$output_dir"
  shasum -a 256 "$(basename "$archive")" >"$(basename "$checksum")"
)

echo "$archive"
echo "$checksum"
