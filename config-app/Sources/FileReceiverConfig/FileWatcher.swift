import Foundation

/// Следит за файлом через события ядра (kqueue) вместо постоянного опроса.
/// Переживает ротацию и атомарную перезапись: при удалении/переименовании открывает файл заново.
@MainActor
final class FileWatcher {
    private let url: URL
    private let onChange: @MainActor () -> Void
    private var source: DispatchSourceFileSystemObject?
    private var retry: DispatchWorkItem?

    init(url: URL, onChange: @escaping @MainActor () -> Void) {
        self.url = url
        self.onChange = onChange
        start()
    }

    func stop() {
        retry?.cancel()
        source?.cancel()
        source = nil
    }

    private func start() {
        let fd = open(url.path, O_EVTONLY)
        guard fd >= 0 else {
            // Файла пока нет — проверим позже
            schedule(after: 2)
            return
        }
        let src = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: fd, eventMask: [.write, .extend, .delete, .rename, .revoke], queue: .main)
        src.setEventHandler { [weak self] in
            MainActor.assumeIsolated {
                guard let self, let src = self.source else { return }
                let replaced = !src.data.isDisjoint(with: [.delete, .rename, .revoke])
                self.onChange()
                if replaced {
                    src.cancel()
                    self.source = nil
                    self.schedule(after: 0.3)
                }
            }
        }
        src.setCancelHandler { close(fd) }
        source = src
        src.resume()
    }

    private func schedule(after seconds: Double) {
        retry?.cancel()
        let item = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.start()
                self.onChange()
            }
        }
        retry = item
        DispatchQueue.main.asyncAfter(deadline: .now() + seconds, execute: item)
    }

    deinit {
        source?.cancel()
    }
}
