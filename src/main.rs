//! Телеграм-бот, который сохраняет присланные файлы на Мак.
//!
//! Работает напрямую через MTProto (библиотека grammers), поэтому принимает файлы до 2 ГБ.
//! Умеет: сортировку по папкам, прогресс загрузки, альбомы, защиту диска и от дублей,
//! распаковку ZIP, резервную копию, уведомления macOS, распознавание речи (Whisper на candle)
//! и текста (macOS Vision), команды /list /find /get /delete /disk.

mod archive;
mod commands;
mod config;
mod download;
mod history;
mod logger;
mod notify;
mod recognize;
mod storage;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use grammers_client::Client;
use grammers_client::media::{Document, Media};
use grammers_client::message::Message;
use grammers_client::update::Update;
use grammers_mtsender::{SenderPool, UpdatesConfiguration};
use grammers_session::storages::SqliteSession;
use grammers_session::types::PeerKind;
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinSet;

use config::Config;
use history::{Entry, History};
use storage::{Kind, human_size};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// С какого размера показывать прогресс загрузки
const PROGRESS_FROM: u64 = 10 * 1024 * 1024;
/// Как часто обновлять сообщение с прогрессом
const PROGRESS_EVERY: Duration = Duration::from_secs(3);
/// Сколько ждать остальные файлы альбома
const ALBUM_WAIT: Duration = Duration::from_millis(1500);
/// Ограничение длины распознанного текста в ответе
const REPLY_TEXT_LIMIT: usize = 3500;

pub struct App {
    pub client: Client,
    pub cfg: Config,
    pub history: History,
    albums: Mutex<HashMap<i64, Album>>,
    /// Речь распознаём строго по одной и в порядке поступления (tokio::Mutex — честная очередь)
    speech_queue: tokio::sync::Mutex<()>,
    /// Текст с картинок можно распознавать параллельно, но не больше 4 сразу
    ocr_slots: tokio::sync::Semaphore,
    /// Большие файлы качаются не больше чем по BIG_DOWNLOADS сразу (каждый — в 8 потоков),
    /// иначе Telegram может временно ограничить бота (FLOOD_WAIT)
    big_downloads: tokio::sync::Semaphore,
}

const BIG_DOWNLOADS: usize = 2;

/// Сколько картинок распознавать одновременно
const OCR_PARALLEL: usize = 4;

struct Album {
    first: Message,
    pending: usize,
    generation: u64,
    saved: Vec<(PathBuf, u64)>,
    duplicates: usize,
    failed: Vec<String>,
    texts: Vec<AlbumText>,
}

/// Распознанный текст одного файла из альбома
struct AlbumText {
    /// Номер сообщения — чтобы тексты шли в том же порядке, что и файлы
    order: i32,
    name: String,
    speech: bool,
    text: String,
}

/// Лимит Telegram на длину сообщения — 4096 символов, оставляем запас
const MESSAGE_LIMIT: usize = 4000;
/// Сколько сообщений с распознанным текстом присылать на один альбом
const MAX_TEXT_MESSAGES: usize = 10;

/// Описание вложения
struct Incoming {
    name: String,
    size: u64,
    kind: Kind,
    media_id: i64,
}

enum Outcome {
    Saved { id: u64, path: PathBuf, size: u64, kind: Kind, notes: Vec<String> },
    Duplicate(Entry),
    Failed(String),
}

fn main() {
    dotenvy::dotenv().ok();
    let selftest_mode = std::env::args().nth(1).is_some_and(|a| a.starts_with("--"));
    let cfg = Config::load();
    if selftest_mode {
        // В самопроверке лог идёт в консоль
        logger::RotatingLogger::init("/dev/stderr", u64::MAX);
    } else {
        let log_max = cfg.as_ref().map(|c| c.log_max_bytes).unwrap_or(10 << 20);
        logger::RotatingLogger::init("bot.log", log_max);
    }
    // Уведомления настраиваются в основном потоке: туда же macOS присылает нажатия
    if selftest_mode || cfg.as_ref().is_ok_and(|c| c.notify) {
        notify::init();
    }

    // Сам бот (Telegram, загрузки, распознавание) — в отдельном потоке с tokio
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio");
        let code = rt.block_on(async move {
            if selftest_mode {
                selftest().await.unwrap_or(2)
            } else {
                bot_main(cfg).await
            }
        });
        log::logger().flush();
        std::process::exit(code);
    });

    // Основной поток обслуживает системные события (ответы на уведомления)
    objc2_core_foundation::CFRunLoop::run();
}

