import AppKit

let output = CommandLine.arguments.dropFirst().first ?? "ControlRoomIcon.png"
let size = NSSize(width: 1024, height: 1024)
guard
  let bitmap = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: Int(size.width),
    pixelsHigh: Int(size.height),
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
  ),
  let graphics = NSGraphicsContext(bitmapImageRep: bitmap)
else {
  fatalError("could not create icon bitmap")
}
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = graphics

let bounds = NSRect(origin: .zero, size: size)
NSColor.clear.setFill()
bounds.fill()

let tile = NSBezierPath(roundedRect: bounds.insetBy(dx: 72, dy: 72), xRadius: 218, yRadius: 218)
let gradient = NSGradient(colors: [
  NSColor(red: 1.0, green: 0.70, blue: 0.24, alpha: 1),
  NSColor(red: 1.0, green: 0.46, blue: 0.10, alpha: 1),
])!
gradient.draw(in: tile, angle: -45)

let highlight = NSBezierPath(
  roundedRect: bounds.insetBy(dx: 91, dy: 91), xRadius: 198, yRadius: 198)
NSColor.white.withAlphaComponent(0.16).setStroke()
highlight.lineWidth = 7
highlight.stroke()

let waveform = NSBezierPath()
waveform.move(to: NSPoint(x: 236, y: 512))
waveform.line(to: NSPoint(x: 355, y: 512))
waveform.line(to: NSPoint(x: 407, y: 345))
waveform.line(to: NSPoint(x: 480, y: 676))
waveform.line(to: NSPoint(x: 547, y: 424))
waveform.line(to: NSPoint(x: 601, y: 555))
waveform.line(to: NSPoint(x: 663, y: 512))
waveform.line(to: NSPoint(x: 788, y: 512))
waveform.lineCapStyle = .round
waveform.lineJoinStyle = .round
waveform.lineWidth = 46
NSColor(red: 0.035, green: 0.043, blue: 0.055, alpha: 1).setStroke()
waveform.stroke()

NSGraphicsContext.restoreGraphicsState()
guard let png = bitmap.representation(using: .png, properties: [:]) else {
  fatalError("could not render icon")
}
try png.write(to: URL(fileURLWithPath: output))
