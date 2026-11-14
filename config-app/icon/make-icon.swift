// Рисует иконку приложения: синий фон, белая папка и самолётик.
// Запуск: swift make-icon.swift <выходной.png>
import AppKit

let size: CGFloat = 1024
let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "icon.png"

let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size), pixelsHigh: Int(size),
                           bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                           colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

// Основа по сетке иконок macOS: 824×824 с отступом 100
let body = NSRect(x: 100, y: 100, width: 824, height: 824)
let shape = NSBezierPath(roundedRect: body, xRadius: 185, yRadius: 185)

// Тень
NSGraphicsContext.saveGraphicsState()
let shadow = NSShadow()
shadow.shadowColor = NSColor.black.withAlphaComponent(0.3)
shadow.shadowBlurRadius = 28
shadow.shadowOffset = NSSize(width: 0, height: -12)
shadow.set()
NSColor.black.setFill()
shape.fill()
NSGraphicsContext.restoreGraphicsState()

// Градиент фона
let gradient = NSGradient(colors: [
    NSColor(srgbRed: 0.33, green: 0.75, blue: 0.96, alpha: 1),
    NSColor(srgbRed: 0.12, green: 0.53, blue: 0.85, alpha: 1),
])!
gradient.draw(in: shape, angle: -90)

func symbol(_ name: String, pointSize: CGFloat, weight: NSFont.Weight, colors: [NSColor]) -> NSImage {
    let cfg = NSImage.SymbolConfiguration(pointSize: pointSize, weight: weight)
        .applying(NSImage.SymbolConfiguration(paletteColors: colors))
    return NSImage(systemSymbolName: name, accessibilityDescription: nil)!.withSymbolConfiguration(cfg)!
}

func draw(_ img: NSImage, centerX: CGFloat, centerY: CGFloat, shadowAlpha: CGFloat) {
    NSGraphicsContext.saveGraphicsState()
    let s = NSShadow()
    s.shadowColor = NSColor.black.withAlphaComponent(shadowAlpha)
    s.shadowBlurRadius = 18
    s.shadowOffset = NSSize(width: 0, height: -8)
    s.set()
    let r = NSRect(x: centerX - img.size.width / 2, y: centerY - img.size.height / 2,
                   width: img.size.width, height: img.size.height)
    img.draw(in: r)
    NSGraphicsContext.restoreGraphicsState()
}

// Белая папка
let folder = symbol("folder.fill", pointSize: 520, weight: .regular, colors: [.white])
draw(folder, centerX: 512, centerY: 480, shadowAlpha: 0.25)

// Синий самолётик на папке
let plane = symbol("paperplane.fill", pointSize: 250, weight: .semibold,
                   colors: [NSColor(srgbRed: 0.12, green: 0.53, blue: 0.85, alpha: 1)])
draw(plane, centerX: 512, centerY: 440, shadowAlpha: 0.0)

NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
print("Сохранено: \(out)")
