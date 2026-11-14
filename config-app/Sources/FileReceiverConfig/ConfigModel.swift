import AppKit
import Foundation
import Combine

enum TokenStatus: Equatable {
    case unknown
    case checking
    case valid(username: String)
    case invalid(String)
}

@MainActor
final class ConfigModel: ObservableObject {
    private static let projectKey = "projectDirectory"

    @Published var projectDir: URL {
        didSet {
            UserDefaults.standard.set(projectDir.path, forKey: Self.projectKey)
            BotRunner.shared.configure(projectDir: projectDir)
            HistoryStore.shared.configure(projectDir: projectDir)
        }
    }

    @Published var token = "" { didSet { if token != oldValue { tokenStatus = .unknown; markDirty() } } }
    @Published var apiId = "" { didSet { markDirty() } }
    @Published var apiHash = "" { didSet { markDirty() } }
    @Published var allowedUsers: [String] = [] { didSet { markDirty() } }
    @Published var saveDir = "files" { didSet { markDirty() } }

    // Файлы
    @Published var sortByType = true { didSet { markDirty() } }
    @Published var sortByDate = false { didSet { markDirty() } }
    @Published var groupFolders = true { didSet { markDirty() } }
    @Published var captionAsName = true { didSet { markDirty() } }
    @Published var unzip = false { didSet { markDirty() } }
    @Published var dedup = true { didSet { markDirty() } }
    @Published var minFreeGB: Double = 5 { didSet { markDirty() } }
    @Published var backupDir = "" { didSet { markDirty() } }

    // Распознавание
    @Published var transcribe = true { didSet { markDirty() } }
    @Published var whisperModel = "models/whisper-small" { didSet { markDirty() } }
    @Published var whisperLang = "auto" { didSet { markDirty() } }
    @Published var whisperGPU = true { didSet { markDirty() } }
    @Published var whisperUnloadMin: Double = 10 { didSet { markDirty() } }
    @Published var ocr = true { didSet { markDirty() } }
    @Published var saveTextFiles = true { didSet { markDirty() } }
    @Published var replyText = true { didSet { markDirty() } }

    // Прочее
    @Published var notify = true { didSet { markDirty() } }
    @Published var logMaxMB: Double = 10 { didSet { markDirty() } }

    @Published var tokenStatus: TokenStatus = .unknown
    @Published var isDirty = false
    @Published var message: String?
    @Published var messageIsError = false

    private var env = EnvFile()
    private var loading = false

    init() {
        let stored = UserDefaults.standard.string(forKey: Self.projectKey)
        projectDir = URL(fileURLWithPath: stored ?? NSString(string: "~/projects/file_receiver").expandingTildeInPath)
        load()
        BotRunner.shared.configure(projectDir: projectDir)
        HistoryStore.shared.configure(projectDir: projectDir)
    }

    var envURL: URL { projectDir.appendingPathComponent(".env") }

    /// Папка для файлов с учётом того, что относительный путь считается от папки бота
    var resolvedSaveDir: URL {
        let expanded = NSString(string: saveDir).expandingTildeInPath
        return expanded.hasPrefix("/")
            ? URL(fileURLWithPath: expanded)
            : projectDir.appendingPathComponent(expanded)
    }

    var projectLooksValid: Bool {
        FileManager.default.fileExists(atPath: projectDir.appendingPathComponent("Cargo.toml").path)
    }

    private func markDirty() {
        if !loading { isDirty = true }
    }

