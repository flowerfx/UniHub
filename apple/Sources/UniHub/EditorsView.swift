import UniHubCore
import SwiftUI

struct EditorsView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List(model.editors) { editor in
            HStack(spacing: 12) {
                Image(systemName: "cube.fill")
                    .font(.title2)
                    .foregroundStyle(.tint)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text("Unity \(editor.version)").font(.headline).monospacedDigit()
                        Text(editor.architecture)
                            .font(.caption)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 1)
                            .background(.quaternary, in: Capsule())
                    }
                    Text(editor.installDir)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer()
                Text(usage(of: editor)).foregroundStyle(.secondary)
                Button("Show in Finder", systemImage: "folder") {
                    model.revealInFinder(editor.appPath)
                }
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
            }
            .padding(.vertical, 4)
        }
        .navigationTitle("Installs")
        .navigationSubtitle("\(model.editors.count) editors")
        .overlay {
            if model.editors.isEmpty {
                ContentUnavailableView(
                    "No editors found",
                    systemImage: "shippingbox",
                    description: Text("UniHub looks in /Applications/Unity/Hub/Editor.")
                )
            }
        }
    }

    private func usage(of editor: Editor) -> String {
        let count = model.projects.filter { $0.editorVersion == editor.version }.count
        return count == 1 ? "1 project" : "\(count) projects"
    }
}
