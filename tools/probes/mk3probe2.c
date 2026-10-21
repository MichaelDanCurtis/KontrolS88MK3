// mk3probe2 — READ-ONLY diagnostic for the Kontrol S88 MK3.
//
// Purpose: distinguish three explanations for the failed feature-report read:
//   (a) feature report 0xF8 is write-only (SET_REPORT only)  -> other IDs succeed
//   (b) GetReport is blocked for this process entirely        -> all IDs fail alike
//   (c) buffer length was wrong                               -> some size succeeds
//
// Also lists CoreMIDI endpoints, to test the prediction from NI's own Cubase
// MIDI Remote script that a port named "KONTROL S88 MK3 DAW" exists.
//
// READS ONLY. No SetReport. Never touches the bulk display pipe.
//
// Build: cc -o mk3probe2 mk3probe2.c -framework IOKit -framework CoreFoundation \
//            -framework CoreMIDI

#include <stdio.h>
#include <string.h>
#include <IOKit/hid/IOHIDManager.h>
#include <CoreMIDI/CoreMIDI.h>

#define NI_VID 0x17CC

static void list_midi(void) {
    printf("=== CoreMIDI endpoints ===\n");
    ItemCount ns = MIDIGetNumberOfSources();
    ItemCount nd = MIDIGetNumberOfDestinations();
    printf("sources: %lu   destinations: %lu\n",
           (unsigned long)ns, (unsigned long)nd);

    for (ItemCount i = 0; i < ns; i++) {
        MIDIEndpointRef e = MIDIGetSource(i);
        CFStringRef nm = NULL;
        MIDIObjectGetStringProperty(e, kMIDIPropertyName, &nm);
        char buf[256] = {0};
        if (nm) { CFStringGetCString(nm, buf, sizeof buf, kCFStringEncodingUTF8);
                  CFRelease(nm); }
        printf("  IN  [%lu] \"%s\"\n", (unsigned long)i, buf);
    }
    for (ItemCount i = 0; i < nd; i++) {
        MIDIEndpointRef e = MIDIGetDestination(i);
        CFStringRef nm = NULL;
        MIDIObjectGetStringProperty(e, kMIDIPropertyName, &nm);
        char buf[256] = {0};
        if (nm) { CFStringGetCString(nm, buf, sizeof buf, kCFStringEncodingUTF8);
                  CFRelease(nm); }
        printf("  OUT [%lu] \"%s\"\n", (unsigned long)i, buf);
    }
    printf("\n");
}

static const char *fmt_name(unsigned f) {
    switch (f & ~0x40u) {
        case 0x10: return "XRGB8888";  case 0x11: return "RGBX8888";
        case 0x12: return "ARGB8888";  case 0x13: return "RGBA8888";
        case 0x14: return "XBGR8888";  case 0x15: return "BGRX8888";
        case 0x16: return "ABGR8888";  case 0x17: return "BGRA8888";
        case 0x18: return "XRGB1555";  case 0x19: return "R5G5B6";
        case 0x1A: return "ARGB1555";
        case 0x20: return "RGB565 (BE in word)";
        case 0x21: return "BGR565 (BE in word)";
        case 0x00: case 0x01: return "1bpp mono";
        default:   return "UNVERIFIED / unknown";
    }
}

static void decode_f8(const uint8_t *b, CFIndex len, int id_in_buf) {
    // Static analysis of drawBulk: [1..2] width LE16, [3..4] height LE16,
    // [6] DataFormat, [9] == 0x32. Those indices assume the report ID occupies
    // byte 0 of the buffer. If IOKit stripped the ID, shift by one.
    int off = id_in_buf ? 0 : -1;
    int iw = 1 + off, ih = 3 + off, ifm = 6 + off, ichk = 9 + off;
    if (iw < 0 || ichk >= (int)len) { printf("    (buffer too short to decode)\n"); return; }
    unsigned w = (unsigned)b[iw]   | ((unsigned)b[iw+1] << 8);
    unsigned h = (unsigned)b[ih]   | ((unsigned)b[ih+1] << 8);
    unsigned f = b[ifm];
    printf("    assuming report ID %s in buffer:\n",
           id_in_buf ? "PRESENT (byte 0)" : "STRIPPED");
    printf("      width=%u height=%u DataFormat=0x%02X (%s) check[%d]=0x%02X %s\n",
           w, h, f, fmt_name(f), ichk, b[ichk],
           b[ichk] == 0x32 ? "<== 0x32 SELF-CHECK PASSES"
                           : "(expected 0x32; decode SUSPECT)");
}

