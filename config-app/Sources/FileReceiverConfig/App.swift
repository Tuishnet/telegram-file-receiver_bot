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

    /// Клик по иконке в Dock, ярлыку на рабочем столе или повторный запуск, когда окно закрыто, —
    /// открываем окно настроек заново
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag {
            MainWindow.show()
        }
        return true
    }
}

/// Открывает главное окно из любого места (в том числе из AppDelegate, где нет SwiftUI-окружения)
@MainActor
enum MainWindow {
    static let id = "main"
    /// Действие SwiftUI «открыть окно» — запоминается, как только появляется любое окно или меню
    static var openAction: OpenWindowAction?

    static func show() {
        if let existing = NSApp.windows.first(where: { $0.identifier?.rawValue.hasPrefix(id) == true && $0.canBecomeKey }) {
            existing.makeKeyAndOrderFront(nil)
        } else {
            openAction?(id: id)
        }
        NSApp.activate(ignoringOtherApps: true)
    }
}

/// Запоминает действие «открыть окно», чтобы им можно было воспользоваться после закрытия окна
struct RememberOpenWindow: ViewModifier {
    @Environment(\.openWindow) private var openWindow

    func body(content: Content) -> some View {
        content.onAppear { MainWindow.openAction = openWindow }
    }
}

@main
struct FileReceiverConfigApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var model = ConfigModel()
    @AppStorage("showMenuBar") private var showMenuBar = true

    var body: some Scene {
        Window("Настройки бота", id: MainWindow.id) {
            ContentView(model: model)
                .modifier(RememberOpenWindow())
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
            MainWindow.openAction = openWindow
            MainWindow.show()
        }
        .keyboardShortcut(",")
        Button("Выйти") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
    }
}
