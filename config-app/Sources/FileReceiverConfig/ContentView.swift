import SwiftUI

struct ContentView: View {
    @ObservedObject var model: ConfigModel
    @StateObject private var ui = UIState()
    @ObservedObject private var runner = BotRunner.shared

    var body: some View {
        VStack(spacing: 0) {
            TabView(selection: $ui.tab) {
                BotTab(model: model)
                    .tabItem { Label("Бот", systemImage: "play.circle") }
                    .tag(UIState.Tab.bot)
                ConnectionTab(model: model, ui: ui)
                    .tabItem { Label("Подключение", systemImage: "key") }
                    .tag(UIState.Tab.connection)
                FilesTab(model: model)
                    .tabItem { Label("Файлы", systemImage: "folder") }
                    .tag(UIState.Tab.files)
                RecognitionTab(model: model)
                    .tabItem { Label("Распознавание", systemImage: "text.viewfinder") }
                    .tag(UIState.Tab.recognition)
                HistoryTab()
                    .tabItem { Label("История", systemImage: "clock.arrow.circlepath") }
                    .tag(UIState.Tab.history)
            }
            .padding(.top, 8)

            Divider()
            footer
        }
        .frame(minWidth: 680, idealWidth: 760, minHeight: 640, idealHeight: 780)
        .onAppear { HistoryStore.shared.configure(projectDir: model.projectDir) }
        .onChange(of: model.projectDir) { HistoryStore.shared.configure(projectDir: model.projectDir) }
    }

    private var footer: some View {
        HStack(alignment: .center) {
            if let message = model.message {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(model.messageIsError ? .red : .secondary)
                    .lineLimit(2)
            }
            Spacer()
            Button("Отменить изменения") { model.load() }
                .disabled(!model.isDirty)
            Button(runner.isRunning ? "Сохранить и перезапустить" : "Сохранить") {
                model.save()
                if runner.isRunning, !model.isDirty {
                    runner.restart(projectDir: model.projectDir)
                }
            }
            .keyboardShortcut("s", modifiers: .command)
            .buttonStyle(.borderedProminent)
            .disabled(!model.isDirty)
        }
        .padding(12)
    }
}

@MainActor
final class UIState: ObservableObject {
    enum Tab: Hashable { case bot, connection, files, recognition, history }

    @Published var tab: Tab = .bot
    @Published var showToken = false
    @Published var showApiHash = false
    @Published var newUser = ""
    @Published var userError: String?
}
