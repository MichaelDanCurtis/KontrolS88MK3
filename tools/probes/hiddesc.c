// Dump the S88's ACTUAL HID report descriptor and list every declared report
// ID by type. Read-only: reads the kIOHIDReportDescriptorKey property only.
#include <stdio.h>
#include <string.h>
#include <IOKit/hid/IOHIDManager.h>
#define NI_VID 0x17CC
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
    for(CFIndex d=0; d<n; d++){
        CFTypeRef r = IOHIDDeviceGetProperty(l[d], CFSTR(kIOHIDReportDescriptorKey));
        if(!r || CFGetTypeID(r)!=CFDataGetTypeID()){ printf("device %ld: no descriptor property\n",(long)d); continue; }
        CFDataRef data=(CFDataRef)r; CFIndex len=CFDataGetLength(data);
        const uint8_t *p=CFDataGetBytePtr(data);
        printf("=== device %ld: HID report descriptor, %ld bytes ===\n",(long)d,(long)len);
        // Walk HID items. Track Report ID (0x85) and Input/Output/Feature main items.
        int cur_id=-1;
        printf("declared reports:\n");
        for(CFIndex i=0;i<len;){
            uint8_t b=p[i]; uint8_t sz=b&0x03; if(sz==3) sz=4;
            uint8_t tag=b&0xFC;
            unsigned val=0; for(uint8_t k=0;k<sz;k++) val |= (unsigned)p[i+1+k]<<(8*k);
            if(tag==0x84) cur_id=(int)val;                 // Report ID (global 0x85 w/ size)
            else if(tag==0x80) printf("  INPUT   id=0x%02X\n", cur_id);
            else if(tag==0x90) printf("  OUTPUT  id=0x%02X\n", cur_id);
            else if(tag==0xB0) printf("  FEATURE id=0x%02X\n", cur_id);
            i += 1+sz;
        }
        printf("\nraw descriptor hex:\n");
        for(CFIndex i=0;i<len;i++){ printf("%02X ", p[i]); if((i%16)==15) printf("\n"); }
        printf("\n");
    }
    return 0;
}
