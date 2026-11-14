import AppKit
import Combine
import Foundation

/// Запись истории (формат history.jsonl, который пишет бот)
struct HistoryEntry: Decodable, Identifiable, Hashable {
    let id: UInt64
    let time: String
    let user_id: Int64
    let chat: String?
    let kind: String
    let name: String
    let path: String
    let size: UInt64
    let text: String?
    let deleted: Bool?

    var url: URL { URL(fileURLWithPath: path) }
    var exists: Bool { deleted != true && FileManager.default.fileExists(atPath: path) }

    var date: Date? {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f.date(from: time) ?? ISO8601DateFormatter().date(from: time)
    }

    var dateText: String {
        guard let d = date else { return "" }
        return d.formatted(date: .abbreviated, time: .shortened)
    }

    var sizeText: String { ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file) }

    var symbol: String {
        switch kind {
        case "photo": "photo"
        case "video", "video_note": "film"
        case "audio": "music.note"
        case "voice": "waveform"
        case "archive": "doc.zipper"
        case "document": "doc.text"
        default: "doc"
        }
    }
}

@MainActor
final class HistoryStore: ObservableObject {
    static let shared = HistoryStore()

    @Published private(set) var entries: [HistoryEntry] = []
    private var projectDir: URL?
    private var lastModified: Date?
    private var watcher: FileWatcher?

    private init() {}

    func configure(projectDir: URL) {
        guard projectDir != self.projectDir else { return }
        self.projectDir = projectDir
        lastModified = nil
        reloadIfChanged()
        // История обновляется по событию изменения файла, без опроса
        watcher?.stop()
        if let url = fileURL {
            watcher = FileWatcher(url: url) { HistoryStore.shared.reloadIfChanged() }
        }
    }

    private var fileURL: URL? { projectDir?.appendingPathComponent("history.jsonl") }

    func reloadIfChanged() {
        guard let url = fileURL else { return }
        let mod = (try? FileManager.default.attributesOfItem(atPath: url.path))?[.modificationDate] as? Date
        guard mod != lastModified else { return }
        lastModified = mod
        guard let text = try? String(contentsOf: url, encoding: .utf8) else {
            entries = []
            return
        }
        let decoder = JSONDecoder()
        entries = text.split(separator: "\n").compactMap { line in
            try? decoder.decode(HistoryEntry.self, from: Data(line.utf8))
        }.reversed()
    }

    func recent(_ n: Int) -> [HistoryEntry] {
        Array(entries.filter { $0.deleted != true }.prefix(n))
    }

    func search(_ query: String) -> [HistoryEntry] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        let live = entries.filter { $0.deleted != true }
        guard !q.isEmpty else { return live }
        return live.filter {
            $0.name.lowercased().contains(q) || ($0.text?.lowercased().contains(q) ?? false)
                || ($0.chat?.lowercased().contains(q) ?? false)
        }
    }

    static func reveal(_ e: HistoryEntry) {
        if e.exists {
            NSWorkspace.shared.activateFileViewerSelecting([e.url])
        }
    }

    static func open(_ e: HistoryEntry) {
        if e.exists {
            NSWorkspace.shared.open(e.url)
        }
    }
}
