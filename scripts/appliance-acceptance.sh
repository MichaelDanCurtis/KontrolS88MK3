#!/bin/sh
set -eu

report=-
restart_test=0
previous_boot_id=

while [ "$#" -gt 0 ]; do
  case "$1" in
    --report) report=${2:?--report requires a path}; shift 2 ;;
    --restart-test) restart_test=1; shift ;;
    --previous-boot-id) previous_boot_id=${2:?--previous-boot-id requires a value}; shift 2 ;;
    -h|--help)
      echo "usage: scripts/appliance-acceptance.sh [--restart-test] [--previous-boot-id ID] [--report FILE]"
      exit 0
      ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if [ "$(uname -s)" != Linux ]; then
  echo "appliance acceptance must run on the Linux target" >&2
  exit 1
fi
if ! command -v systemctl >/dev/null 2>&1; then
  echo "systemctl is required on the appliance target" >&2
  exit 1
fi
if [ "$restart_test" -eq 1 ] && [ "$(id -u)" -ne 0 ]; then
  echo "--restart-test must run as root" >&2
  exit 1
fi
if [ "$report" != - ] && [ -e "$report" ]; then
  echo "refusing to overwrite existing appliance evidence: $report" >&2
  exit 1
fi

work_dir=$(mktemp -d /tmp/controlroom-appliance-acceptance.XXXXXX)
cleanup() {
  rm -rf -- "$work_dir"
}
trap cleanup EXIT HUP INT TERM
draft="$work_dir/report.md"
command_log="$work_dir/command.log"
failures=0

append() {
  printf '%s\n' "$*" >>"$draft"
}

check() {
  title=$1
  shift
  if "$@" >"$command_log" 2>&1; then
    result=PASS
  else
    result=FAIL
    failures=$((failures + 1))
  fi
  append "### $title — $result"
  append
  append '```text'
  sed -n '1,160p' "$command_log" >>"$draft"
  append '```'
  append
}

boot_id=$(cat /proc/sys/kernel/random/boot_id)
append "# Control Room appliance acceptance"
append
append "- Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
append "- Host: $(uname -n)"
append "- Platform: $(uname -srm)"
append "- Boot ID: $boot_id"
append "- Restart test requested: $restart_test"
append

check "aarch64 target" sh -c 'case "$(uname -m)" in aarch64|arm64) uname -m ;; *) uname -m; exit 1 ;; esac'
check "installed patch validates" /usr/bin/controlroom --check /etc/controlroom/controlroom.yaml
check "Control Room enabled" systemctl is-enabled controlroom.service
check "Control Room active" systemctl is-active controlroom.service
check "Avahi enabled" systemctl is-enabled avahi-daemon.service
check "Avahi active" systemctl is-active avahi-daemon.service
check "localhost health" /usr/lib/controlroom/controlroom-healthcheck http://127.0.0.1:7777/api/v1/health
check "mDNS name resolution" getent hosts controlroom.local
check "controlroom.local health" /usr/lib/controlroom/controlroom-healthcheck http://controlroom.local:7777/api/v1/health
check "runtime state endpoint" curl --fail --silent --show-error http://127.0.0.1:7777/api/v1/state
check "service status" systemctl status --no-pager controlroom.service

if [ -n "$previous_boot_id" ]; then
  if [ "$previous_boot_id" = "$boot_id" ]; then
    append "### Reboot changed boot ID — FAIL"
    append
    append "Previous and current boot IDs are both \`$boot_id\`."
    append
    failures=$((failures + 1))
  else
    append "### Reboot changed boot ID — PASS"
    append
    append "Previous: \`$previous_boot_id\`; current: \`$boot_id\`."
    append
  fi
fi

if [ "$restart_test" -eq 1 ]; then
  state=/var/lib/controlroom/active.state.yaml
  if [ -f "$state" ]; then
    before_checksum=$(sha256sum "$state" | awk '{print $1}')
  else
    before_checksum=missing
  fi
  check "service restart" systemctl restart controlroom.service
  check "health after service restart" /usr/lib/controlroom/controlroom-healthcheck http://127.0.0.1:7777/api/v1/health
  if [ -f "$state" ]; then
    after_checksum=$(sha256sum "$state" | awk '{print $1}')
  else
    after_checksum=missing
  fi
  if [ "$before_checksum" = "$after_checksum" ]; then
    append "### Last-known-good state survived restart — PASS"
  else
    append "### Last-known-good state survived restart — FAIL"
    failures=$((failures + 1))
  fi
  append
  append "Before: \`$before_checksum\`; after: \`$after_checksum\`."
  append
fi

append "## Manual observations still required"
append
append "| Observation | Result |"
append "|---|---|"
append "| Power was physically removed and restored without intervention | NOT RECORDED |"
append "| Peak reconnected and resumed modulation after boot | NOT RECORDED |"
append "| S88 reconnected after device unplug/replug | NOT RECORDED |"
append "| Network disconnect/reconnect preserved UI reachability | NOT RECORDED |"
append "| Last valid patch remained musically correct | NOT RECORDED |"
append
append "## Result"
append
if [ "$failures" -eq 0 ]; then
  append "AUTOMATED CHECKS PASS — record the manual observations above before closing the M5 hardware gate."
else
  append "FAIL — $failures appliance acceptance check(s) failed."
fi

if [ "$report" = - ]; then
  cat "$draft"
else
  install -d -m0755 "$(dirname -- "$report")"
  install -m0644 "$draft" "$report"
  echo "$report"
fi

[ "$failures" -eq 0 ]
