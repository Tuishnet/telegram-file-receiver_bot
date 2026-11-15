import SwiftUI

// MARK: - Бот: статус, запуск, лог, прочие настройки

struct BotTab: View {
    @ObservedObject var model: ConfigModel
    @ObservedObject private var runner = BotRunner.shared
    @AppStorage("showMenuBar") private var showMenuBar = true

    var body: some View {
        Form {
            Section {
                HStack(spacing: 10) {
                    Circle().fill(statusColor).frame(width: 10, height: 10)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(statusTitle).font(.headline)
                        if let sub = statusSubtitle {
                            Text(sub).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    if runner.isBusy { ProgressView().controlSize(.small) }
                    if runner.isRunning {
                        Button("Перезапустить") { runner.restart(projectDir: model.projectDir) }
                        Button("Остановить", role: .destructive) { runner.stop() }
                    } else {
                        Button {
                            if model.isDirty { model.save() }
                            runner.start(projectDir: model.projectDir)
                        } label: {
                            Label("Запустить", systemImage: "play.fill")
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(runner.isBusy || !canStart)
                    }
                }
                if !canStart && !runner.isRunning {
                    Text(startHint).font(.callout).foregroundStyle(.orange)
                }
                Toggle("Запускать бота при входе в систему", isOn: $runner.autostart)
            } header: {
                Text("Бот")
            } footer: {
                Text("Бот работает как фоновая служба macOS: приложение можно закрыть, бот продолжит работать.")
            }

            Section("Прочее") {
                Toggle("Уведомления macOS о принятых файлах", isOn: $model.notify)
                Toggle("Значок в строке меню", isOn: $showMenuBar)
                LabeledContent("Размер лога до ротации") {
                    Stepper("\(Int(model.logMaxMB)) МБ", value: $model.logMaxMB, in: 1...200, step: 1)
                }
            }

            Section {
                LogView(runner: runner)
            } header: {
                HStack {
                    Text("Лог бота")
                    Spacer()
                    Button("Очистить") { runner.clearLog() }
                        .buttonStyle(.borderless)
                        .font(.caption)
                        .disabled(runner.log.isEmpty)
                }
            }
        }
        .formStyle(.grouped)
    }

    private var canStart: Bool {
        !model.token.trimmingCharacters(in: .whitespaces).isEmpty && model.apiConfigured && model.projectLooksValid
    }

    private var startHint: String {
        if !model.projectLooksValid { return "Укажите папку проекта бота на вкладке «Подключение»." }
        if model.token.trimmingCharacters(in: .whitespaces).isEmpty {
            return "Впишите токен на вкладке «Подключение»."
        }
        return "Впишите API ID и API Hash на вкладке «Подключение»."
    }

    private var statusColor: Color {
        switch runner.status {
        case .running, .external: .green
        case .building, .waiting: .yellow
        case .stopped: .gray
        case .failed: .red
        }
    }

    private var statusTitle: String {
        switch runner.status {
        case .running: "Работает в фоне"
        case .waiting: "Перезапускается…"
        case .external: "Работает (запущен не службой)"
        case .building: "Сборка…"
        case .stopped: "Остановлен"
        case .failed(let reason): reason
        }
    }

    private var statusSubtitle: String? {
        switch runner.status {
        case .running(let pid), .external(let pid): "PID \(pid)"
        default: nil
        }
    }
}

struct LogView: View {
    @ObservedObject var runner: BotRunner

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2) {
                    ForEach(runner.log) { line in
                        Text(line.text)
                            .font(.caption.monospaced())
                            .foregroundStyle(color(for: line.kind))
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .textSelection(.enabled)
                            .id(line.id)
                    }
                }
                .padding(6)
            }
            .frame(height: 260)
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .onChange(of: runner.log.count) {
                if let last = runner.log.last { proxy.scrollTo(last.id, anchor: .bottom) }
            }
            .overlay {
                if runner.log.isEmpty { Text("Лог пуст").foregroundStyle(.tertiary) }
            }
        }
    }

    private func color(for kind: BotRunner.LogLine.Kind) -> Color {
        switch kind {
        case .info: .primary
        case .warn: .orange
        case .error: .red
        case .system: .secondary
        }
    }
}