    func load() {
        loading = true
        defer { loading = false }

        let example = projectDir.appendingPathComponent(".env.example")
        if let e = EnvFile.load(from: envURL) {
            env = e
            show("Загружено: \(envURL.path)")
        } else if let e = EnvFile.load(from: example) {
            env = e
            show("Файл .env ещё не создан — взяты значения из .env.example")
        } else {
            env = EnvFile()
            show("Файл .env не найден, будет создан при сохранении")
        }

        token = env.value("TELOXIDE_TOKEN") ?? ""
        // Плейсхолдер из примера не считаем настоящим токеном
        if token.hasPrefix("123456789:AAAA") { token = "" }
        apiId = env.value("API_ID") ?? ""
        apiHash = env.value("API_HASH") ?? ""
        allowedUsers = (env.value("ALLOWED_USERS") ?? "")
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        saveDir = env.value("SAVE_DIR") ?? "files"

        sortByType = flag("SORT_BY_TYPE", true)
        sortByDate = flag("SORT_BY_DATE", false)
        groupFolders = flag("GROUP_FOLDERS", true)
        captionAsName = flag("CAPTION_AS_NAME", true)
        unzip = flag("UNZIP", false)
        dedup = flag("DEDUP", true)
        minFreeGB = number("MIN_FREE_GB", 5)
        backupDir = env.value("BACKUP_DIR") ?? ""
        transcribe = flag("TRANSCRIBE", true)
        whisperModel = nonEmpty(env.value("WHISPER_MODEL")) ?? "models/whisper-small"
        whisperLang = nonEmpty(env.value("WHISPER_LANG")) ?? "auto"
        whisperGPU = flag("WHISPER_GPU", true)
        whisperUnloadMin = number("WHISPER_UNLOAD_MIN", 10)
        ocr = flag("OCR", true)
        saveTextFiles = flag("SAVE_TEXT_FILES", true)
        replyText = flag("REPLY_TEXT", true)
        notify = flag("NOTIFY", true)
        logMaxMB = number("LOG_MAX_MB", 10)
        tokenStatus = .unknown
        isDirty = false
    }

    func save() {
        let trimmedToken = token.trimmingCharacters(in: .whitespacesAndNewlines)
        env.set("TELOXIDE_TOKEN", trimmedToken)
        env.set("API_ID", apiId.trimmingCharacters(in: .whitespaces))
        env.set("API_HASH", apiHash.trimmingCharacters(in: .whitespaces))
        env.set("ALLOWED_USERS", allowedUsers.joined(separator: ","))
        env.set("SAVE_DIR", saveDir.trimmingCharacters(in: .whitespaces))
        env.set("SORT_BY_TYPE", String(sortByType))
        env.set("SORT_BY_DATE", String(sortByDate))
        env.set("GROUP_FOLDERS", String(groupFolders))
        env.set("CAPTION_AS_NAME", String(captionAsName))
        env.set("UNZIP", String(unzip))
        env.set("DEDUP", String(dedup))
        env.set("MIN_FREE_GB", Self.format(minFreeGB))
        env.set("BACKUP_DIR", backupDir.trimmingCharacters(in: .whitespaces))
        env.set("TRANSCRIBE", String(transcribe))
        env.set("WHISPER_MODEL", whisperModel)
        env.set("WHISPER_LANG", whisperLang)
        env.set("WHISPER_GPU", String(whisperGPU))
        env.set("WHISPER_UNLOAD_MIN", Self.format(whisperUnloadMin))
        env.set("OCR", String(ocr))
        env.set("SAVE_TEXT_FILES", String(saveTextFiles))
        env.set("REPLY_TEXT", String(replyText))
        env.set("NOTIFY", String(notify))
        env.set("LOG_MAX_MB", Self.format(logMaxMB))
        do {
            try env.write(to: envURL)
            isDirty = false
            show("Сохранено. Перезапустите бота, чтобы изменения вступили в силу.")
        } catch {
            show("Не удалось сохранить: \(error.localizedDescription)", error: true)
        }
    }

    private func flag(_ key: String, _ def: Bool) -> Bool {
        switch env.value(key)?.lowercased() {
        case "1", "true", "yes", "on": true
        case "0", "false", "no", "off": false
        default: def
        }
    }

    private func number(_ key: String, _ def: Double) -> Double {
        env.value(key).flatMap { Double($0.replacingOccurrences(of: ",", with: ".")) } ?? def
    }

