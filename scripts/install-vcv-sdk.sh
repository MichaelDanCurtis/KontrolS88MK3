#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
rack_app=${VCV_RACK_APP:-/Applications/VCV Rack 2 Pro.app}
sdk_root=${VCV_SDK_ROOT:-"$repo_dir/target/vcv-sdk"}
sdk_dir="$sdk_root/Rack-SDK"

if [ ! -f "$rack_app/Contents/Info.plist" ]; then
  echo "VCV Rack app not found: $rack_app" >&2
  exit 1
fi

version=$(plutil -extract CFBundleShortVersionString raw "$rack_app/Contents/Info.plist")
case $(uname -m) in
  arm64) sdk_arch=arm64 ;;
  x86_64) sdk_arch=x64 ;;
  *) echo "unsupported macOS architecture: $(uname -m)" >&2; exit 1 ;;
esac

if [ -f "$sdk_dir/plugin.mk" ]; then
  echo "Rack SDK already installed: $sdk_dir"
  exit 0
fi
if [ -e "$sdk_root" ]; then
  echo "SDK destination exists but is incomplete: $sdk_root" >&2
  exit 1
fi

archive="Rack-SDK-$version-mac-$sdk_arch.zip"
download_dir="$repo_dir/target/vcv-sdk-download"
install_dir=$(mktemp -d "$repo_dir/target/vcv-sdk-install.XXXXXX")
cleanup() {
  rm -rf -- "$install_dir"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$download_dir"
curl -fL --retry 3 --output "$download_dir/$archive" "https://vcvrack.com/downloads/$archive"
unzip -q "$download_dir/$archive" -d "$install_dir"
if [ ! -f "$install_dir/Rack-SDK/plugin.mk" ]; then
  echo "downloaded archive does not contain Rack-SDK/plugin.mk" >&2
  exit 1
fi
mv "$install_dir" "$sdk_root"
trap - EXIT HUP INT TERM
echo "Installed Rack SDK $version for macOS $sdk_arch: $sdk_dir"
