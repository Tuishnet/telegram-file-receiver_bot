# telegram-file-receiver_bot

**English** · [Русский](#русский)

A Telegram bot that saves every file you send it to a folder on your Mac. Written in Rust on top of [grammers](https://codeberg.org/Lonami/grammers) (MTProto), so it accepts files up to 2 GB (4 GB from Premium accounts), unlike the 20 MB Bot API limit.

Comes with a native SwiftUI manager app (settings, status, history, menu bar icon).

## Features

- Documents, photos, videos, audio, voice messages, video notes, GIFs, albums
- Sorting by type, by month, per-group folders; caption used as file name
- Duplicate detection (Telegram media id + streaming SHA-256), free disk space guard
- ZIP extraction with zip-bomb protection (UTF-8 and CP866 file names)
- Optional backup copy (e.g. iCloud Drive)
- Speech-to-text for voice messages, video notes, audio and videos — Whisper via [candle](https://github.com/huggingface/candle) on Metal GPU (f16), model unloaded when idle
- OCR for photos and PDFs (including scans) via macOS Vision
- macOS notifications with "Show in Finder"
- Commands: `/list`, `/find text`, `/get N`, `/delete [N]` (moves to Trash), `/disk`, `/help`

## Requirements

- macOS on Apple Silicon (Metal, Vision and notifications are macOS-only)
- Rust (stable), Xcode Command Line Tools for the manager app

## Quick start

1. Create a bot with @BotFather and get its token.
2. Get `API_ID` and `API_HASH` at my.telegram.org → API development tools.
3. `cp .env.example .env` and fill in `TELOXIDE_TOKEN`, `API_ID`, `API_HASH`.
4. `cargo run --release`
5. Message the bot — it replies with your Telegram ID. Put it into `ALLOWED_USERS` and restart.

Files go to `SAVE_DIR` (default `files/`). The session is stored in `bot-<bot id>.session` — it is an access key, never share it.

To run as a background service, build the app bundle with `./bundle-bot.sh` and the manager with `config-app/build-app.sh`; the manager installs the LaunchAgent.

All settings are documented in `.env.example` and in the Russian section below.

## License

MIT. `src/recognize/whisper_model.rs` is derived from candle-transformers (MIT OR Apache-2.0, © Hugging Face).

---

<a name="русский"></a>
# Русский


Телеграм-бот: присылаете ему файл — он сохраняет его в папку.

Поддерживает документы, фото, видео, аудио, голосовые, кружочки и GIF.

## Запуск

1. Создайте бота у @BotFather и получите токен.
2. `cp .env.example .env` и впишите токен в `TELOXIDE_TOKEN`, а также `API_ID` и `API_HASH`.
3. `cargo run --release`
4. Напишите боту — он ответит вашим Telegram ID. Впишите его в `ALLOWED_USERS` и перезапустите.

Файлы сохраняются в `SAVE_DIR` (по умолчанию `files/`). Одинаковые имена не перезаписываются — добавляется `(1)`, `(2)`, ...

Бот работает напрямую через MTProto (библиотека [grammers](https://codeberg.org/Lonami/grammers)), поэтому принимает файлы до 2 ГБ (до 4 ГБ от Premium). Для этого нужны `API_ID` и `API_HASH` с my.telegram.org → API development tools.

Сессия хранится в `bot-<id бота>.session` — это ключ доступа, не передавайте его никому.

Старая версия на teloxide (Bot API, лимит 20 МБ) лежит в `backup/main_teloxide.rs`.

## Работа в группах

В группах бот сохраняет файлы только от доверенных аккаунтов (`ALLOWED_USERS`), остальных молча игнорирует и не отвечает на обычные сообщения.

Чтобы бот видел файлы в группе, у @BotFather выполните `/setprivacy` → выберите бота → **Disable**, затем удалите бота из группы и добавьте снова. Либо сделайте бота администратором группы.

## Возможности

| Функция | Настройка в `.env` | Где в приложении |
|---|---|---|
| Сортировка по типам (Фото, Видео, Аудио, Голосовые, Документы, Архивы) | `SORT_BY_TYPE` | Файлы → Порядок |
| Сортировка по месяцам | `SORT_BY_DATE` | Файлы → Порядок |
| Файлы из групп — в `Группы/<название>` | `GROUP_FOLDERS` | Файлы → Порядок |
| Подпись к файлу как имя | `CAPTION_AS_NAME` | Файлы → Порядок |
| Без дублей (по id в Telegram и SHA-256) | `DEDUP` | Файлы → Защита |
| Минимум свободного места | `MIN_FREE_GB` | Файлы → Защита |
| Распаковка ZIP (с защитой от zip-бомб) | `UNZIP` | Файлы → Архивы |
| Резервная копия (iCloud Drive и др.) | `BACKUP_DIR` | Файлы → Резервная копия |
| Прогресс загрузки больших файлов, загрузка в 8 потоков | — | — |
| Альбомы: один ответ на всю пачку | — | — |
| Расшифровка голосовых/кружочков/аудио (Whisper на candle) | `TRANSCRIBE`, `WHISPER_LANG` | Распознавание |
| Расшифровка обычных видео (MP4, MOV, WebM, MKV), первые N минут | `TRANSCRIBE_VIDEO`, `VIDEO_MAX_MIN` | Распознавание |
| Текст с фото и PDF, включая сканы (macOS Vision) | `OCR` | Распознавание |
| Ответ текстом и .txt рядом с файлом | `REPLY_TEXT`, `SAVE_TEXT_FILES` | Распознавание |
| Уведомления macOS с кнопкой «Показать в Finder» | `NOTIFY` | Бот → Прочее |
| Ротация лога | `LOG_MAX_MB` | Бот → Прочее |
| Распознавание речи на видеокарте (Metal, f16) | `WHISPER_GPU` | Распознавание |
| Выгрузка модели речи из памяти в простое (0 — никогда) | `WHISPER_UNLOAD_MIN` | Распознавание |

Команды в Telegram: `/list`, `/find текст`, `/get номер`, `/delete [номер]` (в Корзину), `/disk`, `/help`.

История принятых файлов — `history.jsonl` (её показывает вкладка «История» в приложении).

Самопроверка без Telegram:

```
./target/release/file_receiver --ocr картинка.png
./target/release/file_receiver --pdf документ.pdf
./target/release/file_receiver --decode голосовое.ogg
./target/release/file_receiver --transcribe голосовое.ogg
./target/release/file_receiver --video видео.mp4
./target/release/file_receiver --notify
```

## Как устроено ради скорости и экономии

- Бот запускается из `FileReceiverBot.app` (`bundle-bot.sh`) — без бандла macOS не даёт показывать уведомления.
- Речь распознаётся на видеокарте в половинной точности (`src/recognize/whisper_model.rs` — модель Whisper из candle с правками для f16). Голосовые обрабатываются строго по очереди, картинки — до 4 параллельно.
- Модель речи выгружается из памяти через `WHISPER_UNLOAD_MIN` минут простоя.
- Большие файлы качаются в 8 потоков, SHA-256 для поиска дублей считается на лету, без повторного чтения файла.
- Менеджер узнаёт об изменениях лога и истории через события файловой системы (kqueue), а статус службы опрашивает раз в 3 с на переднем плане и раз в 30 с в фоне.
