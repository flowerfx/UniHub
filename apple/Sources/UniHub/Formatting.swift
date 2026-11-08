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
        case .Cancelled: "Cancelled."
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

    static func day(iso: String) -> String {
        let parser = ISO8601DateFormatter()
        parser.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let date = parser.date(from: iso) else { return String(iso.prefix(10)) }
        return date.formatted(date: .abbreviated, time: .omitted)
    }
}
