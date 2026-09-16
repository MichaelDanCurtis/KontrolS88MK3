#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
  echo "uninstall must run as root" >&2
  exit 1
fi

timestamp=$(date -u +%Y%m%dT%H%M%SZ)
backup_dir=/var/backups/controlroom/uninstall-$timestamp
install -d -m0750 "$backup_dir"

systemctl disable --now controlroom.service 2>/dev/null || true
if [ -d /etc/controlroom ]; then
  mv /etc/controlroom "$backup_dir/config"
fi
if [ -d /var/lib/controlroom ]; then
  mv /var/lib/controlroom "$backup_dir/state"
fi

rm -f /usr/bin/controlroom
rm -f /usr/lib/controlroom/controlroom-healthcheck
rm -f /usr/lib/systemd/system/controlroom.service
rm -f /usr/lib/sysusers.d/controlroom.conf
rm -f /usr/lib/tmpfiles.d/controlroom.conf
rm -f /etc/avahi/services/controlroom.service
rm -f /etc/avahi/avahi-daemon.conf.d/controlroom.conf
systemctl daemon-reload
systemctl restart avahi-daemon.service 2>/dev/null || true

echo "Control Room removed; configuration and state retained in $backup_dir"