async fn bot_main(cfg: Result<Config, String>) -> i32 {
    let cfg = match cfg {
        Ok(c) => c,
        Err(e) => {
            // Код 0, чтобы служба не перезапускала бота каждые 10 секунд с той же ошибкой
            log::error!("{e}");
            log::error!("Исправьте настройки в приложении «Настройки бота» и запустите бота снова.");
            return 0;
        }
    };
    match run(cfg).await {
        Ok(()) => 0,
        Err(e) => {
            log::error!("Бот остановлен из-за ошибки: {e}");
            1
        }
    }
}

/// Самопроверка без Telegram: `file_receiver --ocr|--pdf|--decode|--transcribe <файл>` или `--notify`
async fn selftest() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1)?.as_str();
    if !mode.starts_with("--") {
        return None;
    }
    let path = PathBuf::from(args.get(2).cloned().unwrap_or_default());
    let result: Result<String, String> = match mode {
        "--ocr" => recognize::ocr::image_text(&path),
        "--pdf" => recognize::ocr::pdf_text(&path),
        "--unzip" => archive::unzip(&path, 1 << 30).map(|u| format!("{} файлов ({} байт) → {}", u.files, u.bytes, u.dir.display())),
        "--decode" => recognize::audio::decode_to_16k_mono(&path, 7200)
            .map(|pcm| format!("{:.1} с аудио, {} сэмплов 16 кГц", pcm.len() as f32 / 16000.0, pcm.len())),
        "--transcribe" => {
            let model = std::env::var("WHISPER_MODEL").map(PathBuf::from).unwrap_or_else(|_| "models/whisper-small".into());
            let lang = std::env::var("WHISPER_LANG").unwrap_or_else(|_| "auto".into());
            let gpu = std::env::var("WHISPER_GPU").map(|v| v != "false").unwrap_or(true);
            recognize::audio::decode_to_16k_mono(&path, 7200).and_then(|pcm| {
                let t = Instant::now();
                let r = recognize::whisper::transcribe(&model, &pcm, &lang, gpu)
                    .map(|(text, l)| format!("[{l}, {:.1} с] {text}", t.elapsed().as_secs_f32()));
                // Для замера памяти после распознавания: SELFTEST_HOLD=секунды
                if let Some(secs) = std::env::var("SELFTEST_HOLD").ok().and_then(|v| v.parse().ok()) {
                    std::thread::sleep(Duration::from_secs(secs));
                }
                r
            })
        }
        "--notify" => {
            if !notify::in_bundle() {
                Err("уведомления работают только при запуске из FileReceiverBot.app".into())
            } else {
                // Ждём ответа на запрос разрешения
                tokio::time::sleep(Duration::from_secs(2)).await;
                notify::file_saved("Проверка уведомлений".into(), "Бот умеет показывать уведомления".into(), PathBuf::from("."));
                tokio::time::sleep(Duration::from_secs(3)).await;
                Ok(format!(
                    "уведомление отправлено, разрешение: {}; {}; ошибка запроса: «{}»; бандл: {}",
                    if notify::enabled() { "есть" } else { "нет" },
                    notify::authorization_status(),
                    notify::last_error(),
                    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
                ))
            }
        }
        _ => return None,
    };
    match result {
        Ok(s) => {
            println!("{s}");
            Some(0)
        }
        Err(e) => {
            eprintln!("Ошибка: {e}");
            Some(1)
        }
    }
}

/// Сессия своя для каждого бота: если сменить токен, старая сессия не помешает
fn session_path(token: &str) -> PathBuf {
    let bot_id = token.split(':').next().unwrap_or("bot");
    PathBuf::from(format!("bot-{bot_id}.session"))
}

