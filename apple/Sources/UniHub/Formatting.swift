import Foundation
import UniHubCore

extension Project: Identifiable {
    public var id: String { path }
}

extension Editor: Identifiable {
    public var id: String { appPath }
}

extension Release: Identifiable {
    public var id: String { version }

    /// Unity's name for the release line: "Unity 6.3 LTS", "Unity 6.6", "Unity 2022.3 LTS".
    var lineName: String {
        let parts = version.split(separator: ".")
        guard parts.count >= 2, let major = Int(parts[0]) else { return "Unity \(version)" }
        let name = "Unity \(major >= 6000 ? major / 1000 : major).\(parts[1])"
        return stream == "LTS" ? name + " LTS" : name
    }

    /// "Unity 6", "Unity 2022".
    var majorName: String {
        let major = Int(version.split(separator: ".").first ?? "") ?? 0
        return "Unity \(major >= 6000 ? major / 1000 : major)"
    }

    /// "LTS", "Supported", "Beta", …
    var streamName: String {
        stream == "LTS" ? "LTS" : stream.capitalized
    }
}

/// Sort key that orders Unity versions numerically (`6000.0.9f1` < `6000.0.85f1`).
struct VersionKey: Comparable {
    let raw: String

    static func < (lhs: VersionKey, rhs: VersionKey) -> Bool {
        lhs.raw.compare(rhs.raw, options: .numeric) == .orderedAscending
    }
}

extension Project {
    var versionKey: VersionKey { VersionKey(raw: editorVersion ?? "") }
    var modifiedKey: Int64 { modifiedMs ?? 0 }
    var favoriteKey: Int { isFavorite ? 1 : 0 }
    var branchKey: String { sourceControl?.branch ?? "" }
}

extension VcsProvider {
    var displayName: String {
        switch self {
        case .gitHub: "GitHub"
        case .gitLab: "GitLab"
        case .git: "Git"
        case .unityVersionControl: "Unity Version Control"
        case .perforce: "Perforce"
        }
    }

    var symbol: String {
        switch self {
        case .gitHub, .gitLab, .git: "arrow.triangle.branch"
        case .unityVersionControl: "point.3.connected.trianglepath.dotted"
        case .perforce: "server.rack"
        }
    }

    /// "Pull requests" on GitHub, "Merge requests" on GitLab.
    var reviewsLabel: String { self == .gitLab ? "Merge Requests" : "Pull Requests" }
}

extension SourceControl {
    var repositoryName: String? {
        switch (owner, repository) {
        case let (o?, r?): "\(o)/\(r)"
        case let (nil, r?): r
        default: nil
        }
    }
}

extension UniHubError {
    var message: String {
        switch self {
        case .Io(let message): "File error: \(message)"
        case .InvalidData(let message): "Unexpected data: \(message)"
        case .NotAProject(let path): "This folder is not a Unity project:\n\(path)"
        case .UnknownProject(let path): "This project is not in the list:\n\(path)"
        case .EditorNotInstalled(let version): "Unity \(version) is not installed."
        case .NoEditorVersion: "This project has no editor version. Use Open With to pick one."
        case .Network(let message): "Network error: \(message)"
        case .CliNotInstalled: "This needs the Unity CLI. Install it from the Tools page."
        case .CliFailed(let code, let message):
            code == "AUTH_ERROR" ? "You are not signed in, or your session expired.\n\(message)" : message
        case .ProjectInUse: "This project is open in Unity. Close it in Unity first."
        case .Cancelled: "Cancelled."
        }
    }
}

extension CleanKind {
    var title: String {
        switch self {
        case .cache: "Unity cache"
        case .logs: "Logs and crash dumps"
        case .ideFiles: "IDE and solution files"
        case .debugFiles: "Debug symbols"
        case .systemFiles: "Finder files"
        case .other: "Not regenerated"
        }
    }

    var symbol: String {
        switch self {
        case .cache: "shippingbox"
        case .logs: "doc.text"
        case .ideFiles: "chevron.left.forwardslash.chevron.right"
        case .debugFiles: "ladybug"
        case .systemFiles: "macwindow"
        case .other: "exclamationmark.triangle"
        }
    }
}

extension Plugin: Identifiable {}

extension Account: Identifiable {}

extension AuthStatus {
    var displayName: String { name ?? email ?? "Unity account" }
}

/// Up to two initials for an avatar, e.g. "Ada Lovelace" → "AL".
func initials(_ name: String?) -> String {
    let words = (name ?? "").split(whereSeparator: { $0 == " " || $0 == "." || $0 == "@" })
    let letters = words.prefix(2).compactMap(\.first).map { String($0).uppercased() }
    return letters.isEmpty ? "?" : letters.joined()
}

func describe(_ error: Error) -> String {
    (error as? UniHubError)?.message ?? error.localizedDescription
}

enum Format {
    static func bytes(_ value: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(value), countStyle: .file)
    }

    static func relative(ms: Int64?) -> String {
        guard let ms, ms > 0 else { return "—" }
        let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
        return date.formatted(.relative(presentation: .named))
    }

    static func relative(iso: String?) -> String {
        guard let iso, let date = ISO8601DateFormatter().date(from: iso) else { return "" }
        return date.formatted(.relative(presentation: .named))
    }

    static func day(iso: String) -> String {
        let parser = ISO8601DateFormatter()
        parser.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let date = parser.date(from: iso) else { return String(iso.prefix(10)) }
        return date.formatted(date: .abbreviated, time: .omitted)
    }
}
