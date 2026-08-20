import AppKit
import SwiftUI

@main
struct CTXBarApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        // Keeps the SwiftUI app lifecycle alive for an accessory (menu-bar) app.
        Settings {
            EmptyView()
        }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let model = StatusModel()
    private var statusItem: NSStatusItem?
    private var popover: NSPopover?
    private var titleObserver: NSObjectProtocol?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)

        let popover = NSPopover()
        popover.behavior = .transient
        popover.animates = true
        popover.contentSize = NSSize(width: 336, height: 560)
        popover.contentViewController = NSHostingController(
            rootView: PopoverView().environmentObject(model)
        )
        self.popover = popover

        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = item.button {
            button.image = Self.menuBarImage()
            button.imagePosition = .imageLeft
            button.imageHugsTitle = true
            button.title = model.menuLabel
            button.toolTip = "CTX"
            button.target = self
            button.action = #selector(togglePopover(_:))
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
        }
        statusItem = item
        refreshTitle()

        titleObserver = NotificationCenter.default.addObserver(
            forName: .ctxStatusDidChange,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor in
                self?.refreshTitle()
            }
        }

        // Ensure login item points at this bundle when already enabled.
        if LoginItem.isEnabled {
            try? LoginItem.setEnabled(true)
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        if let titleObserver {
            NotificationCenter.default.removeObserver(titleObserver)
        }
    }

    @objc private func togglePopover(_ sender: Any?) {
        guard let button = statusItem?.button, let popover else { return }
        if popover.isShown {
            popover.performClose(sender)
            return
        }
        model.refresh()
        refreshTitle()
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        popover.contentViewController?.view.window?.makeKey()
    }

    private func refreshTitle() {
        statusItem?.button?.title = " \(model.menuLabel)"
        statusItem?.button?.image = Self.menuBarImage()
    }

    private static func menuBarImage() -> NSImage {
        let names = ["ctx-menubar", "ctx-mark"]
        for name in names {
            if let url = Bundle.main.url(forResource: name, withExtension: "png"),
               let image = NSImage(contentsOf: url)
            {
                let h: CGFloat = 16
                let scale = h / max(image.size.height, 1)
                let size = NSSize(width: max(16, image.size.width * scale), height: h)
                image.size = size
                image.isTemplate = true
                return image
            }
        }
        let fallback = NSImage(systemSymbolName: "circle.grid.cross", accessibilityDescription: "CTX")
        fallback?.isTemplate = true
        return fallback ?? NSImage(size: NSSize(width: 16, height: 16))
    }
}

extension Notification.Name {
    static let ctxStatusDidChange = Notification.Name("ai.ctx.bar.statusDidChange")
}