static void try_get(IOHIDDeviceRef dev, uint8_t rid, CFIndex want) {
    uint8_t buf[256];
    memset(buf, 0, sizeof buf);
    CFIndex len = want;
    IOReturn rc = IOHIDDeviceGetReport(dev, kIOHIDReportTypeFeature, rid, buf, &len);
    if (rc != kIOReturnSuccess) {
        printf("  feature 0x%02X len=%-3ld -> FAIL rc=0x%08X\n",
               rid, (long)want, rc);
        return;
    }
    printf("  feature 0x%02X len=%-3ld -> OK, got %ld bytes:",
           rid, (long)want, (long)len);
    for (CFIndex i = 0; i < len && i < 40; i++) printf(" %02X", buf[i]);
    printf("\n");
    if (rid == 0xF8) {
        decode_f8(buf, len, 1);
        decode_f8(buf, len, 0);
    }
}

int main(void) {
    list_midi();

    IOHIDManagerRef mgr = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
    CFMutableDictionaryRef m = CFDictionaryCreateMutable(
        kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks);
    int vid = NI_VID;
    CFNumberRef vnum = CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &vid);
    CFDictionarySetValue(m, CFSTR(kIOHIDVendorIDKey), vnum);
    IOHIDManagerSetDeviceMatching(mgr, m);
    IOHIDManagerOpen(mgr, kIOHIDOptionsTypeNone);

    CFSetRef devs = IOHIDManagerCopyDevices(mgr);
    if (!devs || CFSetGetCount(devs) == 0) {
        printf("No NI HID device found.\n"); return 2;
    }
    CFIndex n = CFSetGetCount(devs);
    IOHIDDeviceRef *list = calloc((size_t)n, sizeof *list);
    CFSetGetValues(devs, (const void **)list);

    for (CFIndex i = 0; i < n; i++) {
        IOHIDDeviceRef dev = list[i];
        printf("=== HID device %ld ===\n", (long)i);
        IOReturn orc = IOHIDDeviceOpen(dev, kIOHIDOptionsTypeNone);
        printf("IOHIDDeviceOpen rc=0x%08X %s\n", orc,
               orc == kIOReturnSuccess ? "(opened)" : "(not opened; continuing)");

        // Feature report IDs the S88's own report map declares:
        //   F-D0 (29B payload), F-D8 (32B), F-D9 (32B), F-F8 (10B)
        // Try each at its declared size, +1 for an ID byte, and at the max (33).
        printf("-- control: other declared feature reports --\n");
        try_get(dev, 0xD0, 30);
        try_get(dev, 0xD0, 33);
        try_get(dev, 0xD8, 33);
        try_get(dev, 0xD9, 33);
        printf("-- target: the geometry report --\n");
        try_get(dev, 0xF8, 10);
        try_get(dev, 0xF8, 11);
        try_get(dev, 0xF8, 33);
        printf("-- negative control: an ID NOT in the report map --\n");
        try_get(dev, 0x55, 33);

        IOHIDDeviceClose(dev, kIOHIDOptionsTypeNone);
        printf("\n");
    }

    printf("Interpretation guide:\n");
    printf("  all FAIL identically      -> GetReport blocked/unsupported for us\n");
    printf("  others OK, 0xF8 FAIL      -> 0xF8 is SET-only; geometry not readable\n");
    printf("  0xF8 OK at some length    -> geometry readable; see decode above\n");
    printf("  unknown ID 0x55 also OK   -> results are not trustworthy\n");
    printf("\nAll operations were reads. No SetReport was issued.\n");
    free(list); CFRelease(devs); CFRelease(vnum); CFRelease(m); CFRelease(mgr);
    return 0;
}
