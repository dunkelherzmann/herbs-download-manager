// Draws an SVG at one size with AppKit's own SVG renderer, adding the tile's shadow (it ignores SVG filters).
// Usage: render IN.svg OUT.png SIZE
import AppKit
let a = CommandLine.arguments
guard let img = NSImage(contentsOfFile: a[1]) else { print("no image"); exit(1) }
let size = Int(a[3])!
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let scale = CGFloat(size) / 1024
let shadow = NSShadow()
shadow.shadowOffset = NSSize(width: 0, height: -10 * scale)
shadow.shadowBlurRadius = 24 * scale
shadow.shadowColor = NSColor.black.withAlphaComponent(0.32)
if size >= 64 { shadow.set() }
img.draw(in: NSRect(x: 0, y: 0, width: size, height: size))
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: a[2]))
