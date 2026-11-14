import AppKit
import Combine
import Foundation

/// Управляет ботом как фоновой службой macOS (LaunchAgent).
/// Бот работает независимо от приложения и запускается при входе в систему.
@MainActor
final class BotRunner: ObservableObject {
    static let shared = BotRunner()

    static let label = "local.file-receiver.bot"
    private static let binaryName = "file_receiver"
    private static let maxLogLines = 1000
    private static let autostartKey = "botAutostart"

    enum Status: Equatable {
        case stopped
        case building
        case running(pid: Int32)
        /// Служба загружена, но процесс сейчас не работает (например, перезапускается после сбоя)
        case waiting
        /// Бот запущен не службой (например, из терминала)
        case external(pid: Int32)
        case failed(String)
    }

    struct LogLine: Identifiable {
        let id = UUID()
        let text: String
        let kind: Kind
        enum Kind { case info, warn, error, system }
    }

    @Published private(set) var status: Status = .stopped
    @Published private(set) var log: [LogLine] = []
    @Published var autostart: Bool {
        didSet {
            UserDefaults.standard.set(autostart, forKey: Self.autostartKey)
            if FileManager.default.fileExists(atPath: plistURL.path) {
                writePlist()
                appendSystem(autostart ? "Автозапуск при входе в систему включён"
                                       : "Автозапуск при входе в систему выключен")
            }
        }
    }

    private(set) var projectDir: URL = URL(fileURLWithPath: NSHomeDirectory())
    private var timer: Timer?
    private var logWatcher: FileWatcher?
    private var lastStatusCheck = Date.distantPast
    private var logOffset: UInt64 = 0
    private var logPartial = ""

    private init() {
        autostart = UserDefaults.standard.object(forKey: Self.autostartKey) as? Bool ?? true
        // Статус службы: каждые 3 с, пока приложение на переднем плане, и раз в 30 с в фоне.
        // Лог читается по событиям файловой системы, а не опросом.
        timer = Timer.scheduledTimer(withTimeInterval: 3, repeats: true) { _ in
            Task { @MainActor in
                let me = BotRunner.shared
                let interval: TimeInterval = NSApplication.shared.isActive ? 3 : 30
                if Date().timeIntervalSince(me.lastStatusCheck) >= interval - 0.5 {
                    me.refreshStatus()
                }
            }
        }
    }

    // MARK: - Пути

    private var uid: String { String(getuid()) }
    private var service: String { "gui/\(uid)/\(Self.label)" }

