use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{FileId, FileMeta, ReplyParameters};
use tokio::fs;

/// Telegram Bot API (облачный) отдаёт ботам файлы размером до 20 МБ
const MAX_DOWNLOAD_SIZE: u32 = 20 * 1024 * 1024;

struct Config {
    save_dir: PathBuf,
    allowed_users: HashSet<u64>,
}

/// Что именно прислали: метаданные файла + имя, под которым сохранить
struct Incoming {
    meta: FileMeta,
    name: String,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    if std::env::var("RUST_LOG").is_err() {
        // SAFETY: вызывается до запуска других потоков
        unsafe { std::env::set_var("RUST_LOG", "info") };
    }
    pretty_env_logger::init();

    let save_dir = PathBuf::from(std::env::var("SAVE_DIR").unwrap_or_else(|_| "files".into()));
    if let Err(e) = fs::create_dir_all(&save_dir).await {
        log::error!("Не удалось создать папку {}: {e}", save_dir.display());
        return;
    }

    let allowed_users: HashSet<u64> = std::env::var("ALLOWED_USERS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if allowed_users.is_empty() {
        log::warn!(
            "ALLOWED_USERS не задан: бот не будет сохранять файлы, \
             а только сообщит каждому его Telegram ID"
        );
    }

    log::info!("Файлы сохраняются в {}", save_dir.display());
    let config = Arc::new(Config { save_dir, allowed_users });

    // Токен берётся из переменной TELOXIDE_TOKEN
    let bot = Bot::from_env();

    let handler = Update::filter_message().endpoint(handle_message);
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![config])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

async fn handle_message(bot: Bot, msg: Message, config: Arc<Config>) -> ResponseResult<()> {
    let Some(user) = msg.from.as_ref() else {
        return Ok(());
    };

    if !config.allowed_users.contains(&user.id.0) {
        log::warn!("Отклонено сообщение от пользователя {}", user.id);
        bot.send_message(
            msg.chat.id,
            format!(
                "⛔ У вас нет доступа.\nВаш Telegram ID: {}\n\
                 Добавьте его в ALLOWED_USERS в файле .env, если это вы.",
                user.id
            ),
        )
        .await?;
        return Ok(());
    }

    let Some(incoming) = extract_file(&msg) else {
        if msg.text().is_some_and(|t| t.starts_with("/start")) {
            bot.send_message(
                msg.chat.id,
                "👋 Пришлите мне файл, фото, видео, аудио или голосовое — я сохраню его.",
            )
            .await?;
        } else {
            bot.send_message(msg.chat.id, "Пришлите файл — я сохраню его в папку.")
                .await?;
        }
        return Ok(());
    };

    if incoming.meta.size > MAX_DOWNLOAD_SIZE {
        bot.send_message(
            msg.chat.id,
            format!(
                "⚠️ Файл слишком большой ({}). Telegram позволяет ботам скачивать файлы до 20 МБ.",
                human_size(incoming.meta.size)
            ),
        )
        .await?;
        return Ok(());
    }

    match save_file(&bot, &incoming, &config.save_dir).await {
        Ok(path) => {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            log::info!("Сохранён {} от {}", path.display(), user.id);
            bot.send_message(
                msg.chat.id,
                format!("✅ Сохранено: {name} ({})", human_size(incoming.meta.size)),
            )
            .reply_parameters(ReplyParameters::new(msg.id))
            .await?;
        }
        Err(e) => {
            log::error!("Ошибка сохранения: {e}");
            bot.send_message(msg.chat.id, format!("❌ Не удалось сохранить файл: {e}"))
                .await?;
        }
    }
    Ok(())
}

/// Достаёт файл из сообщения любого поддерживаемого типа
fn extract_file(msg: &Message) -> Option<Incoming> {
    let ts = msg.date.format("%Y-%m-%d_%H-%M-%S");

    if let Some(d) = msg.document() {
        let name = d.file_name.clone().unwrap_or_else(|| format!("document_{ts}"));
        return Some(Incoming { meta: d.file.clone(), name });
    }
    if let Some(sizes) = msg.photo() {
        // Последний элемент — фото в наибольшем разрешении
        let p = sizes.last()?;
        return Some(Incoming { meta: p.file.clone(), name: format!("photo_{ts}.jpg") });
    }
    if let Some(v) = msg.video() {
        let name = v.file_name.clone().unwrap_or_else(|| format!("video_{ts}.mp4"));
        return Some(Incoming { meta: v.file.clone(), name });
    }
    if let Some(a) = msg.audio() {
        let name = a.file_name.clone().unwrap_or_else(|| format!("audio_{ts}.mp3"));
        return Some(Incoming { meta: a.file.clone(), name });
    }
    if let Some(a) = msg.animation() {
        let name = a.file_name.clone().unwrap_or_else(|| format!("animation_{ts}.mp4"));
        return Some(Incoming { meta: a.file.clone(), name });
    }
    if let Some(v) = msg.voice() {
        return Some(Incoming { meta: v.file.clone(), name: format!("voice_{ts}.ogg") });
    }
    if let Some(v) = msg.video_note() {
        return Some(Incoming { meta: v.file.clone(), name: format!("video_note_{ts}.mp4") });
    }
    None
}

async fn save_file(bot: &Bot, incoming: &Incoming, dir: &Path) -> anyhow_lite::Result<PathBuf> {
    let file = bot.get_file(FileId(incoming.meta.id.0.clone())).await?;

    let path = unique_path(dir, &sanitize(&incoming.name)).await;
    // Пишем во временный файл и переименовываем, чтобы не оставлять обрывков при ошибке
    let tmp = path.with_extension("part");
    let mut dst = fs::File::create(&tmp).await?;
    if let Err(e) = bot.download_file(&file.path, &mut dst).await {
        drop(dst);
        let _ = fs::remove_file(&tmp).await;
        return Err(e.into());
    }
    dst.sync_all().await?;
    drop(dst);
    fs::rename(&tmp, &path).await?;
    Ok(path)
}

/// Убирает из имени разделители путей и прочие опасные символы
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() { "file".into() } else { cleaned }
}

/// Если файл с таким именем уже есть — добавляет (1), (2), ...
async fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !fs::try_exists(&candidate).await.unwrap_or(false) {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for i in 1.. {
        let candidate = dir.join(format!("{stem} ({i}){ext}"));
        if !fs::try_exists(&candidate).await.unwrap_or(false) {
            return candidate;
        }
    }
    unreachable!()
}

fn human_size(bytes: u32) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 {
        format!("{:.1} МБ", b / 1024.0 / 1024.0)
    } else if b >= 1024.0 {
        format!("{:.1} КБ", b / 1024.0)
    } else {
        format!("{bytes} Б")
    }
}

/// Мини-обёртка для ошибок, чтобы не тянуть лишние зависимости
mod anyhow_lite {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
}
