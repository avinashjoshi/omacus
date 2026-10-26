import Cocoa
import WebKit

// A separate Mac app for the sync test. It is not Omacus.
// Bundle id dev.omacus.OmacusTest keeps this window's pages in WebKit's
// store for that id. It does not open the real app's pages.
// This build does not register the omacus:// link, so it does not take links
// from the installed app. Desktop sync in Omacus itself stays a later version.

private let testBundleID = "dev.omacus.OmacusTest"
private let syncURL = URL(string: "https://sync-test.omacus.com/")!
private let testUserAgent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) omacus-test/1.0"

final class TestApp: NSObject, NSApplicationDelegate {
    let window: NSWindow
    let web: WKWebView

    override init() {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = WKWebsiteDataStore.default()
        web = WKWebView(frame: .zero, configuration: config)
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1100, height: 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "omacus Test"
        window.contentView = web
        window.minSize = NSSize(width: 420, height: 320)
        super.init()
        web.customUserAgent = testUserAgent
        web.navigationDelegate = self
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let bundleID = Bundle.main.bundleIdentifier ?? ""
        if bundleID != testBundleID {
            fputs("omacus Test refused to open because its id is \(bundleID).\n", stderr)
            NSApp.terminate(nil)
            return
        }
        window.center()
        window.makeKeyAndOrderFront(nil)
        if #available(macOS 14.0, *) {
            NSApp.activate()
        } else {
            NSApp.activate(ignoringOtherApps: true)
        }
        web.load(URLRequest(url: syncURL))
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

extension TestApp: WKNavigationDelegate {}

let app = NSApplication.shared
let delegate = TestApp()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