async fn run(cfg: Config) -> Result<(), Error> {
    tokio::fs::create_dir_all(&cfg.save_dir).await?;
    if cfg.allowed_users.is_empty() {
        log::warn!("ALLOWED_USERS не задан: бот не будет сохранять файлы, а только сообщит каждому его Telegram ID");
    }

    let session_file = session_path(&cfg.token);
    let session = Arc::new(SqliteSession::open(&session_file).await?);
    restrict_permissions(&session_file);
    // История (с распознанным текстом) и логи — только для владельца
    for f in ["history.jsonl", "bot.log", "bot.log.1", "bot.log.2", "bot-stderr.log"] {
        if Path::new(f).exists() {
            restrict_permissions(Path::new(f));
        }
    }

    let SenderPool { runner, updates, handle } = SenderPool::new(Arc::clone(&session), cfg.api_id);
    let client = Client::new(handle.clone());
    let pool_task = tokio::spawn(runner.run());

    if !client.is_authorized().await? {
        log::info!("Вход в Telegram…");
        client.bot_sign_in(&cfg.token, &cfg.api_hash).await?;
    }
    let me = client.get_me().await?;
    log::info!("Бот @{} запущен. Файлы сохраняются в {}", me.username().unwrap_or("?"), cfg.save_dir.display());
    log_features(&cfg);

    if let Err(e) = commands::register(&client).await {
        log::warn!("Не удалось зарегистрировать меню команд: {e}");
    }

    let app = Arc::new(App {
        client: client.clone(),
        cfg,
        history: History::open("history.jsonl"),
        albums: Default::default(),
        speech_queue: tokio::sync::Mutex::new(()),
        ocr_slots: tokio::sync::Semaphore::new(OCR_PARALLEL),
        big_downloads: tokio::sync::Semaphore::new(BIG_DOWNLOADS),
    });

    if app.cfg.ocr {
        tokio::task::spawn_blocking(recognize::ocr::warm_up);
    }

    // Выгрузка модели речи из памяти, когда ею давно не пользовались
    if app.cfg.whisper_unload_minutes > 0 {
        let idle = Duration::from_secs(app.cfg.whisper_unload_minutes * 60);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                tokio::task::spawn_blocking(move || recognize::whisper::unload_if_idle(idle));
            }
        });
    }

    let mut updates = client.stream_updates(updates, UpdatesConfiguration { catch_up: true }).await?;
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut tasks = JoinSet::new();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = sigterm.recv() => break,
            update = updates.next() => {
                if let Update::NewMessage(msg) = update? {
                    if !msg.outgoing() {
                        let app = Arc::clone(&app);
                        tasks.spawn(async move {
                            if let Err(e) = handle_message(&app, &msg).await {
                                log::error!("Ошибка обработки сообщения: {e}");
                            }
                        });
                    }
                }
            }
            Some(res) = tasks.join_next(), if !tasks.is_empty() => {
                if let Err(e) = res {
                    log::error!("Обработчик упал: {e}");
                }
            }
        }
    }

    log::info!("Остановка: дожидаюсь текущих загрузок…");
    while tasks.join_next().await.is_some() {}
    updates.sync_update_state().await?;
    handle.quit();
    let _ = pool_task.await;
    log::info!("Бот остановлен");
    Ok(())
}

fn log_features(cfg: &Config) {
    let on = |b: bool| if b { "вкл" } else { "выкл" };
    log::info!(
        "Сортировка по типам: {}, по датам: {}, папки групп: {}, подпись как имя: {}, распаковка ZIP: {}",
        on(cfg.sort_by_type),
        on(cfg.sort_by_date),
        on(cfg.group_folders),
        on(cfg.caption_as_name),
        on(cfg.unzip)
    );
    let model = if recognize::whisper::model_ready(&cfg.whisper_model) { "готова" } else { "не скачана" };
    log::info!(
        "Распознавание речи: {} (модель {}), текста: {}, без дублей: {}, уведомления: {}, резервная копия: {}",
        on(cfg.transcribe),
        model,
        on(cfg.ocr),
        on(cfg.dedup),
        on(cfg.notify),
        cfg.backup_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "выкл".into())
    );
}