// MARK: - Подключение: проект, токен, API, доверенные аккаунты

struct ConnectionTab: View {
    @ObservedObject var model: ConfigModel
    @ObservedObject var ui: UIState

    var body: some View {
        Form {
            Section {
                HStack {
                    Text(model.projectDir.path)
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .textSelection(.enabled)
                    Spacer()
                    Button("Выбрать…") { model.chooseProjectDir() }
                }
                if !model.projectLooksValid {
                    Label("В этой папке нет Cargo.toml — точно ли это проект бота?", systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.orange)
                        .font(.callout)
                }
            } header: {
                Text("Папка бота")
            } footer: {
                Text("Настройки сохраняются в файл .env в этой папке.")
            }

            Section {
                HStack {
                    SecretField(placeholder: "123456789:AA…", text: $model.token, shown: $ui.showToken)
                    Button("Проверить") { Task { await model.checkToken() } }
                        .disabled(model.token.isEmpty || model.tokenStatus == .checking)
                }
                tokenStatusView
            } header: {
                Text("Токен бота")
            } footer: {
                Text("Получите токен у @BotFather в Telegram командой /newbot.")
            }

            Section {
                LabeledContent("API ID") {
                    TextField("12345678", text: $model.apiId)
                        .textFieldStyle(.roundedBorder)
                        .font(.body.monospaced())
                        .labelsHidden()
                }
                if !model.apiId.isEmpty && !model.apiIdIsValid {
                    Text("API ID состоит только из цифр").foregroundStyle(.red).font(.callout)
                }
                LabeledContent("API Hash") {
                    SecretField(placeholder: "32 символа", text: $model.apiHash, shown: $ui.showApiHash)
                }
                if !model.apiHash.isEmpty && !model.apiHashIsValid {
                    Text("API Hash — это 32 символа из цифр и букв a–f").foregroundStyle(.red).font(.callout)
                }
                if model.apiConfigured {
                    Label("Можно принимать файлы до 2 ГБ", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                }
            } header: {
                Text("Доступ к Telegram API")
            } footer: {
                HStack(spacing: 4) {
                    Text("Получите их на")
                    Link("my.telegram.org", destination: URL(string: "https://my.telegram.org/apps")!)
                    Text("→ API development tools.")
                }
            }

            Section {
                if model.allowedUsers.isEmpty {
                    Text("Нет доверенных аккаунтов — бот не будет сохранять файлы ни от кого.")
                        .foregroundStyle(.secondary)
                        .font(.callout)
                }
                ForEach(model.allowedUsers, id: \.self) { id in
                    HStack {
                        Image(systemName: "person.crop.circle").foregroundStyle(.secondary)
                        Text(id).font(.body.monospaced()).textSelection(.enabled)
                        Spacer()
                        Button(role: .destructive) { model.removeUser(id) } label: {
                            Image(systemName: "minus.circle.fill")
                        }
                        .buttonStyle(.borderless)
                        .foregroundStyle(.red)
                        .help("Удалить")
                    }
                }
                HStack {
                    TextField("Telegram ID, например 123456789", text: $ui.newUser)
                        .textFieldStyle(.roundedBorder)
                        .font(.body.monospaced())
                        .labelsHidden()
                        .onSubmit(addUser)
                        .onChange(of: ui.newUser) { ui.userError = nil }
                    Button("Добавить", action: addUser)
                        .disabled(ui.newUser.trimmingCharacters(in: .whitespaces).isEmpty)
                }
                if let error = ui.userError {
                    Text(error).foregroundStyle(.red).font(.callout)
                }
            } header: {
                Text("Доверенные аккаунты")
            } footer: {
                Text("Узнать свой ID: напишите боту что-нибудь — он ответит вашим ID.")
            }
        }
        .formStyle(.grouped)
    }

    @ViewBuilder
    private var tokenStatusView: some View {
        switch model.tokenStatus {
        case .unknown:
            EmptyView()
        case .checking:
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Проверяю…").foregroundStyle(.secondary)
            }
        case .valid(let username):
            Label("Токен рабочий: @\(username)", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
        case .invalid(let reason):
            Label(reason, systemImage: "xmark.octagon.fill").foregroundStyle(.red)
        }
    }

    private func addUser() {
        if let error = model.addUser(ui.newUser) {
            ui.userError = error
        } else {
            ui.newUser = ""
            ui.userError = nil
        }
    }
}

struct SecretField: View {
    let placeholder: String
    @Binding var text: String
    @Binding var shown: Bool

