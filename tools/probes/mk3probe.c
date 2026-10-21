// mk3probe — READ-ONLY probe for Native Instruments Kontrol MK3 display geometry.
//
// Reads HID feature report 0xF8, which all three MK3 keyboards declare in their
// own report map as "F-F8 W2 B6" (2 words + 6 bytes = 10 bytes). Static analysis
// of NIHardwareAgent's drawBulk shows that report's payload being read as:
//
//     [1..2] width  LE16
//     [3..4] height LE16
//     [6]    DataFormat
//     [9]    0x32          <-- acts as a self-check on the layout
//
// This program ONLY issues IOHIDDeviceGetReport (a read of a device-declared
// feature report). It never calls SetReport and never writes to the device.
//
// Build: cc -o mk3probe mk3probe.c -framework IOKit -framework CoreFoundation

#include <stdio.h>
#include <string.h>
#include <IOKit/hid/IOHIDManager.h>

#define NI_VID 0x17CC

static int32_t int_prop(IOHIDDeviceRef d, CFStringRef key) {
    int32_t v = -1;
    CFTypeRef r = IOHIDDeviceGetProperty(d, key);
    if (r && CFGetTypeID(r) == CFNumberGetTypeID())
        CFNumberGetValue((CFNumberRef)r, kCFNumberSInt32Type, &v);
    return v;
}

static void str_prop(IOHIDDeviceRef d, CFStringRef key, char *out, size_t n) {
    out[0] = '\0';
    CFTypeRef r = IOHIDDeviceGetProperty(d, key);
    if (r && CFGetTypeID(r) == CFStringGetTypeID())
        CFStringGetCString((CFStringRef)r, out, (CFIndex)n, kCFStringEncodingUTF8);
}

static const char *fmt_name(unsigned f) {
    unsigned c = (f & ~0x40u);
    switch (c) {
        case 0x10: return "XRGB8888 (FF,R,G,B)";
        case 0x11: return "RGBX8888 (R,G,B,FF)";
        case 0x12: return "ARGB8888";
        case 0x13: return "RGBA8888";
        case 0x14: return "XBGR8888 (FF,B,G,R)";
        case 0x15: return "BGRX8888";
        case 0x16: return "ABGR8888";
        case 0x17: return "BGRA8888";
        case 0x18: return "XRGB1555";
        case 0x19: return "R5G5B6";
        case 0x1A: return "ARGB1555";
        case 0x20: return "RGB565 (big-endian in word)";
        case 0x21: return "BGR565 (big-endian in word)";
        case 0x00:
        case 0x01: return "1bpp mono";
        default:   return "UNVERIFIED / unknown";
    }
}

static void probe(IOHIDDeviceRef dev) {
    int32_t vid = int_prop(dev, CFSTR(kIOHIDVendorIDKey));
    int32_t pid = int_prop(dev, CFSTR(kIOHIDProductIDKey));
    char product[256], manuf[256];
    str_prop(dev, CFSTR(kIOHIDProductKey), product, sizeof product);
    str_prop(dev, CFSTR(kIOHIDManufacturerKey), manuf, sizeof manuf);

    printf("--------------------------------------------------------\n");
    printf("device : VID 0x%04X  PID 0x%04X\n", vid, pid);
    printf("product: %s\n", product[0] ? product : "(none)");
    printf("vendor : %s\n", manuf[0] ? manuf : "(none)");
    printf("maxFeatureReport: %d bytes\n",
           int_prop(dev, CFSTR(kIOHIDMaxFeatureReportSizeKey)));

    IOReturn open_rc = IOHIDDeviceOpen(dev, kIOHIDOptionsTypeNone);
    if (open_rc != kIOReturnSuccess)
        printf("note   : IOHIDDeviceOpen returned 0x%08X (continuing; "
               "GetReport can still work, and NI's agent may hold the device)\n",
               open_rc);

    uint8_t buf[64];
    memset(buf, 0, sizeof buf);
    CFIndex len = 10;                       // report map declares exactly 10 bytes
    IOReturn rc = IOHIDDeviceGetReport(dev, kIOHIDReportTypeFeature, 0xF8, buf, &len);

    if (rc != kIOReturnSuccess) {
        printf("feature 0xF8: FAILED rc=0x%08X  (len now %ld)\n", rc, (long)len);
        printf("  -> read did not succeed; nothing was written to the device.\n");
    } else {
        printf("feature 0xF8: OK, %ld bytes\n", (long)len);
        printf("  raw:");
        for (CFIndex i = 0; i < len; i++) printf(" %02X", buf[i]);
        printf("\n");
        if (len >= 10) {
            unsigned w  = (unsigned)buf[1] | ((unsigned)buf[2] << 8);   // LE16
            unsigned h  = (unsigned)buf[3] | ((unsigned)buf[4] << 8);   // LE16
            unsigned f  = buf[6];
            printf("  INTERPRETED (per NIHardwareAgent drawBulk):\n");
            printf("    width      = %u\n", w);
            printf("    height     = %u\n", h);
            printf("    DataFormat = 0x%02X  -> %s\n", f, fmt_name(f));
            printf("    byte[9]    = 0x%02X  %s\n", buf[9],
                   buf[9] == 0x32 ? "(== 0x32, layout self-check PASSES)"
                                  : "(expected 0x32 -- layout interpretation SUSPECT)");
        }
    }
    IOHIDDeviceClose(dev, kIOHIDOptionsTypeNone);
}

int main(void) {
    IOHIDManagerRef mgr = IOHIDManagerCreate(kCFAllocatorDefault,
                                             kIOHIDOptionsTypeNone);
    if (!mgr) { fprintf(stderr, "IOHIDManagerCreate failed\n"); return 1; }

    // Match on vendor only, so we see every NI HID interface present.
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
        printf("No Native Instruments HID devices (VID 0x%04X) found.\n", NI_VID);
        printf("If the MK3 is plugged in and powered, macOS may not have "
               "enumerated its HID interface yet, or it may need Input "
               "Monitoring permission for this terminal.\n");
        if (devs) CFRelease(devs);
        CFRelease(vnum); CFRelease(m); CFRelease(mgr);
        return 2;
    }

    CFIndex n = CFSetGetCount(devs);
    printf("Found %ld Native Instruments HID device(s).\n", (long)n);
    IOHIDDeviceRef *list = calloc((size_t)n, sizeof *list);
    CFSetGetValues(devs, (const void **)list);
    for (CFIndex i = 0; i < n; i++) probe(list[i]);
    printf("--------------------------------------------------------\n");
    printf("Done. This probe performed reads only; no SetReport was issued.\n");

    free(list);
    CFRelease(devs); CFRelease(vnum); CFRelease(m); CFRelease(mgr);
    return 0;
}
