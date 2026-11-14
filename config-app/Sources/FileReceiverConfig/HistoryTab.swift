import QuickLookThumbnailing
import SwiftUI

struct HistoryTab: View {
    @ObservedObject private var store = HistoryStore.shared
    @StateObject private var state = HistoryState()

    var body: some View {
        HSplitView {
            VStack(spacing: 0) {
                TextField("Поиск по имени, тексту и группе", text: $state.query)
                    .textFieldStyle(.roundedBorder)
                    .padding(8)
                let items = store.search(state.query)
                List(items, selection: $state.selected) { e in
                    HistoryRow(entry: e).tag(e.id)
                        .contextMenu {
                            Button("Открыть") { HistoryStore.open(e) }
                            Button("Показать в Finder") { HistoryStore.reveal(e) }
                        }
                }
                .overlay {
                    if items.isEmpty {
                        Text(state.query.isEmpty ? "Пока нет принятых файлов" : "Ничего не найдено")
                            .foregroundStyle(.tertiary)
                    }
                }
                Text("Файлов: \(items.count)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(6)
            }
            .frame(minWidth: 300)

            Group {
                if let id = state.selected, let e = store.entries.first(where: { $0.id == id }) {
                    HistoryDetail(entry: e)
                } else {
                    Text("Выберите файл").foregroundStyle(.tertiary).frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .frame(minWidth: 300)
        }
    }
}

@MainActor
final class HistoryState: ObservableObject {
    @Published var query = ""
    @Published var selected: UInt64?
}

struct HistoryRow: View {
    let entry: HistoryEntry

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: entry.symbol)
                .frame(width: 20)
                .foregroundStyle(entry.exists ? Color.accentColor : .secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.name).lineLimit(1).truncationMode(.middle)
                HStack(spacing: 6) {
                    Text("#\(entry.id)")
                    Text(entry.sizeText)
                    Text(entry.dateText)
                    if let chat = entry.chat, !chat.isEmpty { Text("· \(chat)") }
                    if entry.text != nil { Image(systemName: "text.quote") }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
        .opacity(entry.exists ? 1 : 0.5)
    }
}

struct HistoryDetail: View {
    let entry: HistoryEntry
    @StateObject private var thumb = Thumbnail()

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Group {
                    if let img = thumb.image {
                        Image(nsImage: img).resizable().scaledToFit()
                    } else {
                        Image(nsImage: NSWorkspace.shared.icon(forFile: entry.path)).resizable().scaledToFit()
                            .frame(width: 96, height: 96)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: 280)

                Text(entry.name).font(.headline).textSelection(.enabled)
                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 4) {
                    GridRow { Text("Размер").foregroundStyle(.secondary); Text(entry.sizeText) }
                    GridRow { Text("Получен").foregroundStyle(.secondary); Text(entry.dateText) }
                    if let chat = entry.chat, !chat.isEmpty {
                        GridRow { Text("Группа").foregroundStyle(.secondary); Text(chat) }
                    }
                    GridRow {
                        Text("Путь").foregroundStyle(.secondary)
                        Text(entry.path).font(.caption.monospaced()).textSelection(.enabled).lineLimit(3)
                    }
                }
                .font(.callout)

                if entry.exists {
                    HStack {
                        Button("Открыть") { HistoryStore.open(entry) }
                        Button("Показать в Finder") { HistoryStore.reveal(entry) }
                    }
                } else {
                    Label("Файл удалён или перемещён", systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                }

                if let text = entry.text, !text.isEmpty {
                    Divider()
                    Text(entry.kind == "voice" || entry.kind == "audio" || entry.kind == "video_note"
                         ? "Расшифровка" : "Распознанный текст")
                        .font(.headline)
                    Text(text).textSelection(.enabled).font(.callout)
                }
            }
            .padding()
        }
        .onAppear { thumb.load(entry) }
        .onChange(of: entry.id) { thumb.load(entry) }
    }
}

@MainActor
final class Thumbnail: ObservableObject {
    @Published var image: NSImage?

    func load(_ e: HistoryEntry) {
        image = nil
        guard e.exists else { return }
        let req = QLThumbnailGenerator.Request(fileAt: e.url, size: CGSize(width: 600, height: 600),
                                               scale: 2, representationTypes: .thumbnail)
        QLThumbnailGenerator.shared.generateBestRepresentation(for: req) { rep, _ in
            let img = rep?.nsImage
            Task { @MainActor in self.image = img }
        }
    }
}