    var body: some View {
        HStack {
            Group {
                if shown {
                    TextField(placeholder, text: $text)
                } else {
                    SecureField(placeholder, text: $text)
                }
            }
            .textFieldStyle(.roundedBorder)
            .font(.body.monospaced())
            .labelsHidden()
            Button { shown.toggle() } label: {
                Image(systemName: shown ? "eye.slash" : "eye")
            }
            .buttonStyle(.borderless)
            .help(shown ? "Скрыть" : "Показать")
        }
    }
}

// MARK: - Файлы: папка, сортировка, защита, архивы, резервная копия

struct FilesTab: View {
    @ObservedObject var model: ConfigModel

    var body: some View {
        Form {
            Section {
                HStack {
                    TextField("files", text: $model.saveDir)
                        .textFieldStyle(.roundedBorder)
                        .font(.body.monospaced())
                        .labelsHidden()
                    Button("Выбрать…") { model.chooseSaveDir() }
                    Button { model.revealSaveDir() } label: { Image(systemName: "folder") }
                        .help("Показать в Finder")
                }
                Text(model.resolvedSaveDir.path)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                FolderStatusView(status: ConfigModel.folderStatus(model.resolvedSaveDir))
            } header: {
                Text("Папка для файлов")
            } footer: {
                Text("Относительный путь считается от папки бота.")
            }

            Section {
                Toggle("Раскладывать по типам: Фото, Видео, Аудио, Голосовые, Документы, Архивы", isOn: $model.sortByType)
                Toggle("Раскладывать по месяцам (2026-11)", isOn: $model.sortByDate)
                Toggle("Файлы из групп — в папку «Группы/название группы»", isOn: $model.groupFolders)
                Toggle("Подпись к файлу становится его именем", isOn: $model.captionAsName)
                Text("Пример: \(examplePath)")
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
            } header: {
                Text("Порядок")
            }

            Section {
                Toggle("Не сохранять дубли (тот же файл или файл с тем же содержимым)", isOn: $model.dedup)
                LabeledContent("Оставлять свободными на диске") {
                    Stepper("\(Int(model.minFreeGB)) ГБ", value: $model.minFreeGB, in: 0...500, step: 1)
                }
            } header: {
                Text("Защита")
            } footer: {
                Text("Если после загрузки свободного места станет меньше — бот откажется принимать файл.")
            }

            Section {
                Toggle("Распаковывать ZIP-архивы в папку рядом", isOn: $model.unzip)
            } header: {
                Text("Архивы")
            } footer: {
                Text("Сам архив тоже сохраняется. Опасные пути внутри архива и «zip-бомбы» отсекаются.")
            }

            Section {
                Toggle("Делать копию каждого файла", isOn: Binding(
                    get: { !model.backupDir.isEmpty },
                    set: { on in
                        if on { model.useICloudBackup() } else { model.backupDir = "" }
                    }
                ))
                if !model.backupDir.isEmpty {
                    HStack {
                        Text(model.backupDir)
                            .font(.callout.monospaced())
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Spacer()
                        Button("iCloud Drive") { model.useICloudBackup() }
                        Button("Выбрать…") { model.chooseBackupDir() }
                    }
                    FolderStatusView(status: ConfigModel.folderStatus(model.resolve(model.backupDir)))
                }
            } header: {
                Text("Резервная копия")
            } footer: {
                Text("Подойдёт любая синхронизируемая папка: iCloud Drive, Google Drive, Яндекс Диск, Dropbox.")
            }
        }
        .formStyle(.grouped)
    }

    private var examplePath: String {
        var parts = ["…"]
        if model.groupFolders { parts.append("Группы/Семья") }
        if model.sortByType { parts.append("Фото") }
        if model.sortByDate { parts.append(Date().formatted(.iso8601.year().month())) }
        parts.append(model.captionAsName ? "Паспорт.jpg" : "photo_2026-11-14_23-10-00.jpg")
        return parts.joined(separator: "/")
    }
}

// MARK: - Распознавание: речь (Whisper) и текст (Vision)

struct RecognitionTab: View {
    @ObservedObject var model: ConfigModel
    @ObservedObject private var downloader = ModelDownloader.shared

