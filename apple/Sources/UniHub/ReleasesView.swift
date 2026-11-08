import UniHubCore
import SwiftUI

/// Releases page: browse a stream (Apple Silicon builds), or search every stream by
/// version, including older versions that only have an Intel build. Installing goes
/// through the Unity CLI; Unity Hub is not involved.
struct ReleasesView: View {
    @Environment(AppModel.self) private var model

    private var isSearching: Bool {
        !model.releaseQuery.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        @Bindable var model = model
        List {
            if model.releaseTab == .recommended && !isSearching {
                // Like the Hub's "Official releases": grouped by major version.
                ForEach(majorGroups, id: \.name) { group in
                    Section(group.name) {
                        ForEach(group.releases) { release in
                            ReleaseRow(model: model, release: release, featured: true)
                        }
                    }
                }
            } else {
                ForEach(model.releases) { release in
                    ReleaseRow(model: model, release: release, featured: false)
                }
            }
            if UInt32(model.releases.count) < model.releaseTotal {
                HStack {
                    Spacer()
                    if model.isLoadingReleases {
                        ProgressView().controlSize(.small)
                    } else {
                        Button("Load More") { Task { await model.loadReleases(reset: false) } }
                    }
                    Spacer()
                }
            }
        }
        // A new list per stream or search, so it starts scrolled to the top.
        .id("\(model.releaseTab)|\(model.releaseQuery)")
        .overlay { emptyState }
        .navigationTitle("Releases")
        .navigationSubtitle(subtitle)
        .searchable(text: $model.releaseQuery, placement: .toolbar, prompt: "Find a version, e.g. 2022.3")
        .toolbar {
            ToolbarItem(placement: .principal) {
                Picker("Stream", selection: $model.releaseTab) {
                    ForEach(ReleaseTab.allCases) { Text($0.title).tag($0) }
                }
                .pickerStyle(.segmented)
                .disabled(isSearching)
                .help(isSearching ? "Searching every stream" : "Release stream")
            }
        }
        .task(id: "\(model.releaseTab)|\(model.releaseQuery)") {
            // Wait for typing to pause before searching.
            if isSearching { try? await Task.sleep(for: .milliseconds(350)) }
            guard !Task.isCancelled else { return }
            await model.loadReleases(reset: true)
        }
        .sheet(item: $model.installingRelease) { release in
            InstallEditorSheet(model: model, release: release)
        }
    }

    private var subtitle: String {
        if model.releases.isEmpty { return "" }
        if isSearching {
            return model.releaseTotal == 1 ? "1 version found" : "\(model.releaseTotal) versions found"
        }
        if model.releaseTab == .recommended {
            return "Latest of each supported version"
        }
        return "\(model.releaseTotal) for Apple Silicon"
    }

    private var majorGroups: [(name: String, releases: [Release])] {
        var groups: [(name: String, releases: [Release])] = []
        for release in model.releases {
            if let i = groups.firstIndex(where: { $0.name == release.majorName }) {
                groups[i].releases.append(release)
            } else {
                groups.append((release.majorName, [release]))
            }
        }
        return groups
    }

    @ViewBuilder
    private var emptyState: some View {
        if model.releases.isEmpty {
            if model.isLoadingReleases {
                ProgressView()
            } else if isSearching {
                ContentUnavailableView(
                    "No versions match “\(model.releaseQuery.trimmingCharacters(in: .whitespaces))”",
                    systemImage: "magnifyingglass",
                    description: Text("Search by the start of a version number, e.g. 2022.3, 2021.3.45 or 6000.0.")
                )
            } else {
                ContentUnavailableView {
                    Label(model.releaseTab == .recommended ? "No versions to show" : "No versions in this stream",
                          systemImage: "shippingbox")
                } description: {
                    if let error = model.releaseError {
                        Text("Unity's release list could not be loaded right now.\n\(error)")
                    } else {
                        Text("There are no releases for Apple Silicon in this stream right now.")
                    }
                } actions: {
                    if model.releaseError != nil {
                        Button("Try Again") { Task { await model.loadReleases(reset: true) } }
                    }
                }
            }
        }
    }
}

private struct ReleaseRow: View {
    let model: AppModel
    let release: Release
    /// Recommended tab: "Unity 6.3 LTS (6000.3.28f1)" with its stream badge.
    var featured = false

    var body: some View {
        let installed = model.editors.contains { $0.version == release.version && $0.architecture == release.architecture }
        let job = model.runningJob(version: release.version)
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    if featured {
                        Text(release.lineName).font(.headline)
                        Text("(\(release.version))").font(.headline).foregroundStyle(.secondary).monospacedDigit()
                        Badge(text: release.streamName, color: .secondary)
                    } else {
                        Text(release.version).font(.headline).monospacedDigit()
                    }
                    if release.recommended {
                        Badge(text: "Recommended", color: .blue)
                    }
                    if release.architecture == "x86_64" {
                        Badge(text: "Intel", color: .orange)
                            .help("No Apple Silicon build; the Intel build runs with Rosetta 2")
                    }
                    if installed {
                        Badge(text: "Installed", color: .green)
                    }
                }
                Text(details).font(.caption).foregroundStyle(.secondary)
                if let job {
                    HStack(spacing: 6) {
                        if let percent = job.percent {
                            ProgressView(value: percent, total: 100).frame(maxWidth: 220)
                        } else {
                            ProgressView().progressViewStyle(.linear).frame(maxWidth: 220)
                        }
                        Text(job.message).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
            }
            Spacer()
            if release.releaseNotesUrl != nil {
                Button("Release Notes") { model.openURL(release.releaseNotesUrl) }
                    .buttonStyle(.borderless)
            }
            if let job {
                Button("Cancel") { model.cancelJob(job) }
            } else {
                Button(installed ? "Installed" : "Install") { model.installingRelease = release }
                    .disabled(installed || release.editor == nil)
                    .help(installed ? "Already installed" : "Install this version with UniHub")
            }
        }
        .padding(.vertical, 4)
    }

    private var details: String {
        var parts = [Format.day(iso: release.releaseDate)]
        if !model.releaseQuery.trimmingCharacters(in: .whitespaces).isEmpty {
            parts.insert(release.streamName, at: 0)
        }
        if let editor = release.editor {
            parts.append("\(Format.bytes(editor.downloadSize)) download")
        }
        parts.append("\(release.modules.filter { !$0.hidden }.count) modules")
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