async fn handle_message(app: &Arc<App>, msg: &Message) -> Result<(), Error> {
    let chat = msg.peer_id();
    let private = chat.kind() == PeerKind::User;

    // В личке отправитель — сам собеседник; в группе берём автора сообщения
    let sender = msg.sender_id().unwrap_or(chat);
    if sender.kind() != PeerKind::User {
        return Ok(()); // посты от имени каналов и анонимных админов пропускаем
    }
    let user_id = sender.bare_id().unwrap_or_default();

    if !app.cfg.allowed_users.contains(&user_id) {
        if private {
            log::warn!("Отклонено сообщение от пользователя {user_id}");
            msg.reply(format!(
                "⛔ У вас нет доступа.\nВаш Telegram ID: {user_id}\n\
                 Добавьте его в доверенные аккаунты в приложении «Настройки бота», если это вы."
            ))
            .await?;
        }
        return Ok(());
    }

    let Some(media) = msg.media() else {
        if msg.text().starts_with('/') {
            return commands::handle(app, msg, user_id, private).await;
        }
        if private {
            msg.reply("Пришлите файл — я сохраню его в папку. Команды: /help").await?;
        }
        return Ok(());
    };

    let Some(info) = describe(app, msg, &media) else {
        if private {
            msg.reply("Это сообщение не содержит файла, который можно сохранить.").await?;
        }
        return Ok(());
    };

    let group_title = (!private).then(|| msg.peer().and_then(|p| p.name()).unwrap_or("Группа").to_string());

    if let Some(gid) = msg.grouped_id() {
        album_begin(app, gid, msg);
        let outcome = save(app, msg, &media, &info, user_id, group_title.as_deref(), None).await;
        // Текст распознаём до сводки, чтобы прислать его вместе со сводкой по альбому
        let text = match &outcome {
            Outcome::Saved { id, path, kind, .. } => recognize_file(app, *id, *kind, path)
                .await
                .map(|r| (file_name(path), matches!(r, recognize::Recognized::Speech { .. }), r.text().to_string())),
            // Файл уже был: берём текст из истории, а если его нет — распознаём заново
            Outcome::Duplicate(e) => {
                let kind = Kind::from_code(&e.kind);
                let speech = matches!(kind, Kind::Voice | Kind::Audio | Kind::VideoNote);
                match &e.text {
                    Some(t) => Some((e.name.clone(), speech, t.clone())),
                    None => recognize_file(app, e.id, kind, Path::new(&e.path))
                        .await
                        .map(|r| (e.name.clone(), speech, r.text().to_string())),
                }
            }
            Outcome::Failed(_) => None,
        };
        let text = text.map(|(name, speech, text)| AlbumText { order: msg.id(), name, speech, text });
        album_finish(app, gid, outcome, text);
        return Ok(());
    }

    // Одиночный файл: для больших — сообщение с прогрессом
    let notice = if info.size >= PROGRESS_FROM {
        Some(msg.reply(format!("⏳ Скачиваю {} ({})…", info.name, human_size(info.size))).await?)
    } else {
        None
    };
    let outcome = save(app, msg, &media, &info, user_id, group_title.as_deref(), notice.as_ref()).await;

    let text = match &outcome {
        Outcome::Saved { path, size, notes, .. } => {
            let mut t = format!("✅ Сохранено: {} ({})", file_name(path), human_size(*size));
            if let Some(rel) = rel_dir(&app.cfg, path) {
                t += &format!("\n📁 {rel}");
            }
            for n in notes {
                t += &format!("\n{n}");
            }
            t
        }
        Outcome::Duplicate(e) => format!("♻️ Такой файл уже есть: {} (#{})", rel_path(&app.cfg, Path::new(&e.path)), e.id),
        Outcome::Failed(e) => format!("❌ Не удалось сохранить файл: {e}"),
    };
    match &notice {
        Some(n) => n.edit(text).await?,
        None => {
            msg.reply(text).await?;
        }
    }

    // Файл уже был, но текст из него не распознан (например, распознавание прервалось) — пробуем снова
    if let Outcome::Duplicate(e) = &outcome {
        if e.text.is_none() {
            let kind = Kind::from_code(&e.kind);
            let path = PathBuf::from(&e.path);
            if let Some(r) = recognize_file(app, e.id, kind, &path).await {
                reply_recognized(app, msg, &r).await?;
            }
        }
        return Ok(());
    }

    if let Outcome::Saved { id, path, size, kind, .. } = outcome {
        if app.cfg.notify {
            let where_ = group_title.map(|g| format!(" · {g}")).unwrap_or_default();
            notify::file_saved(
                "Файл сохранён".into(),
                format!("{} · {}{where_}", file_name(&path), human_size(size)),
                path.clone(),
            );
        }
        if let Some(r) = recognize_file(app, id, kind, &path).await {
            reply_recognized(app, msg, &r).await?;
        }
    }
    Ok(())
}

async fn reply_recognized(app: &App, msg: &Message, r: &recognize::Recognized) -> Result<(), Error> {
    if app.cfg.reply_text {
        let head = match r {
            recognize::Recognized::Speech { lang, .. } => format!("🗣 Расшифровка ({lang}):"),
            recognize::Recognized::Text(_) => "📝 Распознанный текст:".to_string(),
        };
        msg.reply(format!("{head}\n{}", truncate(r.text(), REPLY_TEXT_LIMIT))).await?;
    }
    Ok(())
}