    private let languages: [(String, String)] = [
        ("auto", "Определять автоматически"), ("ru", "Русский"), ("uk", "Украинский"), ("en", "Английский"),
    ]

    var body: some View {
        Form {
            Section {
                Toggle("Расшифровывать голосовые, кружочки и аудио", isOn: $model.transcribe)
                Toggle("Расшифровывать и обычные видео", isOn: $model.transcribeVideo)
                    .disabled(!model.transcribe)
                LabeledContent("Из длинного видео расшифровывать") {
                    Stepper("первые \(Int(model.videoMaxMin)) мин", value: $model.videoMaxMin, in: 5...240, step: 5)
                }
                .disabled(!model.transcribe || !model.transcribeVideo)
                Picker("Язык речи", selection: $model.whisperLang) {
                    ForEach(languages, id: \.0) { Text($0.1).tag($0.0) }
                }
                .disabled(!model.transcribe)
                Toggle("Считать на видеокарте (Metal) — в 3 раза быстрее", isOn: $model.whisperGPU)
                    .disabled(!model.transcribe)
                LabeledContent("Выгружать модель из памяти") {
                    Stepper(model.whisperUnloadMin == 0 ? "никогда" : "через \(Int(model.whisperUnloadMin)) мин без голосовых",
                            value: $model.whisperUnloadMin, in: 0...120, step: 5)
                }
                .disabled(!model.transcribe)
            } header: {
                Text("Речь в текст")
            } footer: {
                Text("Распознавание идёт прямо на Маке моделью Whisper (на Rust, фреймворк candle) — записи никуда не отправляются. Загруженная модель занимает около 1 ГБ памяти, поэтому в простое её лучше выгружать.")
            }

            Section {
                modelRow(WhisperModel.small)
                if let error = downloader.error {
                    Text(error).foregroundStyle(.red).font(.callout)
                }
            } header: {
                Text("Модель распознавания речи")
            } footer: {
                Text("Модель скачивается с Hugging Face в папку models/ проекта бота.")
            }

            Section {
                Toggle("Распознавать текст на фото и в PDF", isOn: $model.ocr)
            } header: {
                Text("Текст с картинок")
            } footer: {
                Text("Используется встроенный в macOS движок Vision; работает с русским, украинским и английским, в том числе для сканов PDF.")
            }

            Section("Куда девать распознанный текст") {
                Toggle("Отвечать текстом в Telegram", isOn: $model.replyText)
                Toggle("Сохранять .txt рядом с файлом", isOn: $model.saveTextFiles)
                Text("Текст всегда попадает в историю — по нему работают поиск /find и вкладка «История».")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }

    @ViewBuilder
    private func modelRow(_ m: WhisperModel) -> some View {
        let ready = downloader.isReady(m, projectDir: model.projectDir)
        HStack {
            Text("\(m.title) · \(m.size)").font(.body)
            Spacer()
            if downloader.downloading == m.id {
                ProgressView(value: downloader.progress).frame(width: 120)
                Text("\(Int(downloader.progress * 100))%").font(.caption.monospacedDigit()).frame(width: 36)
                Button("Отмена") { downloader.cancel() }
            } else if ready {
                Label("Скачана", systemImage: "checkmark.circle.fill").foregroundStyle(.green).font(.callout)
            } else {
                Button("Скачать") { downloader.download(m, projectDir: model.projectDir) }
                .disabled(downloader.downloading != nil)
            }
        }
    }
}

/// Строка состояния папки: доступна / будет создана / проблема
struct FolderStatusView: View {
    let status: ConfigModel.FolderStatus

    var body: some View {
        switch status {
        case .ok:
            Label("Папка доступна", systemImage: "checkmark.circle.fill")
                .foregroundStyle(.green)
                .font(.callout)
        case .willBeCreated:
            Label("Папки пока нет — бот создаст её сам", systemImage: "folder.badge.plus")
                .foregroundStyle(.secondary)
                .font(.callout)
        case .problem(let text):
            Label(text, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
                .font(.callout)
        }
    }
}
