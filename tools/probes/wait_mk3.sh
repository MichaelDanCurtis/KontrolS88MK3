#!/bin/zsh
# Wait (bounded) for a Kontrol MK3 to enumerate, then probe it read-only.
deadline=$(( $(date +%s) + 1200 ))   # 20 minutes
found=0
while [ $(date +%s) -lt $deadline ]; do
  if ioreg -p IOUSB -w 0 2>/dev/null | grep -qi "kontrol"; then found=1; break; fi
  if ioreg -c IOHIDDevice -w 0 2>/dev/null | grep -qi "kontrol"; then found=1; break; fi
  sleep 3
done

if [ $found -eq 0 ]; then
  echo "TIMEOUT: no Kontrol device enumerated within 20 minutes."
  exit 1
fi

echo "=== KONTROL DEVICE DETECTED ==="
sleep 3   # let macOS finish enumerating all interfaces

echo
echo "--- USB identity ---"
ioreg -p IOUSB -l -w 0 2>/dev/null | grep -iA6 '"kontrol' | grep -iE '"USB Product Name"|"idVendor"|"idProduct"' | head -12

echo
echo "--- MIDI ports (CoreMIDI) ---"
system_profiler SPUSBDataType 2>/dev/null | grep -iA4 "kontrol" | head -20

echo
echo "--- READ-ONLY HID feature report 0xF8 probe ---"
./mk3probe 2>&1

echo
echo "=== PROBE COMPLETE (reads only; nothing written to the device) ==="
