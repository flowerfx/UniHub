import AppKit
import SwiftUI
import UniHubCore

/// The menu of UniHub's menu bar icon: recent projects, the Unity account, help, and
/// opening or quitting UniHub (the official Hub has the same menu).
struct MenuBarMenu: View {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow

    static let mainWindowID = "main"
    /// UserDefaults key of Settings → Appearance → "Show UniHub in the menu bar".
    static let showKey = "showMenuBarIcon"

    var body: some View {
        let recent = model.recentProjects
        if recent.isEmpty {
            Text("No recent projects")
        }
        ForEach(recent) { project in
            Button(project.title) { Task { await model.open(project) } }
                .disabled(!model.isInstalled(project.editorVersion))
        }

        Divider()

        Button("Go to Unity Dashboard") { model.openURL("https://cloud.unity.com") }
        if let auth = model.auth, auth.signedIn {
            Button("Sign out \(auth.displayName)") { Task { await model.signOut() } }
        } else if model.cli != nil && !model.isSigningIn {
            Button("Sign In…") { Task { await model.signIn() } }
        }

        Divider()

        Menu("Help and support") {
            Button("UniHub on GitHub") { model.openURL("https://github.com/flowerfx/UniHub") }
            Button("Release Notes") { model.openURL("https://github.com/flowerfx/UniHub/releases") }
            Button("Report an Issue") { model.openURL("https://github.com/flowerfx/UniHub/issues/new") }
            Divider()
            Button("Unity Manual") { model.openURL("https://docs.unity3d.com/Manual/index.html") }
            Button("Unity Discussions") { model.openURL("https://discussions.unity.com") }
            Divider()
            CheckForUpdatesButton(model: model)
        }
        Button("Open UniHub") { MainWindow.show(openWindow) }
        Button("Quit UniHub") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
    }
}

/// Brings the main window to the front, or opens it when it was closed. Works from AppKit
/// code too (the Dock icon), using SwiftUI's open action kept from a view.
@MainActor
enum MainWindow {
    /// Set when the main window or the menu bar menu first appears.
    static var openAction: OpenWindowAction?

    static func show(_ openWindow: OpenWindowAction? = nil) {
        if let openWindow { openAction = openWindow }
        let existing = NSApp.windows.first { window in
            !(window is NSPanel)
                && window.identifier?.rawValue.hasPrefix(MenuBarMenu.mainWindowID) == true
        }
        if let window = existing {
            if window.isMiniaturized { window.deminiaturize(nil) }
            window.makeKeyAndOrderFront(nil)
        } else {
            openAction?(id: MenuBarMenu.mainWindowID)
        }
        NSApp.activate()
    }
}

/// "Check for Updates…" (app menu and menu bar): the result shows on the main window, so
/// it is opened first.
struct CheckForUpdatesButton: View {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Check for Updates…") {
            MainWindow.show(openWindow)
            Task { await model.checkForUpdate(manual: true) }
        }
        .disabled(model.isCheckingForUpdate || model.isInstallingUpdate)
    }
}
