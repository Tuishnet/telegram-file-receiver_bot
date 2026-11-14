import Foundation

/// Простой редактор .env: меняет нужные ключи, сохраняя комментарии и остальные строки.
struct EnvFile {
    private(set) var lines: [String] = []

    init(contents: String = "") {
        lines = contents.isEmpty ? [] : contents.components(separatedBy: "\n")
        // Убираем пустой хвост от финального перевода строки
        while lines.last == "" { lines.removeLast() }
    }

    static func load(from url: URL) -> EnvFile? {
        guard let text = try? String(contentsOf: url, encoding: .utf8) else { return nil }
        return EnvFile(contents: text)
    }

    private func lineIndex(of key: String) -> Int? {
        lines.firstIndex { line in
            let t = line.trimmingCharacters(in: .whitespaces)
            guard !t.hasPrefix("#"), let eq = t.firstIndex(of: "=") else { return false }
            return t[..<eq].trimmingCharacters(in: .whitespaces) == key
        }
    }

    func value(_ key: String) -> String? {
        guard let i = lineIndex(of: key), let eq = lines[i].firstIndex(of: "=") else { return nil }
        var v = lines[i][lines[i].index(after: eq)...].trimmingCharacters(in: .whitespaces)
        if v.count >= 2, let f = v.first, f == v.last, f == "\"" || f == "'" {
            v = String(v.dropFirst().dropLast())
        }
        return v
    }

    mutating func set(_ key: String, _ value: String) {
        let needsQuotes = value.contains(" ") || value.contains("#")
        let escaped = value.replacingOccurrences(of: "\"", with: "\\\"")
        let line = needsQuotes ? "\(key)=\"\(escaped)\"" : "\(key)=\(value)"
        if let i = lineIndex(of: key) {
            lines[i] = line
        } else {
            lines.append(line)
        }
    }

    func write(to url: URL) throws {
        let text = lines.joined(separator: "\n") + "\n"
        try text.write(to: url, atomically: true, encoding: .utf8)
        // В файле токен — доступ только владельцу
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }
}