    var plistURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/LaunchAgents/\(Self.label).plist")
    }

    var logURL: URL { projectDir.appendingPathComponent("bot.log") }
    private var binaryURL: URL { projectDir.appendingPathComponent("target/release/\(Self.binaryName)") }
    private var bundleExecURL: URL {
        projectDir.appendingPathComponent("FileReceiverBot.app/Contents/MacOS/\(Self.binaryName)")
    }

    /// Бот запускается из своего .app (нужно для уведомлений); если упаковать не вышло — напрямую
    private var launchURL: URL {
        FileManager.default.isExecutableFile(atPath: bundleExecURL.path) ? bundleExecURL : binaryURL
    }

    /// Упаковывает свежесобранного бота в FileReceiverBot.app (скрипт bundle-bot.sh)
    private func prepareBundle() {
        let fm = FileManager.default
        let mod = { (u: URL) in (try? fm.attributesOfItem(atPath: u.path))?[.modificationDate] as? Date }
        if let bin = mod(binaryURL), let packed = mod(bundleExecURL), packed >= bin { return }
        let script = projectDir.appendingPathComponent("bundle-bot.sh")
        guard fm.fileExists(atPath: script.path) else { return }
        let r = Self.run("/bin/bash", [script.path])
        if r.code == 0 {
            appendSystem("Бот упакован в FileReceiverBot.app")
        } else {
            appendSystem("Не удалось упаковать бота: \(r.output)", kind: .warn)
        }
    }

    var isRunning: Bool {
        switch status {
        case .running, .external, .waiting: true
        default: false
        }
    }

    var isBusy: Bool { status == .building }

    /// Вызывается при старте и при смене папки проекта
    func configure(projectDir: URL) {
        let real = projectDir.resolvingSymlinksInPath()
        guard real != self.projectDir else { return }
        self.projectDir = real
        log.removeAll()
        logOffset = 0
        logPartial = ""
        loadLogTail()
        logWatcher?.stop()
        logWatcher = FileWatcher(url: logURL) { BotRunner.shared.readLog() }
        refreshStatus()
    }

    // MARK: - Управление

    func start(projectDir: URL) {
        configure(projectDir: projectDir)
        guard !isBusy else { return }
        if case .external(let pid) = status {
            appendSystem("Останавливаю бота, запущенного вне службы (PID \(pid))")
            kill(pid, SIGINT)
        }
        buildIfNeeded { [weak self] ok in
            guard ok, let self else { return }
            self.prepareBundle()
            self.writePlist()
            if !self.isLoaded() {
                let r = Self.launchctl(["bootstrap", "gui/\(self.uid)", self.plistURL.path])
                if r.code != 0 && !self.isLoaded() {
                    self.status = .failed("Не удалось запустить службу")
                    self.appendSystem("launchctl bootstrap: \(r.output)", kind: .error)
                    return
                }
            }
            _ = Self.launchctl(["kickstart", self.service])
            self.appendSystem("Бот запущен как фоновая служба")
            self.refreshStatus()
        }
    }

    /// Останавливает бота. Если автозапуск включён, он снова стартует при следующем входе в систему.
    func stop() {
        switch status {
        case .external(let pid):
            appendSystem("Останавливаю бота (PID \(pid))…")
            kill(pid, SIGINT)
        default:
            if isLoaded() {
                appendSystem("Останавливаю службу бота…")
                let r = Self.launchctl(["bootout", service])
                if r.code != 0 && isLoaded() {
                    appendSystem("launchctl bootout: \(r.output)", kind: .error)
                }
            }
        }
        refreshStatus()
    }

    func restart(projectDir: URL) {
        configure(projectDir: projectDir)
        guard isLoaded() else {
            start(projectDir: projectDir)
            return
        }
        buildIfNeeded { [weak self] ok in
            guard ok, let self else { return }
            self.prepareBundle()
            self.writePlist()
            self.appendSystem("Перезапуск бота…")
            // bootout + bootstrap, чтобы launchd перечитал описание службы
            _ = Self.launchctl(["bootout", self.service])
            for _ in 0..<50 where self.isLoaded() { usleep(100_000) }
            let r = Self.launchctl(["bootstrap", "gui/\(self.uid)", self.plistURL.path])
            if r.code != 0 && !self.isLoaded() {
                self.status = .failed("Не удалось запустить службу")
                self.appendSystem("launchctl bootstrap: \(r.output)", kind: .error)
                return
            }
            _ = Self.launchctl(["kickstart", self.service])
            self.refreshStatus()
        }
    }

    func clearLog() {
        log.removeAll()
        // Обрезаем файл лога, чтобы он не рос бесконечно
        if let h = try? FileHandle(forWritingTo: logURL) {
            try? h.truncate(atOffset: 0)
            try? h.close()
        }
        logOffset = 0
        logPartial = ""
    }

    // MARK: - LaunchAgent

    private func writePlist() {
        let dict: [String: Any] = [
            "Label": Self.label,
            "ProgramArguments": [launchURL.path],
            "WorkingDirectory": projectDir.path,
            "RunAtLoad": autostart,
            // Перезапускать, если бот упал (но не после штатной остановки)
            "KeepAlive": ["SuccessfulExit": false],
            "ThrottleInterval": 10,
            // Бот сам пишет bot.log (с ротацией); сюда попадают только аварийные сообщения
            "StandardOutPath": projectDir.appendingPathComponent("bot-stderr.log").path,
            "StandardErrorPath": projectDir.appendingPathComponent("bot-stderr.log").path,
            "EnvironmentVariables": ["RUST_LOG": "info"],
            "ProcessType": "Background",
        ]
        do {
            try FileManager.default.createDirectory(
                at: plistURL.deletingLastPathComponent(), withIntermediateDirectories: true)
            let data = try PropertyListSerialization.data(fromPropertyList: dict, format: .xml, options: 0)
            try data.write(to: plistURL, options: .atomic)
        } catch {
            appendSystem("Не удалось записать \(plistURL.path): \(error.localizedDescription)", kind: .error)
        }
    }

    private func isLoaded() -> Bool {
        Self.launchctl(["print", service]).code == 0
    }

    func refreshStatus() {
        lastStatusCheck = Date()
        if isBusy { return }
        let r = Self.launchctl(["print", service])
        let new: Status
        if r.code == 0 {
            if let pid = Self.parsePid(r.output) {
                new = .running(pid: pid)
            } else {
                new = .waiting
            }
        } else if let pid = Self.findPid() {
            new = .external(pid: pid)
        } else if case .failed = status {
            new = status
        } else {
            new = .stopped
        }
        if new != status {
            if case .running = status, new == .waiting {
                appendSystem("Бот завершился, служба перезапустит его", kind: .warn)
            }
            status = new
        }
    }

    private static func parsePid(_ text: String) -> Int32? {
        for line in text.split(separator: "\n") {
            let t = line.trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("pid = "), let pid = Int32(t.dropFirst(6)) { return pid }
        }
        return nil
    }

    private static func findPid() -> Int32? {
        let r = run("/usr/bin/pgrep", ["-x", binaryName])
        return r.output.split(separator: "\n").compactMap { Int32($0) }.first
    }

    @discardableResult
    private static func launchctl(_ args: [String]) -> (code: Int32, output: String) {
        run("/bin/launchctl", args)
    }

    private static func run(_ path: String, _ args: [String]) -> (code: Int32, output: String) {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: path)
        p.arguments = args
        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = pipe
        do { try p.run() } catch { return (-1, error.localizedDescription) }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        return (p.terminationStatus, String(decoding: data, as: UTF8.self))
    }

    // MARK: - Сборка

    private func buildIfNeeded(_ completion: @escaping (Bool) -> Void) {
        guard needsBuild() else {
            completion(true)
            return
        }
        guard let cargo = Self.findCargo() else {
            status = .failed("Не найден cargo")
            appendSystem("Не найден cargo. Установите Rust: https://rustup.rs", kind: .error)
            completion(false)
            return
        }
        status = .building
        appendSystem("Собираю бота (cargo build --release)… Первый раз это может занять пару минут.")

        let p = Process()
        p.executableURL = cargo
        p.arguments = ["build", "--release"]
        p.currentDirectoryURL = projectDir
        var env = ProcessInfo.processInfo.environment
        let home = NSHomeDirectory()
        env["PATH"] = ["\(home)/.cargo/bin", "/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin",
                       env["PATH"] ?? ""].joined(separator: ":")
        p.environment = env

        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = pipe
        let buffer = LineBuffer()
        pipe.fileHandleForReading.readabilityHandler = { h in
            let chunk = h.availableData
            if chunk.isEmpty { h.readabilityHandler = nil; return }
            let lines = buffer.feed(chunk)
            guard !lines.isEmpty else { return }
            Task { @MainActor in for l in lines { BotRunner.shared.appendOutput(l) } }
        }
        p.terminationHandler = { proc in
            let code = proc.terminationStatus
            Task { @MainActor in
                let me = BotRunner.shared
                if code == 0 {
                    me.appendSystem("Сборка завершена")
                    me.status = .stopped
                    completion(true)
                } else {
                    me.appendSystem("Сборка не удалась (код \(code))", kind: .error)
                    me.status = .failed("Ошибка сборки")
                    completion(false)
                }
            }
        }
        do {
            try p.run()
        } catch {
            status = .failed("Не удалось запустить cargo")
            appendSystem("Не удалось запустить cargo: \(error.localizedDescription)", kind: .error)
            completion(false)
        }
    }

    private func needsBuild() -> Bool {
        let fm = FileManager.default
        guard let binDate = (try? fm.attributesOfItem(atPath: binaryURL.path))?[.modificationDate] as? Date
        else { return true }
        var files = ["Cargo.toml", "Cargo.lock"].map { projectDir.appendingPathComponent($0) }
        if let e = fm.enumerator(at: projectDir.appendingPathComponent("src"), includingPropertiesForKeys: nil) {
            for case let url as URL in e where url.pathExtension == "rs" { files.append(url) }
        }
        return files.contains { url in
            ((try? fm.attributesOfItem(atPath: url.path))?[.modificationDate] as? Date ?? .distantPast) > binDate
        }
    }

    private static func findCargo() -> URL? {
        let home = NSHomeDirectory()
        return ["\(home)/.cargo/bin/cargo", "/opt/homebrew/bin/cargo", "/usr/local/bin/cargo"]
            .first { FileManager.default.isExecutableFile(atPath: $0) }
            .map { URL(fileURLWithPath: $0) }
    }

    // MARK: - Лог (читается из bot.log)

    private func loadLogTail() {
        guard let data = try? Data(contentsOf: logURL) else { return }
        logOffset = UInt64(data.count)
        let text = String(decoding: data, as: UTF8.self)
        for l in text.split(separator: "\n").suffix(200) { appendOutput(String(l)) }
    }

    private func readLog() {
        guard let h = try? FileHandle(forReadingFrom: logURL) else { return }
        defer { try? h.close() }
        let size = (try? h.seekToEnd()) ?? 0
        if size < logOffset { logOffset = 0; logPartial = "" } // файл обрезали
        guard size > logOffset else { return }
        try? h.seek(toOffset: logOffset)
        let data = h.readData(ofLength: Int(size - logOffset))
        logOffset = size
        var text = logPartial + String(decoding: data, as: UTF8.self)
        if let lastNL = text.lastIndex(of: "\n") {
            logPartial = String(text[text.index(after: lastNL)...])
            text = String(text[..<lastNL])
        } else {
            logPartial = text
            return
        }
        for l in text.split(separator: "\n", omittingEmptySubsequences: false) { appendOutput(String(l)) }
    }

    private func appendOutput(_ raw: String) {
        let text = raw.replacingOccurrences(of: "\u{1B}\\[[0-9;]*[A-Za-z]", with: "", options: .regularExpression)
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        let kind: LogLine.Kind
        if text.contains("ERROR") || text.hasPrefix("error") { kind = .error }
        else if text.contains("WARN") || text.hasPrefix("warning") { kind = .warn }
        else { kind = .info }
        append(LogLine(text: text, kind: kind))
    }

    private func appendSystem(_ text: String, kind: LogLine.Kind = .system) {
        let time = Date().formatted(date: .omitted, time: .standard)
        append(LogLine(text: "[\(time)] \(text)", kind: kind))
    }

    private func append(_ line: LogLine) {
        log.append(line)
        if log.count > Self.maxLogLines { log.removeFirst(log.count - Self.maxLogLines) }
    }
}

private final class LineBuffer: @unchecked Sendable {
    private var data = Data()

    func feed(_ chunk: Data) -> [String] {
        data.append(chunk)
        var lines: [String] = []
        while let nl = data.firstIndex(of: 0x0A) {
            lines.append(String(decoding: data[data.startIndex..<nl], as: UTF8.self))
            data.removeSubrange(data.startIndex...nl)
        }
        return lines
    }
}