/// Имя файла, размер и тип вложения
fn describe(app: &App, msg: &Message, media: &Media) -> Option<Incoming> {
    let ts = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let (mut name, size, kind, media_id) = match media {
        Media::Photo(p) => (format!("photo_{ts}.jpg"), p.size().unwrap_or(0) as u64, Kind::Photo, p.id()),
        Media::Document(d) => {
            let (name, kind) = document_name(d, &ts);
            (name, d.size().unwrap_or(0) as u64, kind, d.id())
        }
        _ => return None,
    };
    if app.cfg.caption_as_name {
        if let Some(n) = storage::name_from_caption(msg.text(), &name) {
            name = n;
        }
    }
    Some(Incoming { name: storage::sanitize(&name), size, kind, media_id })
}

fn document_name(d: &Document, ts: &str) -> (String, Kind) {
    let mime = d.mime_type().unwrap_or("");
    if let Some(name) = d.name().filter(|n| !n.trim().is_empty()) {
        let kind = Kind::from_file(name, mime);
        return (name.to_string(), kind);
    }
    let (kind, prefix, ext) = match mime {
        "audio/ogg" => (Kind::Voice, "voice", "ogg"),
        "audio/mpeg" => (Kind::Audio, "audio", "mp3"),
        "audio/mp4" | "audio/x-m4a" => (Kind::Audio, "audio", "m4a"),
        // Видео без имени — обычно кружочки
        "video/mp4" => (Kind::VideoNote, "video_note", "mp4"),
        "video/quicktime" => (Kind::Video, "video", "mov"),
        "image/gif" => (Kind::Photo, "animation", "gif"),
        "image/webp" => (Kind::Photo, "image", "webp"),
        "image/png" => (Kind::Photo, "image", "png"),
        "image/jpeg" => (Kind::Photo, "image", "jpg"),
        "application/pdf" => (Kind::Document, "document", "pdf"),
        "application/zip" => (Kind::Archive, "archive", "zip"),
        _ => {
            let top = mime.split('/').next().filter(|k| !k.is_empty()).unwrap_or("document");
            let ext = mime.split('/').nth(1).unwrap_or("bin");
            let name = format!("{top}_{ts}.{ext}");
            let kind = Kind::from_file(&name, mime);
            return (name, kind);
        }
    };
    (format!("{prefix}_{ts}.{ext}"), kind)
}

