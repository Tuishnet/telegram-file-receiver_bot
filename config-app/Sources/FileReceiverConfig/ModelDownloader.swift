import Combine
import Foundation

/// Модель Whisper для распознавания речи (скачивается с Hugging Face)
struct WhisperModel {
    let id: String
    let title: String
    let size: String
    let repo: String

    var relativePath: String { "models/\(id)" }

    /// Бот работает только с этой моделью (путь models/whisper-small зашит в бот)
    static let small = WhisperModel(id: "whisper-small", title: "Whisper Small", size: "970 МБ", repo: "openai/whisper-small")

    static let files = ["config.json", "tokenizer.json", "model.safetensors"]
}

@MainActor
final class ModelDownloader: ObservableObject {
    static let shared = ModelDownloader()

    @Published private(set) var downloading: String?
    @Published private(set) var progress: Double = 0
    @Published private(set) var error: String?

    private var task: URLSessionDownloadTask?
    private var observation: NSKeyValueObservation?
    private var cancelled = false

    func isReady(_ model: WhisperModel, projectDir: URL) -> Bool {
        let dir = projectDir.appendingPathComponent(model.relativePath)
        return WhisperModel.files.allSatisfy { FileManager.default.fileExists(atPath: dir.appendingPathComponent($0).path) }
    }

    func download(_ model: WhisperModel, projectDir: URL) {
        guard downloading == nil else { return }
        downloading = model.id
        progress = 0
        error = nil
        cancelled = false
        let dir = projectDir.appendingPathComponent(model.relativePath)
        Task {
            do {
                try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
                // Маленькие файлы — ~1% прогресса, сама модель — остальное
                for (i, file) in WhisperModel.files.enumerated() {
                    let dest = dir.appendingPathComponent(file)
                    if FileManager.default.fileExists(atPath: dest.path) { continue }
                    let url = URL(string: "https://huggingface.co/\(model.repo)/resolve/main/\(file)")!
                    let isBig = i == WhisperModel.files.count - 1
                    try await fetch(url, to: dest, base: isBig ? 0.02 : Double(i) * 0.01, span: isBig ? 0.98 : 0.01)
                    if cancelled { throw CancellationError() }
                }
                progress = 1
            } catch is CancellationError {
                error = nil
            } catch {
                self.error = "Не удалось скачать модель: \(error.localizedDescription)"
            }
            downloading = nil
            task = nil
            observation = nil
        }
    }

    func cancel() {
        cancelled = true
        task?.cancel()
    }

    private func fetch(_ url: URL, to dest: URL, base: Double, span: Double) async throws {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
            let t = URLSession.shared.downloadTask(with: url) { tmp, response, err in
                if let err {
                    cont.resume(throwing: (err as? URLError)?.code == .cancelled ? CancellationError() : err)
                    return
                }
                guard let tmp, let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                    let code = (response as? HTTPURLResponse)?.statusCode ?? 0
                    cont.resume(throwing: NSError(domain: "download", code: code,
                                                  userInfo: [NSLocalizedDescriptionKey: "сервер ответил \(code)"]))
                    return
                }
                do {
                    // Атомарно: файл модели появляется только целиком
                    try? FileManager.default.removeItem(at: dest)
                    try FileManager.default.moveItem(at: tmp, to: dest)
                    cont.resume()
                } catch {
                    cont.resume(throwing: error)
                }
            }
            observation = t.progress.observe(\.fractionCompleted) { p, _ in
                let value = base + p.fractionCompleted * span
                Task { @MainActor in ModelDownloader.shared.progress = value }
            }
            task = t
            t.resume()
        }
    }
}