    private func nonEmpty(_ s: String?) -> String? {
        guard let s, !s.trimmingCharacters(in: .whitespaces).isEmpty else { return nil }
        return s
    }

    private static func format(_ v: Double) -> String {
        v == v.rounded() ? String(Int(v)) : String(v)
    }

    /// Путь относительно папки бота → полный путь
    func resolve(_ path: String) -> URL {
        let expanded = NSString(string: path).expandingTildeInPath
        return expanded.hasPrefix("/") ? URL(fileURLWithPath: expanded) : projectDir.appendingPathComponent(expanded)
    }

    static var iCloudDrive: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Mobile Documents/com~apple~CloudDocs")
    }

    func chooseBackupDir() {
        let start = backupDir.isEmpty ? Self.iCloudDrive : resolve(backupDir)
        guard let url = pickFolder(title: "Папка для резервных копий", start: start, canCreate: true) else { return }
        backupDir = url.path
    }

    func useICloudBackup() {
        backupDir = Self.iCloudDrive.appendingPathComponent("File Receiver").path
    }

    var apiIdIsValid: Bool {
        let t = apiId.trimmingCharacters(in: .whitespaces)
        return !t.isEmpty && t.allSatisfy(\.isNumber)
    }

    var apiHashIsValid: Bool {
        let t = apiHash.trimmingCharacters(in: .whitespaces)
        return t.count == 32 && t.allSatisfy(\.isHexDigit)
    }

    var apiConfigured: Bool { apiIdIsValid && apiHashIsValid }

    // MARK: - Доверенные аккаунты

    /// Возвращает текст ошибки или nil, если ID добавлен
    func addUser(_ raw: String) -> String? {
        let id = raw.trimmingCharacters(in: .whitespaces)
        guard !id.isEmpty else { return "Введите ID" }
        guard UInt64(id) != nil else { return "ID должен состоять только из цифр" }
        guard !allowedUsers.contains(id) else { return "Этот ID уже добавлен" }
        allowedUsers.append(id)
        return nil
    }

    func removeUser(_ id: String) {
        allowedUsers.removeAll { $0 == id }
    }

    // MARK: - Папки

    func chooseProjectDir() {
        guard let url = pickFolder(title: "Папка проекта file_receiver", start: projectDir) else { return }
        if isDirty { save() }
        projectDir = url
        load()
    }

    func chooseSaveDir() {
        guard let url = pickFolder(title: "Папка для сохранения файлов", start: resolvedSaveDir, canCreate: true)
        else { return }
        saveDir = url.path
    }

    func revealSaveDir() {
        let dir = resolvedSaveDir
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        NSWorkspace.shared.activateFileViewerSelecting([dir])
    }

    private func pickFolder(title: String, start: URL, canCreate: Bool = false) -> URL? {
        let panel = NSOpenPanel()
        panel.title = title
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = canCreate
        panel.allowsMultipleSelection = false
        panel.directoryURL = start
        return panel.runModal() == .OK ? panel.url : nil
    }

    // MARK: - Проверка токена

    func checkToken() async {
        let t = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else {
            tokenStatus = .invalid("Токен пустой")
            return
        }
        guard let url = URL(string: "https://api.telegram.org/bot\(t)/getMe") else {
            tokenStatus = .invalid("Некорректный формат токена")
            return
        }
        tokenStatus = .checking
        do {
            let (data, _) = try await URLSession.shared.data(from: url)
            let resp = try JSONDecoder().decode(GetMeResponse.self, from: data)
            if resp.ok, let u = resp.result?.username {
                tokenStatus = .valid(username: u)
            } else {
                tokenStatus = .invalid(resp.description ?? "Telegram отклонил токен")
            }
        } catch {
            tokenStatus = .invalid("Нет связи с Telegram: \(error.localizedDescription)")
        }
    }

    private func show(_ text: String, error: Bool = false) {
        message = text
        messageIsError = error
    }
}

private struct GetMeResponse: Decodable {
    struct User: Decodable { let username: String? }
    let ok: Bool
    let result: User?
    let description: String?
}