/// Скачивает и сохраняет файл со всеми проверками
async fn save(
    app: &App,
    msg: &Message,
    media: &Media,
    info: &Incoming,
    user_id: i64,
    group_title: Option<&str>,
    notice: Option<&Message>,
) -> Outcome {
    let cfg = &app.cfg;

    // 1. Уже присылали этот же файл (тот же объект в Telegram)?
    if cfg.dedup {
        if let Some(e) = app.history.find_media(info.media_id) {
            return Outcome::Duplicate(e);
        }
    }

    // 1б. Тот же файл, отправленный заново (у Telegram он получает новый id):
    // для больших файлов сверяем несколько кусков с файлами того же размера — без полной загрузки
    if cfg.dedup && info.size >= PROGRESS_FROM && !matches!(media, Media::Photo(_)) {
        for e in app.history.with_size(info.size) {
            match download::looks_same(&app.client, media, info.size, Path::new(&e.path)).await {
                Ok(true) => {
                    log::info!("Дубль {} (сверка кусков) — уже есть {}, загрузка не нужна", info.name, e.path);
                    return Outcome::Duplicate(e);
                }
                Ok(false) => {}
                Err(err) => log::warn!("Не удалось сверить {} с {}: {err}", info.name, e.path),
            }
        }
    }

    // 2. Хватит ли места
    let dir = storage::target_dir(cfg, info.kind, group_title);
    if let Some(free) = storage::free_space(&dir) {
        if free < info.size + cfg.min_free_bytes {
            log::warn!("Мало места: свободно {}, файл {}", human_size(free), human_size(info.size));
            return Outcome::Failed(format!(
                "мало места на диске: свободно {}, а должно оставаться не меньше {} после загрузки",
                human_size(free),
                human_size(cfg.min_free_bytes)
            ));
        }
    }
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        return Outcome::Failed(format!("не удалось создать папку: {e}"));
    }

    // 3. Очередь больших загрузок
    let _slot = if info.size >= PROGRESS_FROM {
        match app.big_downloads.try_acquire() {
            Ok(permit) => Some(permit),
            Err(_) => {
                if let Some(n) = notice {
                    let _ = n.edit(format!("⏳ В очереди на загрузку: {} ({})…", info.name, human_size(info.size))).await;
                }
                log::info!("{} ждёт очереди на загрузку", info.name);
                app.big_downloads.acquire().await.ok()
            }
        }
    } else {
        None
    };

    // 4. Загрузка во временный .part с прогрессом (имя занимается атомарно)
    let (path, tmp) = match storage::reserve_path(&dir, &info.name) {
        Ok(r) => r,
        Err(e) => {
            log::error!("Не удалось занять имя для {}: {e}", info.name);
            return Outcome::Failed(format!("не удалось создать файл: {e}"));
        }
    };
    let progress = Arc::new(AtomicU64::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let reporter = notice.map(|n| {
        let (n, progress, finished, total, name) =
            (n.clone(), progress.clone(), finished.clone(), info.size, info.name.clone());
        tokio::spawn(async move {
            let started = Instant::now();
            let mut last = String::new();
            loop {
                tokio::time::sleep(PROGRESS_EVERY).await;
                if finished.load(Ordering::Relaxed) {
                    break;
                }
                let text = format!(
                    "⏳ Скачиваю {name}\n{}",
                    download::progress_text(progress.load(Ordering::Relaxed), total, started.elapsed().as_secs_f64())
                );
                if text != last {
                    let _ = n.edit(text.as_str()).await;
                    last = text;
                }
            }
        })
    });
    let started = Instant::now();
    let result = download::download(&app.client, media, info.size, &tmp, progress).await;
    finished.store(true, Ordering::Relaxed);
    if let Some(r) = reporter {
        r.abort();
    }
    let streamed_sha = match result {
        Ok(h) => h,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            log::error!("Не удалось скачать {}: {e}", info.name);
            return Outcome::Failed(e.to_string());
        }
    };
    if let Err(e) = tokio::fs::rename(&tmp, &path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        log::error!("Не удалось переименовать {} → {}: {e}", tmp.display(), path.display());
        return Outcome::Failed(e.to_string());
    }
    let size = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(info.size);
    let secs = started.elapsed().as_secs_f64().max(0.001);
    log::info!(
        "Сохранён {} ({}, {}/с) от {user_id}{}",
        path.display(),
        human_size(size),
        human_size((size as f64 / secs) as u64),
        group_title.map(|g| format!(" в группе «{g}»")).unwrap_or_default()
    );

    // 4. Такой же по содержимому файл уже есть?
    let sha = match streamed_sha {
        Some(h) => h,
        // Небольшие файлы: посчитать после загрузки дёшево
        None => {
            let p = path.clone();
            tokio::task::spawn_blocking(move || storage::sha256_file(&p)).await.ok().and_then(|r| r.ok()).unwrap_or_default()
        }
    };
    if cfg.dedup && !sha.is_empty() {
        if let Some(e) = app.history.find_hash(&sha, &path) {
            let _ = tokio::fs::remove_file(&path).await;
            log::info!("Дубль {} — уже есть {}", path.display(), e.path);
            return Outcome::Duplicate(e);
        }
    }

    // 5. История
    let id = app.history.add(Entry {
        id: 0,
        time: chrono::Local::now().to_rfc3339(),
        user_id,
        chat_id: msg.peer_id().bot_api_dialog_id().unwrap_or_default(),
        chat: group_title.unwrap_or_default().to_string(),
        kind: info.kind.code().into(),
        name: file_name(&path),
        path: path.to_string_lossy().into(),
        size,
        sha256: sha,
        media_id: info.media_id,
        text: None,
        deleted: false,
    });

    let mut notes = Vec::new();

    // 6. Распаковка ZIP
    if cfg.unzip && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        let free = storage::free_space(&dir).unwrap_or(0);
        let limit = free.saturating_sub(cfg.min_free_bytes).min(50 << 30);
        let p = path.clone();
        match tokio::task::spawn_blocking(move || archive::unzip(&p, limit)).await {
            Ok(Ok(u)) => {
                notes.push(format!("📦 Распаковано {} файлов ({}) в «{}»", u.files, human_size(u.bytes), file_name(&u.dir)));
                log::info!("Распакован {} → {}", path.display(), u.dir.display());
            }
            Ok(Err(e)) => notes.push(format!("⚠️ Не удалось распаковать: {e}")),
            Err(e) => notes.push(format!("⚠️ Не удалось распаковать: {e}")),
        }
    }

    // 7. Резервная копия
    if cfg.backup_dir.is_some() {
        match storage::backup(cfg, &path).await {
            Ok(dest) => log::info!("Резервная копия: {}", dest.display()),
            Err(e) => {
                log::warn!("Резервная копия не создана: {e}");
                notes.push(format!("⚠️ Резервная копия не создана: {e}"));
            }
        }
    }

    Outcome::Saved { id, path, size, kind: info.kind, notes }
}

