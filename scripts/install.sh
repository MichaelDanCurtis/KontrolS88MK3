#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
root_prefix=${DESTDIR:-}
binary="$repo_dir/target/aarch64-unknown-linux-gnu/release/controlroom"
start_service=1

while [ "$#" -gt 0 ]; do
  case "$1" in
    --binary) binary=${2:?--binary requires a path}; shift 2 ;;
    --root) root_prefix=${2:?--root requires a path}; shift 2 ;;
    --no-start) start_service=0; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if [ ! -x "$binary" ]; then
  echo "Control Room binary is missing or not executable: $binary" >&2
  exit 1
fi
if [ -z "$root_prefix" ] && [ "$(id -u)" -ne 0 ]; then
  echo "run as root, or pass --root/DESTDIR for a staged install" >&2
  exit 1
fi

install_file() {
  mode=$1
  source=$2
  destination=$3
  install -d -m0755 "$(dirname -- "$destination")"
  install -m"$mode" "$source" "$destination"
}

install_file 0755 "$binary" "$root_prefix/usr/bin/controlroom"
install_file 0755 "$repo_dir/deploy/controlroom-healthcheck" "$root_prefix/usr/lib/controlroom/controlroom-healthcheck"
install_file 0644 "$repo_dir/deploy/controlroom.service" "$root_prefix/usr/lib/systemd/system/controlroom.service"
install_file 0644 "$repo_dir/deploy/controlroom.sysusers" "$root_prefix/usr/lib/sysusers.d/controlroom.conf"
install_file 0644 "$repo_dir/deploy/controlroom.tmpfiles" "$root_prefix/usr/lib/tmpfiles.d/controlroom.conf"
install_file 0644 "$repo_dir/deploy/controlroom.service.avahi" "$root_prefix/etc/avahi/services/controlroom.service"
install_file 0644 "$repo_dir/deploy/controlroom.avahi.conf" "$root_prefix/etc/avahi/avahi-daemon.conf.d/controlroom.conf"
install -d -m0750 "$root_prefix/etc/controlroom" "$root_prefix/var/lib/controlroom" "$root_prefix/var/backups/controlroom"
if [ ! -e "$root_prefix/etc/controlroom/controlroom.yaml" ]; then
  install -m0640 "$repo_dir/deploy/controlroom.yaml" "$root_prefix/etc/controlroom/controlroom.yaml"
fi

if [ -z "$root_prefix" ]; then
  systemd-sysusers /usr/lib/sysusers.d/controlroom.conf
  systemd-tmpfiles --create /usr/lib/tmpfiles.d/controlroom.conf
  chown root:controlroom /etc/controlroom
  chown root:controlroom /etc/controlroom/controlroom.yaml
  chown -R controlroom:controlroom /var/lib/controlroom
  if [ "$start_service" -eq 1 ]; then
    systemctl daemon-reload
    systemctl enable avahi-daemon.service
    systemctl restart avahi-daemon.service
    systemctl enable --now controlroom.service
  fi
fi

echo "Control Room installed; configuration: ${root_prefix}/etc/controlroom/controlroom.yaml"
