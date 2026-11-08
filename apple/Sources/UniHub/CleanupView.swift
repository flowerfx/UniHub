import SwiftUI
import UniHubCore

/// "Clean Up" sheet: lists what can be removed to free disk space, lets the user pick,
/// then deletes. In a git repository it offers the files git ignores; otherwise the
/// folders and files Unity and IDEs regenerate (Library, Temp, Logs, .sln/.csproj…).
struct CleanupView: View {
    // Passed explicitly: sheets do not reliably inherit the environment.
    let model: AppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            if let cleanup = model.cleanup {
                header(cleanup)
                Divider()
                content(cleanup)
                Divider()
                footer(cleanup)
            }
        }
        // Fits the main window's minimum height (520).
        .frame(width: 620, height: 470)
    }

    // MARK: Parts

    private func header(_ cleanup: Cleanup) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "sparkles")
                .font(.system(size: 26))
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 4) {
                Text("Clean Up “\(cleanup.project.title)”").font(.title3.weight(.semibold))
                Text(explanation(cleanup.plan?.method))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(16)
    }

    @ViewBuilder
    private func content(_ cleanup: Cleanup) -> some View {
        if let result = cleanup.result {
            ResultView(result: result)
        } else if cleanup.isMeasuring {
            VStack(spacing: 10) {
                ProgressView()
                Text("Measuring…").foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let plan = cleanup.plan, plan.items.isEmpty {
            ContentUnavailableView(
                "Nothing to clean",
                systemImage: "checkmark.circle",
                description: Text(plan.method == .gitIgnored
                                  ? "This repository ignores no files in the project folder."
                                  : "No Library, Temp, Logs or generated project files found.")
            )
        } else if let plan = cleanup.plan {
            VStack(spacing: 0) {
                if plan.openInEditor {
                    Banner(
                        text: "This project is open in Unity. Close it there before cleaning up.",
                        action: ("Check Again", { Task { await model.loadCleanupPlan() } })
                    )
                }
                ItemList(model: model, plan: plan, selected: cleanup.selected)
            }
        } else if let error = cleanup.error {
            ContentUnavailableView("Could not read the project", systemImage: "exclamationmark.triangle",
                                   description: Text(error))
        }
    }

    private func footer(_ cleanup: Cleanup) -> some View {
        HStack(spacing: 10) {
            if cleanup.result == nil, let plan = cleanup.plan, !plan.items.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    Text("\(Format.bytes(cleanup.selectedBytes)) selected")
                        .fontWeight(.semibold)
                        .monospacedDigit()
                    Text("\(cleanup.selected.count) of \(plan.items.count) items · deleted for good, not moved to the Trash")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            if let error = cleanup.error, cleanup.plan != nil {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                    .font(.callout)
                    .lineLimit(2)
            }
            Spacer()
            if cleanup.result != nil {
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            } else {
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                    .disabled(cleanup.isDeleting)
                if cleanup.isDeleting {
                    ProgressView().controlSize(.small)
                }
                // No Return shortcut: deleting should take a deliberate click.
                Button("Clean Up", role: .destructive) {
                    Task { await model.performCleanup() }
                }
                .disabled(cleanup.plan == nil || cleanup.selected.isEmpty || cleanup.isDeleting
                          || cleanup.plan?.openInEditor == true)
            }
        }
        .padding(16)
    }

    private func explanation(_ method: CleanMethod?) -> String {
        switch method {
        case .gitIgnored:
            "Removes untracked files this git repository ignores. Tracked files and new files you have not committed are never touched. Unity rebuilds the Library folder the next time the project opens, which can take a while."
        case .unityGenerated:
            "No git repository: removes only folders and files Unity or your IDE recreates (Library, Temp, obj, Logs, .sln and .csproj files, debug symbols). Unity rebuilds the Library folder the next time the project opens, which can take a while."
        case nil:
            "Frees disk space by removing files Unity and your IDE recreate."
        }
    }
}

// MARK: - Item list

private struct ItemList: View {
    let model: AppModel
    let plan: CleanPlan
    let selected: Set<String>

    var body: some View {
        let regenerated = plan.items.filter { $0.kind != .other }
        let other = plan.items.filter { $0.kind == .other }
        List {
            if !regenerated.isEmpty {
                Section {
                    ForEach(regenerated, id: \.path) { row($0) }
                } header: {
                    header("Recreated by Unity or your IDE", items: regenerated)
                }
            }
            if !other.isEmpty {
                Section {
                    ForEach(other, id: \.path) { row($0) }
                } header: {
                    header("Ignored by git, not recreated", items: other)
                } footer: {
                    Text("Builds, local settings and assets kept out of the repository. Select them only if you no longer need them.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .listStyle(.inset)
    }

    private func row(_ item: CleanItem) -> some View {
        Toggle(isOn: Binding(
            get: { selected.contains(item.path) },
            set: { on in
                if on { model.cleanup?.selected.insert(item.path) } else { model.cleanup?.selected.remove(item.path) }
            }
        )) {
            HStack(spacing: 8) {
                Image(systemName: item.isDir ? "folder" : "doc")
                    .foregroundStyle(.secondary)
                    .frame(width: 16)
                VStack(alignment: .leading, spacing: 1) {
                    Text(item.path + (item.isDir ? "/" : ""))
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Label(item.kind.title, systemImage: item.kind.symbol)
                        .font(.caption)
                        .foregroundStyle(item.kind == .other ? AnyShapeStyle(.orange) : AnyShapeStyle(.secondary))
                }
                Spacer()
                Text(Format.bytes(item.bytes))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
        }
        .toggleStyle(.checkbox)
        .help(item.path)
    }

    private func header(_ title: String, items: [CleanItem]) -> some View {
        let all = items.allSatisfy { selected.contains($0.path) }
        return HStack {
            Text(title)
            Spacer()
            Text(Format.bytes(items.reduce(0) { $0 + $1.bytes })).monospacedDigit()
            Button(all ? "Deselect All" : "Select All") {
                for item in items {
                    if all { model.cleanup?.selected.remove(item.path) } else { model.cleanup?.selected.insert(item.path) }
                }
            }
            .buttonStyle(.link)
            .font(.caption)
        }
    }
}

// MARK: - Pieces

private struct ResultView: View {
    let result: CleanResult

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: result.failed.isEmpty ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                .font(.system(size: 40))
                .foregroundStyle(result.failed.isEmpty ? Color.green : Color.orange)
            Text("Freed \(Format.bytes(result.freedBytes))").font(.title2.weight(.semibold))
            Text("Removed \(result.removed) \(result.removed == 1 ? "item" : "items").")
                .foregroundStyle(.secondary)
            if !result.failed.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Could not remove:").font(.callout.weight(.semibold))
                    ScrollView {
                        VStack(alignment: .leading, spacing: 2) {
                            ForEach(result.failed, id: \.self) { Text($0).font(.caption).textSelection(.enabled) }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(maxHeight: 160)
                }
                .padding(12)
                .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8))
                .padding(.horizontal, 24)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct Banner: View {
    let text: String
    let action: (String, () -> Void)

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
            Text(text).font(.callout)
            Spacer()
            Button(action.0, action: action.1).controlSize(.small)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.orange.opacity(0.12))
    }
}
