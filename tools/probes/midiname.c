// Read-only: show how the S88's MIDI ports present at each CoreMIDI level,
// because NI's Cubase script matches the literal string "KONTROL S88 MK3 DAW".
#include <stdio.h>
#include <CoreMIDI/CoreMIDI.h>
static void s(MIDIObjectRef o, CFStringRef k, char *b, size_t n) {
    b[0]=0; CFStringRef v=NULL;
    if (MIDIObjectGetStringProperty(o,k,&v)==noErr && v) {
        CFStringGetCString(v,b,(CFIndex)n,kCFStringEncodingUTF8); CFRelease(v); }
}
int main(void){
    ItemCount nd = MIDIGetNumberOfDevices();
    printf("MIDI devices: %lu\n", (unsigned long)nd);
    for (ItemCount d=0; d<nd; d++){
        MIDIDeviceRef dev = MIDIGetDevice(d);
        char dn[256], dmodel[256], dmanu[256];
        s(dev, kMIDIPropertyName, dn, sizeof dn);
        s(dev, kMIDIPropertyModel, dmodel, sizeof dmodel);
        s(dev, kMIDIPropertyManufacturer, dmanu, sizeof dmanu);
        printf("\ndevice[%lu] name=\"%s\" model=\"%s\" manuf=\"%s\"\n",
               (unsigned long)d, dn, dmodel, dmanu);
        ItemCount ne = MIDIDeviceGetNumberOfEntities(dev);
        for (ItemCount e=0;e<ne;e++){
            MIDIEntityRef ent = MIDIDeviceGetEntity(dev,e);
            char en[256]; s(ent, kMIDIPropertyName, en, sizeof en);
            printf("  entity[%lu] name=\"%s\"\n", (unsigned long)e, en);
            ItemCount ns=MIDIEntityGetNumberOfSources(ent);
            ItemCount nq=MIDIEntityGetNumberOfDestinations(ent);
            for(ItemCount i=0;i<ns;i++){ char n2[256];
                s(MIDIEntityGetSource(ent,i), kMIDIPropertyName, n2, sizeof n2);
                printf("    src  endpoint name=\"%s\"   => combined \"%s %s\"\n", n2, dn, n2); }
            for(ItemCount i=0;i<nq;i++){ char n2[256];
                s(MIDIEntityGetDestination(ent,i), kMIDIPropertyName, n2, sizeof n2);
                printf("    dest endpoint name=\"%s\"   => combined \"%s %s\"\n", n2, dn, n2); }
        }
    }
    return 0;
}