/// Распознаёт речь или текст, сохраняет в историю и (по настройке) в .txt рядом с файлом
async fn recognize_file(app: &Arc<App>, id: u64, kind: Kind, path: &Path) -> Option<recognize::Recognized> {
    if !recognize::applicable(&app.cfg, kind, path) {
        return None;
    }
    // Очередь: речь — по одной, картинки — до OCR_PARALLEL сразу
    let speech = matches!(kind, Kind::Voice | Kind::Audio | Kind::VideoNote);
    let _speech_turn = if speech { Some(app.speech_queue.lock().await) } else { None };
    let _ocr_slot = if speech { None } else { app.ocr_slots.acquire().await.ok() };
    let (app2, p) = (Arc::clone(app), path.to_path_buf());
    let result = tokio::task::spawn_blocking(move || recognize::run(&app2.cfg, kind, &p)).await;
    let rec = match result {
        Ok(Ok(Some(r))) => r,
        Ok(Ok(None)) => return None,
        Ok(Err(e)) => {
            log::warn!("Распознавание {}: {e}", path.display());
            return None;
        }
        Err(e) => {
            log::error!("Распознавание упало: {e}");
            return None;
        }
    };
    let text = rec.text().to_string();
    app.history.update(id, |e| e.text = Some(text.clone()));
    if app.cfg.save_text_files {
        let dir = path.parent().unwrap_or(Path::new("."));
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        match storage::reserve_path(dir, &format!("{stem}.txt")) {
            Ok((txt, tmp)) => {
                let res = match tokio::fs::write(&tmp, &text).await {
                    Ok(()) => tokio::fs::rename(&tmp, &txt).await,
                    Err(e) => Err(e),
                };
                if let Err(e) = res {
                    let _ = tokio::fs::remove_file(&tmp).await;
                    log::warn!("Не удалось записать {}: {e}", txt.display());
                }
            }
            Err(e) => log::warn!("Не удалось записать текст для {}: {e}", path.display()),
        }
    }
    log::info!("Распознан текст для {} ({} символов)", path.display(), text.chars().count());
    Some(rec)
}

// ---------- Альбомы: один ответ на всю пачку ----------

fn album_begin(app: &App, gid: i64, msg: &Message) {
    let mut albums = app.albums.lock().unwrap();
    let a = albums.entry(gid).or_insert_with(|| Album {
        first: msg.clone(),
        pending: 0,
        generation: 0,
        saved: Vec::new(),
        duplicates: 0,
        failed: Vec::new(),
        texts: Vec::new(),
    });
    a.pending += 1;
    a.generation += 1;
}

