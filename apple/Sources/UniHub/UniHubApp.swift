import AppKit
import SwiftUI

@main
struct UniHubApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var model = AppModel()

    var body: some Scene {
        WindowGroup("UniHub") {
            ContentView()
                .environment(model)
                .frame(minWidth: 900, minHeight: 520)
                .task { await model.refreshLocal() }
                .onAppear { model.applyAppearance() }
        }
        .defaultSize(width: 1280, height: 800)
        .commands {
            CommandGroup(after: .newItem) {
                Button("Add Project…") { model.isAddingProject = true }
                    .keyboardShortcut("o")
                Button("Refresh") { Task { await model.refreshLocal() } }
                    .keyboardShortcut("r")
            }
        }

        Settings {
            SettingsView()
                .environment(model)
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Needed when started as a bare executable (`swift run`) rather than an .app.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}
