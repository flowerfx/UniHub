import AppKit
import SwiftUI

@main
struct UniHubApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var model = AppModel()
    @AppStorage(MenuBarMenu.showKey) private var showMenuBarIcon = true

    var body: some Scene {
        // One main window: "Open UniHub" in the menu bar brings it back instead of adding more.
        Window("UniHub", id: MenuBarMenu.mainWindowID) {
            ContentView()
                .environment(model)
                .frame(minWidth: 900, minHeight: 520)
                .task {
                    await model.refreshLocal()
                    await model.checkForUpdateIfDue()
                }
                .onAppear { model.applyAppearance() }
        }
        .defaultSize(width: 1280, height: 800)
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Check for Updates…") { Task { await model.checkForUpdate(manual: true) } }
                    .disabled(model.isCheckingForUpdate || model.isInstallingUpdate)
            }
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

        // Menu bar icon with recent projects, like the official Hub's.
        MenuBarExtra(isInserted: $showMenuBarIcon) {
            MenuBarMenu(model: model)
        } label: {
            Image(systemName: "point.3.filled.connected.trianglepath.dotted")
                .accessibilityLabel("UniHub")
        }
        .menuBarExtraStyle(.menu)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Needed when started as a bare executable (`swift run`) rather than an .app.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
    }

    /// With the menu bar icon, closing the window keeps UniHub running there.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        !(UserDefaults.standard.object(forKey: MenuBarMenu.showKey) as? Bool ?? true)
    }
}
