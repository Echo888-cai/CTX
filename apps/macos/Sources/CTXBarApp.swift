import SwiftUI

@main
struct CTXBarApp: App {
    @StateObject private var model = StatusModel()

    var body: some Scene {
        MenuBarExtra {
            PopoverView()
                .environmentObject(model)
        } label: {
            HStack(spacing: 4) {
                Image(systemName: model.enabled ? "memorychip" : "pause.circle")
                Text(model.menuLabel)
                    .monospacedDigit()
            }
        }
        .menuBarExtraStyle(.window)
    }
}
