// mk3probe3 — READ-ONLY. Retry feature report 0xF8 replicating exactly what
// NI's GenericHID::initialize does before the GET:
//   HIDReport(buf, 10, 0xF8+i); buf[0]=0xF8+i; buf[8]=0x50; buf[9]=0x32;
//   IOHIDDeviceGetReport(dev, kIOHIDReportTypeFeature, buf[0], buf, &len);
// Only GetReport is used. No SetReport. Bulk pipe untouched.
#include <stdio.h>
#include <string.h>
#include <IOKit/hid/IOHIDManager.h>
#define NI_VID 0x17CC

static const char *fmt_name(unsigned f){switch(f&~0x40u){
 case 0x10:return "XRGB8888";case 0x11:return "RGBX8888";case 0x12:return "ARGB8888";
 case 0x13:return "RGBA8888";case 0x14:return "XBGR8888";case 0x15:return "BGRX8888";
 case 0x16:return "ABGR8888";case 0x17:return "BGRA8888";case 0x18:return "XRGB1555";
 case 0x19:return "R5G5B6";case 0x1A:return "ARGB1555";
 case 0x20:return "RGB565 (BE in word)";case 0x21:return "BGR565 (BE in word)";
 case 0x00:case 0x01:return "1bpp mono";default:return "UNVERIFIED/unknown";}}

static void attempt(IOHIDDeviceRef d, const char *label, uint8_t rid,
                    int seed_id, int seed_hints, CFIndex want) {
    uint8_t b[64]; memset(b,0,sizeof b);
    if (seed_id)    b[0] = rid;
    if (seed_hints) { b[8] = 0x50; b[9] = 0x32; }
    CFIndex len = want;
    IOReturn rc = IOHIDDeviceGetReport(d, kIOHIDReportTypeFeature, rid, b, &len);
    printf("  %-44s rid=0x%02X len=%-3ld -> ", label, rid, (long)want);
    if (rc != kIOReturnSuccess) { printf("FAIL 0x%08X\n", rc); return; }
    printf("OK %ld bytes:", (long)len);
    for (CFIndex i=0;i<len && i<16;i++) printf(" %02X", b[i]);
    printf("\n");
    if (len >= 10) {
        unsigned w = (unsigned)b[1] | ((unsigned)b[2]<<8);
        unsigned h = (unsigned)b[3] | ((unsigned)b[4]<<8);
        unsigned f = b[6];
        printf("      -> width=%u height=%u DataFormat=0x%02X (%s)  bulkbuf=%u bytes\n",
               w, h, f, fmt_name(f), w*4*h + 0x50);
    }
}

int main(void){
    IOHIDManagerRef mgr=IOHIDManagerCreate(kCFAllocatorDefault,kIOHIDOptionsTypeNone);
    CFMutableDictionaryRef m=CFDictionaryCreateMutable(kCFAllocatorDefault,0,
        &kCFTypeDictionaryKeyCallBacks,&kCFTypeDictionaryValueCallBacks);
    int vid=NI_VID; CFNumberRef v=CFNumberCreate(kCFAllocatorDefault,kCFNumberIntType,&vid);
    CFDictionarySetValue(m,CFSTR(kIOHIDVendorIDKey),v);
    IOHIDManagerSetDeviceMatching(mgr,m); IOHIDManagerOpen(mgr,kIOHIDOptionsTypeNone);
    CFSetRef s=IOHIDManagerCopyDevices(mgr);
    if(!s||CFSetGetCount(s)==0){printf("no NI HID device\n");return 2;}
    CFIndex n=CFSetGetCount(s); IOHIDDeviceRef *l=calloc((size_t)n,sizeof *l);
    CFSetGetValues(s,(const void**)l);
    for(CFIndex i=0;i<n;i++){
        printf("=== device %ld ===\n",(long)i);
        printf("open rc=0x%08X\n", IOHIDDeviceOpen(l[i],kIOHIDOptionsTypeNone));
        attempt(l[i],"zeroed buffer (what I tried before)",0xF8,0,0,10);
        attempt(l[i],"buf[0]=rid",                         0xF8,1,0,10);
        attempt(l[i],"buf[0]=rid + hints 0x50/0x32  <= NI",0xF8,1,1,10);
        attempt(l[i],"buf[0]=rid + hints, len 11",         0xF8,1,1,11);
        attempt(l[i],"buf[0]=rid + hints, len 33",         0xF8,1,1,33);
        attempt(l[i],"display index 1 (0xF9)",             0xF9,1,1,10);
        attempt(l[i],"display index 2 (0xFA)",             0xFA,1,1,10);
        IOHIDDeviceClose(l[i],kIOHIDOptionsTypeNone);
    }
    printf("\nAll reads. No SetReport issued.\n");
    return 0;
}
