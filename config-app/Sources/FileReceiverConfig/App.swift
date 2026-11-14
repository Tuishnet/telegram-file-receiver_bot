import AppKit
import SwiftUI

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Нужно, если запускать бинарник напрямую (без .app-бандла)
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
    }

    /// Со значком в строке меню приложение остаётся работать после закрытия окна
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        !(UserDefaults.standard.object(forKey: "showMenuBar") as? Bool ?? true)
    }
}

@main
struct FileReceiverConfigApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var model = ConfigModel()
    @AppStorage("showMenuBar") private var showMenuBar = true

    var body: some Scene {
        Window("Настройки бота", id: "main") {
            ContentView(model: model)
        }
        .windowResizability(.contentMinSize)

        MenuBarExtra(isInserted: $showMenuBar) {
            MenuBarContent(model: model)
        } label: {
            MenuBarLabel()
        }
    }
}

struct MenuBarLabel: View {
    @ObservedObject private var runner = BotRunner.shared

    var body: some View {
        Image(systemName: runner.isRunning ? "paperplane.fill" : "paperplane")
    }
}

struct MenuBarContent: View {
    @ObservedObject var model: ConfigModel
    @ObservedObject private var runner = BotRunner.shared
    @ObservedObject private var history = HistoryStore.shared
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Text(runner.isRunning ? "Бот работает" : "Бот остановлен")
        if runner.isRunning {
            Button("Перезапустить") { runner.restart(projectDir: model.projectDir) }
            Button("Остановить") { runner.stop() }
        } else {
            Button("Запустить") { runner.start(projectDir: model.projectDir) }
                .disabled(runner.isBusy)
        }

        Divider()
        let recent = history.recent(7)
        if recent.isEmpty {
            Text("Пока нет принятых файлов")
        } else {
            Text("Последние файлы")
            ForEach(recent) { e in
                Button("\(e.name) · \(e.sizeText)") { HistoryStore.reveal(e) }
                    .disabled(!e.exists)
            }
        }
        Button("Открыть папку с файлами") { model.revealSaveDir() }

        Divider()
        Button("Настройки…") {
            openWindow(id: "main")
            NSApp.activate(ignoringOtherApps: true)
        }
        .keyboardShortcut(",")
        Button("Выйти") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
    }
}
