import AppKit
import SwiftUI

@main
struct CTXApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        Window("CTX", id: "main") {
            DashboardView()
        }
        .defaultSize(width: 1080, height: 740)
        .commands {
            CommandGroup(replacing: .newItem) {}
        }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let model = StatusModel()
    private var statusItem: NSStatusItem?
    private var titleObserver: NSObjectProtocol?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        ProcessInfo.processInfo.disableAutomaticTermination("CTX app")
        ProcessInfo.processInfo.disableSuddenTermination()
        Self.forceMenuBarVisible()
        installTray()
        DashboardLoader.shared.start()
        showMainWindow()

        titleObserver = NotificationCenter.default.addObserver(
            forName: .ctxStatusDidChange,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor in
                self?.refreshTitle()
            }
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showMainWindow()
        return true
    }

    func applicationWillTerminate(_ notification: Notification) {
        if let titleObserver {
            NotificationCenter.default.removeObserver(titleObserver)
        }
    }

    static func forceMenuBarVisible() {
        let keysVisible = [
            "NSStatusItem Visible ai.ctx.bar",
            "NSStatusItem VisibleCC ai.ctx.bar",
        ]
        let app = UserDefaults.standard
        for key in keysVisible {
            app.set(true, forKey: key)
        }
        if let cc = UserDefaults(suiteName: "com.apple.controlcenter") {
            for key in keysVisible {
                cc.set(true, forKey: key)
            }
            cc.set(Float(700), forKey: "NSStatusItem Preferred Position ai.ctx.bar")
        }
    }

    private func installTray() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        item.autosaveName = "ai.ctx.bar"
        item.isVisible = true
        if let button = item.button {
            button.image = Self.menuBarImage()
            button.imagePosition = .imageLeft
            button.title = "CTX"
            button.font = NSFont.menuBarFont(ofSize: 13)
            button.toolTip = "CTX"
            button.target = self
            button.action = #selector(handleTray(_:))
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
        }
        statusItem = item
        refreshTitle()
    }

    @objc private func handleTray(_ sender: Any?) {
        if NSApp.currentEvent?.type == .rightMouseUp {
            showTrayMenu()
            return
        }
        showMainWindow()
    }

    private func showTrayMenu() {
        let menu = NSMenu()
        menu.addItem(withTitle: "打开 CTX", action: #selector(openMain(_:)), keyEquivalent: "")
        let pause = NSMenuItem(
            title: model.enabled ? "暂停" : "继续",
            action: #selector(toggleEnabled(_:)),
            keyEquivalent: ""
        )
        menu.addItem(pause)
        menu.addItem(.separator())
        menu.addItem(withTitle: "退出 CTX", action: #selector(quitApp(_:)), keyEquivalent: "q")
        for item in menu.items {
            item.target = self
        }
        statusItem?.menu = menu
        statusItem?.button?.performClick(nil)
        statusItem?.menu = nil
        statusItem?.button?.target = self
        statusItem?.button?.action = #selector(handleTray(_:))
    }

    @objc private func openMain(_ sender: Any?) {
        showMainWindow()
    }

    @objc private func toggleEnabled(_ sender: Any?) {
        model.setEnabled(!model.enabled)
        refreshTitle()
    }

    @objc private func quitApp(_ sender: Any?) {
        NSApp.terminate(nil)
    }

    func showMainWindow() {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        if let window = NSApp.windows.first(where: { $0.canBecomeMain }) {
            window.makeKeyAndOrderFront(nil)
            return
        }
        if let window = NSApp.windows.first {
            window.makeKeyAndOrderFront(nil)
        }
    }

    private func refreshTitle() {
        statusItem?.isVisible = true
        let label = model.menuLabel
        statusItem?.button?.title = (label == "CTX") ? "CTX" : "CTX \(label)"
        statusItem?.button?.image = Self.menuBarImage()
    }

    private static func menuBarImage() -> NSImage {
        if let image = NSImage(
            systemSymbolName: "square.stack.3d.up.fill",
            accessibilityDescription: "CTX"
        ) {
            let sized = image.withSymbolConfiguration(.init(pointSize: 13, weight: .medium)) ?? image
            sized.isTemplate = true
            return sized
        }
        return NSImage(size: NSSize(width: 14, height: 14))
    }
}

extension Notification.Name {
    static let ctxStatusDidChange = Notification.Name("ai.ctx.bar.statusDidChange")
}
