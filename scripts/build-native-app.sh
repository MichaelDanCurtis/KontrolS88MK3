#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
package_dir="$repo_dir/native/ControlRoom"
output_dir=${1:-"$repo_dir/target/Control Room.app"}
contents="$output_dir/Contents"
icon_source="$repo_dir/target/ControlRoomIcon-1024.png"
iconset="$repo_dir/target/ControlRoomIcon.iconset"

swift build --package-path "$package_dir" -c release
install -d -m0755 "$contents/MacOS" "$contents/Resources"
install -m0755 "$package_dir/.build/release/ControlRoomNative" "$contents/MacOS/ControlRoomNative"
install -m0644 "$package_dir/Info.plist" "$contents/Info.plist"
swift "$repo_dir/scripts/render-native-icon.swift" "$icon_source"
install -d -m0755 "$iconset"
sips -z 16 16 "$icon_source" --out "$iconset/icon_16x16.png" >/dev/null
sips -z 32 32 "$icon_source" --out "$iconset/icon_16x16@2x.png" >/dev/null
sips -z 32 32 "$icon_source" --out "$iconset/icon_32x32.png" >/dev/null
sips -z 64 64 "$icon_source" --out "$iconset/icon_32x32@2x.png" >/dev/null
sips -z 128 128 "$icon_source" --out "$iconset/icon_128x128.png" >/dev/null
sips -z 256 256 "$icon_source" --out "$iconset/icon_128x128@2x.png" >/dev/null
sips -z 256 256 "$icon_source" --out "$iconset/icon_256x256.png" >/dev/null
sips -z 512 512 "$icon_source" --out "$iconset/icon_256x256@2x.png" >/dev/null
sips -z 512 512 "$icon_source" --out "$iconset/icon_512x512.png" >/dev/null
install -m0644 "$icon_source" "$iconset/icon_512x512@2x.png"
iconutil -c icns "$iconset" -o "$contents/Resources/ControlRoomIcon.icns"
codesign --force --sign - "$output_dir"

echo "$output_dir"