fn album_finish(app: &Arc<App>, gid: i64, outcome: Outcome, text: Option<AlbumText>) {
    let generation = {
        let mut albums = app.albums.lock().unwrap();
        let Some(a) = albums.get_mut(&gid) else { return };
        a.texts.extend(text);
        match outcome {
            Outcome::Saved { path, size, notes, .. } => {
                a.saved.push((path, size));
                a.failed.extend(notes.into_iter().filter(|n| n.starts_with("⚠️")));
            }
            Outcome::Duplicate(_) => a.duplicates += 1,
            Outcome::Failed(e) => a.failed.push(e),
        }
        a.pending -= 1;
        if a.pending > 0 {
            return;
        }
        a.generation
    };
    let app = Arc::clone(app);
    tokio::spawn(async move {
        tokio::time::sleep(ALBUM_WAIT).await;
        let album = {
            let mut albums = app.albums.lock().unwrap();
            match albums.get(&gid) {
                Some(a) if a.pending == 0 && a.generation == generation => albums.remove(&gid),
                _ => None,
            }
        };
        let Some(mut a) = album else { return };
        let total: u64 = a.saved.iter().map(|(_, s)| s).sum();
        let mut text = format!("✅ Сохранено файлов: {} ({})", a.saved.len(), human_size(total));
        if let Some((p, _)) = a.saved.first() {
            if let Some(rel) = rel_dir(&app.cfg, p) {
                text += &format!("\n📁 {rel}");
            }
        }
        if a.duplicates > 0 {
            text += &format!("\n♻️ Уже были сохранены раньше: {}", a.duplicates);
        }
        for f in &a.failed {
            text += &format!("\n❌ {f}");
        }
        if let Err(e) = a.first.reply(text).await {
            log::error!("Не удалось ответить на альбом: {e}");
        }
        if app.cfg.reply_text && !a.texts.is_empty() {
            a.texts.sort_by_key(|t| t.order);
            let mut parts = album_text_messages(&a.texts);
            // Не заваливаем чат: максимум MAX_TEXT_MESSAGES сообщений с текстом на альбом
            if parts.len() > MAX_TEXT_MESSAGES {
                let rest = parts.len() - MAX_TEXT_MESSAGES;
                parts.truncate(MAX_TEXT_MESSAGES);
                parts.push(format!(
                    "…и ещё {rest} сообщ. текста. Полный текст — в .txt рядом с файлами, поиск по нему: /find"
                ));
            }
            for part in parts {
                if let Err(e) = a.first.reply(part).await {
                    log::error!("Не удалось отправить распознанный текст альбома: {e}");
                    break;
                }
            }
        }
        if app.cfg.notify && !a.saved.is_empty() {
            notify::file_saved(
                "Файлы сохранены".into(),
                format!("{} файлов · {}", a.saved.len(), human_size(total)),
                a.saved[0].0.clone(),
            );
        }
    });
}

/// Тексты всех файлов альбома одним или несколькими сообщениями (с учётом лимита Telegram)
fn album_text_messages(texts: &[AlbumText]) -> Vec<String> {
    let mut messages = Vec::new();
    let mut current = String::new();
    for t in texts {
        let icon = if t.speech { "🗣" } else { "📄" };
        // Каждый файл — не длиннее лимита одного сообщения
        let body = truncate(t.text.trim(), MESSAGE_LIMIT - 200);
        let section = format!("{icon} {}\n{body}", t.name);
        let sep = if current.is_empty() { "" } else { "\n\n" };
        if current.chars().count() + sep.len() + section.chars().count() > MESSAGE_LIMIT && !current.is_empty() {
            messages.push(std::mem::take(&mut current));
            current = section;
        } else {
            current.push_str(sep);
            current.push_str(&section);
        }
    }
    if !current.is_empty() {
        messages.push(current);
    }
    messages
}

// ---------- Мелочи ----------

pub fn file_name(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into()
}

/// Путь относительно папки сохранения (для ответов)
pub fn rel_path(cfg: &Config, p: &Path) -> String {
    p.strip_prefix(&cfg.save_dir).unwrap_or(p).to_string_lossy().into()
}

fn rel_dir(cfg: &Config, p: &Path) -> Option<String> {
    let rel = p.strip_prefix(&cfg.save_dir).ok()?.parent()?;
    let s = rel.to_string_lossy();
    (!s.is_empty()).then(|| s.into())
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

/// Сессия — это ключ доступа к боту, читать её может только владелец
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(order: i32, len: usize) -> AlbumText {
        AlbumText { order, name: format!("f{order}.pdf"), speech: false, text: "я".repeat(len) }
    }

    #[test]
    fn album_texts_fit_telegram_limit() {
        let texts = vec![item(1, 100), item(2, 3000), item(3, 9000), item(4, 50)];
        let msgs = album_text_messages(&texts);
        assert!(msgs.iter().all(|m| m.chars().count() <= 4096), "каждое сообщение в лимите");
        let all = msgs.join("\n");
        for i in 1..=4 {
            assert!(all.contains(&format!("f{i}.pdf")), "есть текст каждого файла");
        }
        assert!(msgs.len() >= 2);
    }

    #[test]
    fn small_album_texts_in_one_message() {
        let msgs = album_text_messages(&[item(1, 10), item(2, 10)]);
        assert_eq!(msgs.len(), 1);
    }
}
