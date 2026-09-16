#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
backup_dir=/var/backups/controlroom/$timestamp

if [ "$(id -u)" -ne 0 ]; then
  echo "upgrade must run as root" >&2
  exit 1
fi

install -d -m0750 "$backup_dir"
for path in /usr/bin/controlroom /etc/controlroom/controlroom.yaml /var/lib/controlroom/active.state.yaml; do
  if [ -f "$path" ]; then
    install -m0600 "$path" "$backup_dir/$(basename "$path")"
  fi
done

"$script_dir/install.sh" "$@"
echo "Pre-upgrade files retained in $backup_dir"
