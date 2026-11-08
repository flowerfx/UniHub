import UniHubCore
import SwiftUI

struct ReleasesView: View {
    @Environment(AppModel.self) private var model

    private let streams: [(ReleaseStream, String)] = [
        (.lts, "LTS"), (.supported, "Supported"), (.beta, "Beta"), (.alpha, "Alpha"), (.tech, "Tech"),
    ]

    var body: some View {
        @Bindable var model = model
        List {
            ForEach(model.releases) { release in
                ReleaseRow(release: release)
            }
            if UInt32(model.releases.count) < model.releaseTotal {
                HStack {
                    Spacer()
                    Button("Load More") { Task { await model.loadReleases(reset: false) } }
                        .disabled(model.isLoadingReleases)
                    Spacer()
                }
            }
        }
        .overlay {
            if model.isLoadingReleases && model.releases.isEmpty {
                ProgressView()
            }
        }
        .navigationTitle("Releases")
        .navigationSubtitle(model.releaseTotal > 0 ? "\(model.releaseTotal) for Apple Silicon" : "")
        .toolbar {
            ToolbarItem(placement: .principal) {
                Picker("Stream", selection: $model.stream) {
                    ForEach(streams, id: \.0) { Text($0.1).tag($0.0) }
                }
                .pickerStyle(.segmented)
            }
        }
        .task(id: model.stream) { await model.loadReleases(reset: true) }
    }
}

private struct ReleaseRow: View {
    @Environment(AppModel.self) private var model
    let release: Release

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(release.version).font(.headline).monospacedDigit()
                    if release.recommended {
                        Badge(text: "Recommended", color: .blue)
                    }
                    if model.isInstalled(release.version) {
                        Badge(text: "Installed", color: .green)
                    }
                }
                Text(details).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            if release.releaseNotesUrl != nil {
                Button("Release Notes") { model.openURL(release.releaseNotesUrl) }
                    .buttonStyle(.borderless)
            }
            // UniHub's own installer is not built yet; hand off to the official Hub.
            Button("Install in Unity Hub") { model.openURL(release.hubDeepLink) }
                .disabled(release.hubDeepLink == nil || model.isInstalled(release.version))
                .help("Opens this version in the official Unity Hub")
        }
        .padding(.vertical, 4)
    }

    private var details: String {
        var parts = [Format.day(iso: release.releaseDate)]
        if let editor = release.editor {
            parts.append("\(Format.bytes(editor.downloadSize)) download")
        }
        parts.append("\(release.modules.count) modules")
        return parts.joined(separator: " · ")
    }
}

private struct Badge: View {
    let text: String
    let color: Color

    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 6)
            .padding(.vertical, 1)
            .foregroundStyle(color)
            .background(color.opacity(0.15), in: Capsule())
    }
}
