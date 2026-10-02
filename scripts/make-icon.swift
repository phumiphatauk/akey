#!/usr/bin/env swift
// Draws the akey app icon (a white key on a blue gradient squircle) and writes
// resources/AppIcon.icns. Run once from the repository root: `swift scripts/make-icon.swift`.
// Every size is drawn from vectors rather than downscaled, so small sizes stay crisp.
// The key is drawn from plain shapes: SF Symbols may not be used in app icons.

import AppKit

let canvas: CGFloat = 1024

/// Draws the icon in a 1024×1024 coordinate space into the current graphics context.
func drawIcon() {
    // macOS icon grid: an 824 pt body centered on the canvas, leaving room for the shadow.
    let body = NSRect(x: 100, y: 100, width: 824, height: 824)
    let squircle = NSBezierPath(roundedRect: body, xRadius: 185, yRadius: 185)

    NSGraphicsContext.saveGraphicsState()
    let bodyShadow = NSShadow()
    bodyShadow.shadowColor = NSColor.black.withAlphaComponent(0.30)
    bodyShadow.shadowOffset = NSSize(width: 0, height: -10)
    bodyShadow.shadowBlurRadius = 24
    bodyShadow.set()
    NSColor(srgbRed: 0.04, green: 0.39, blue: 0.82, alpha: 1).setFill()
    squircle.fill()
    NSGraphicsContext.restoreGraphicsState()

    let gradient = NSGradient(colors: [
        NSColor(srgbRed: 0.40, green: 0.80, blue: 0.99, alpha: 1),  // top: light sky blue
        NSColor(srgbRed: 0.04, green: 0.39, blue: 0.82, alpha: 1),  // bottom: deep blue
    ])!
    gradient.draw(in: squircle, angle: -90)

    // A soft highlight along the top edge.
    NSGraphicsContext.saveGraphicsState()
    squircle.addClip()
    let gloss = NSGradient(colors: [NSColor.white.withAlphaComponent(0.18), NSColor.white.withAlphaComponent(0)])!
    gloss.draw(in: NSRect(x: body.minX, y: body.midY, width: body.width, height: body.height / 2), angle: -90)
    NSGraphicsContext.restoreGraphicsState()

    // The key, built horizontally with its head on the left, then tilted 45° so the shaft
    // points to the lower right.
    let key = NSBezierPath()
    key.windingRule = .evenOdd
    key.appendOval(in: NSRect(x: -150, y: -150, width: 300, height: 300))  // head
    key.appendOval(in: NSRect(x: -58, y: -58, width: 116, height: 116))  // hole
    let shaft = NSBezierPath(roundedRect: NSRect(x: 110, y: -40, width: 330, height: 80), xRadius: 14, yRadius: 14)
    let tooth1 = NSBezierPath(roundedRect: NSRect(x: 376, y: -140, width: 64, height: 120), xRadius: 12, yRadius: 12)
    let tooth2 = NSBezierPath(roundedRect: NSRect(x: 262, y: -112, width: 56, height: 92), xRadius: 12, yRadius: 12)

    var transform = AffineTransform(translationByX: 512, byY: 512)
    transform.rotate(byDegrees: -45)
    transform.translate(x: -145, y: 18)
    for path in [key, shaft, tooth1, tooth2] { path.transform(using: transform) }

    NSGraphicsContext.saveGraphicsState()
    let keyShadow = NSShadow()
    keyShadow.shadowColor = NSColor.black.withAlphaComponent(0.25)
    keyShadow.shadowOffset = NSSize(width: 0, height: -12)
    keyShadow.shadowBlurRadius = 22
    keyShadow.set()
    NSColor.white.setFill()
    // Drawn as one layer so the shadow is cast once for the whole key.
    NSGraphicsContext.current!.cgContext.beginTransparencyLayer(auxiliaryInfo: nil)
    for path in [key, shaft, tooth1, tooth2] { path.fill() }
    NSGraphicsContext.current!.cgContext.endTransparencyLayer()
    NSGraphicsContext.restoreGraphicsState()
}

func png(pixels: Int) -> Data {
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSGraphicsContext.current!.imageInterpolation = .high
    let scale = CGFloat(pixels) / canvas
    NSGraphicsContext.current!.cgContext.scaleBy(x: scale, y: scale)
    drawIcon()
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

let fm = FileManager.default
let iconset = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("AppIcon.iconset")
try? fm.removeItem(at: iconset)
try! fm.createDirectory(at: iconset, withIntermediateDirectories: true)
for points in [16, 32, 128, 256, 512] {
    try! png(pixels: points).write(to: iconset.appendingPathComponent("icon_\(points)x\(points).png"))
    try! png(pixels: points * 2).write(to: iconset.appendingPathComponent("icon_\(points)x\(points)@2x.png"))
}

try! fm.createDirectory(atPath: "resources", withIntermediateDirectories: true)
try! png(pixels: 1024).write(to: URL(fileURLWithPath: "resources/AppIcon.png"))
let iconutil = Process()
iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
iconutil.arguments = ["-c", "icns", iconset.path, "-o", "resources/AppIcon.icns"]
try! iconutil.run()
iconutil.waitUntilExit()
try? fm.removeItem(at: iconset)
print(iconutil.terminationStatus == 0 ? "Wrote resources/AppIcon.icns" : "iconutil failed")
