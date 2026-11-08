// Renders an SVG to a square PNG with WebKit, which supports gradients and filters.
// Usage: swift scripts/render-svg.swift <input.svg> <output.png> <size>
import AppKit
import WebKit

let args = CommandLine.arguments
guard args.count == 4, let size = Int(args[3]) else {
    FileHandle.standardError.write("usage: render-svg.swift <input.svg> <output.png> <size>\n".data(using: .utf8)!)
    exit(2)
}
let input = URL(fileURLWithPath: args[1])
let output = URL(fileURLWithPath: args[2])
guard let svg = try? String(contentsOf: input, encoding: .utf8) else {
    FileHandle.standardError.write("cannot read \(input.path)\n".data(using: .utf8)!)
    exit(1)
}

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)

final class Renderer: NSObject, WKNavigationDelegate {
    let web: WKWebView
    let size: Int
    let output: URL

    init(svg: String, size: Int, output: URL) {
        self.size = size
        self.output = output
        let config = WKWebViewConfiguration()
        web = WKWebView(frame: NSRect(x: 0, y: 0, width: size, height: size), configuration: config)
        web.setValue(false, forKey: "drawsBackground")  // keep the transparent margins
        super.init()
        web.navigationDelegate = self
        let html = """
        <!doctype html><html><head><style>
        html,body{margin:0;padding:0;background:transparent;overflow:hidden}
        svg{display:block;width:\(size)px;height:\(size)px}
        </style></head><body>\(svg)</body></html>
        """
        web.loadHTMLString(html, baseURL: nil)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        let snapshot = WKSnapshotConfiguration()
        snapshot.rect = NSRect(x: 0, y: 0, width: size, height: size)
        snapshot.snapshotWidth = NSNumber(value: size)  // in points; scale handled below
        webView.takeSnapshot(with: snapshot) { image, error in
            guard let image else {
                FileHandle.standardError.write("snapshot failed: \(String(describing: error))\n".data(using: .utf8)!)
                exit(1)
            }
            // Redraw into a bitmap of exactly size×size pixels, whatever the screen scale.
            let rep = NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: self.size, pixelsHigh: self.size,
                bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
            rep.size = NSSize(width: self.size, height: self.size)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
            NSGraphicsContext.current?.imageInterpolation = .high
            image.draw(in: NSRect(x: 0, y: 0, width: self.size, height: self.size))
            NSGraphicsContext.restoreGraphicsState()
            do {
                try rep.representation(using: .png, properties: [:])!.write(to: self.output)
                exit(0)
            } catch {
                FileHandle.standardError.write("write failed: \(error)\n".data(using: .utf8)!)
                exit(1)
            }
        }
    }
}

let renderer = Renderer(svg: svg, size: size, output: output)
DispatchQueue.main.asyncAfter(deadline: .now() + 20) {
    FileHandle.standardError.write("timed out rendering \(input.path)\n".data(using: .utf8)!)
    exit(1)
}
_ = renderer
app.run()
