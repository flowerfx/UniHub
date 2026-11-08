import SwiftUI
import UniHubCore

/// Offers a newer UniHub release: notes, then download, verify, install and relaunch.
struct UpdateSheet: View {
    let model: AppModel
    let update: AppUpdate
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: 12) {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: 56, height: 56)
                VStack(alignment: .leading, spacing: 4) {
                    Text("UniHub \(update.version) is available").font(.title3.weight(.semibold))
                    Text("You have \(update.currentVersion). \(sizeText)")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            .padding(16)
            Divider()
            ScrollView {
                Text(notes)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
            }
            .frame(maxHeight: .infinity)
            Divider()
            if let error = model.updateError {
                Label("The update could not be installed: \(error)", systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                    .font(.callout)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
                    .padding(.top, 12)
            }
            HStack {
                if let url = URL(string: update.pageUrl), !update.pageUrl.isEmpty {
                    Link("View on GitHub", destination: url)
                }
                Spacer()
                if model.isInstallingUpdate {
                    ProgressView().controlSize(.small)
                    Text("Downloading and installing…").foregroundStyle(.secondary)
                }
                Button("Later") {
                    model.updateError = nil
                    dismiss()
                }
                    .keyboardShortcut(.cancelAction)
                    .disabled(model.isInstallingUpdate)
                Button(model.updateError == nil ? "Install and Relaunch" : "Try Again") {
                    Task { await model.installUpdate(update) }
                }
                    .keyboardShortcut(.defaultAction)
                    .disabled(model.isInstallingUpdate || update.checksumUrl == nil)
                    .help(update.checksumUrl == nil
                          ? "This release has no checksum file, so UniHub cannot verify it. Download it from GitHub."
                          : "Download, verify and install, then reopen UniHub")
            }
            .padding(16)
        }
        .frame(width: 520, height: 420)
        .interactiveDismissDisabled(model.isInstallingUpdate)
    }

    private var sizeText: String {
        update.dmgSize > 0 ? "Download \(Format.bytes(update.dmgSize))." : ""
    }

    private var notes: AttributedString {
        let text = update.notes.isEmpty ? "No release notes." : update.notes
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }
}
