//! Команды бота: /list /find /get /delete /disk /help.

use std::path::Path;
use std::sync::Arc;

use grammers_client::Client;
use grammers_client::message::{InputMessage, Message};
use grammers_client::tl;

use crate::history::Entry;
use crate::storage::{self, human_size};
use crate::{App, Error};

const COMMANDS: &[(&str, &str)] = &[
    ("list", "Последние сохранённые файлы"),
    ("find", "Поиск по имени и распознанному тексту"),
    ("get", "Прислать файл с Мака: /get 12 или /get отчёт"),
    ("delete", "Удалить файл в Корзину: /delete или /delete 12"),
    ("disk", "Свободное место на диске"),
    ("help", "Что умеет бот"),
];

/// Меню команд в Telegram (кнопка «/» в поле ввода)
pub async fn register(client: &Client) -> Result<(), Error> {
    let commands = COMMANDS
        .iter()
        .map(|(c, d)| tl::enums::BotCommand::Command(tl::types::BotCommand { ephemeral: false, command: c.to_string(), description: d.to_string() }))
        .collect();
    client
        .invoke(&tl::functions::bots::SetBotCommands {
            scope: tl::enums::BotCommandScope::Default,
            lang_code: String::new(),
            commands,
        })
        .await?;
    Ok(())
}

pub async fn handle(app: &Arc<App>, msg: &Message, user_id: i64, private: bool) -> Result<(), Error> {
    let text = msg.text().trim();
    let (head, arg) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let arg = arg.trim();
    // В группах команда может быть вида /list@имя_бота
    let cmd = head.trim_start_matches('/').split('@').next().unwrap_or("").to_lowercase();

    let reply = match cmd.as_str() {
        "start" | "help" => help(),
        "list" => list(app, arg),
        "find" => find(app, arg),
        "get" => return get(app, msg, arg).await,
        "delete" => delete(app, user_id, arg).await,
        "disk" => disk(app),
        _ if private => "Неизвестная команда. Список команд: /help".into(),
        _ => return Ok(()),
    };
    msg.reply(reply).await?;
    Ok(())
}

fn help() -> String {
    "👋 Пришлите мне файл, фото, видео, аудио или голосовое — я сохраню его на Мак (до 2 ГБ).\n\
     Подпись к файлу станет его именем. Голосовые расшифрую, текст с фото и PDF распознаю.\n\n\
     Команды:\n\
     /list — последние файлы\n\
     /find текст — поиск по имени и содержимому\n\
     /get 12 — прислать файл №12 (или /get часть имени)\n\
     /delete — удалить последний файл в Корзину (или /delete 12)\n\
     /disk — свободное место"
        .into()
}

fn line(_app: &App, e: &Entry) -> String {
    let time = chrono::DateTime::parse_from_rfc3339(&e.time).map(|t| t.format("%d.%m %H:%M").to_string()).unwrap_or_default();
    format!("#{} {} · {} · {time}", e.id, e.name, human_size(e.size))
}

fn list(app: &App, arg: &str) -> String {
    let n = arg.parse().unwrap_or(10).clamp(1, 50);
    let items = app.history.recent(n);
    if items.is_empty() {
        return "Пока нет сохранённых файлов.".into();
    }
    let mut s = "🗂 Последние файлы:\n".to_string();
    for e in &items {
        s += &line(app, e);
        s.push('\n');
    }
    s + "\nПрислать файл: /get номер"
}

fn find(app: &App, arg: &str) -> String {
    if arg.is_empty() {
        return "Напишите, что искать: /find договор".into();
    }
    let items = app.history.search(arg, 20);
    if items.is_empty() {
        return format!("Ничего не нашлось по запросу «{arg}».");
    }
    let mut s = format!("🔎 Найдено по «{arg}»:\n");
    let q = arg.to_lowercase();
    for e in &items {
        s += &line(app, e);
        // Показываем кусочек текста, где нашлось совпадение
        if !e.name.to_lowercase().contains(&q) {
            if let Some(t) = &e.text {
                if let Some(pos) = t.to_lowercase().find(&q) {
                    let start = t[..pos].char_indices().rev().nth(30).map(|(i, _)| i).unwrap_or(0);
                    let snippet: String = t[start..].chars().take(80).collect();
                    s += &format!("\n   «…{}…»", snippet.replace('\n', " "));
                }
            }
        }
        s.push('\n');
    }
    s
}

fn pick(app: &App, arg: &str) -> Result<Entry, String> {
    let id_arg = arg.trim_start_matches('#');
    if let Ok(id) = id_arg.parse::<u64>() {
        return app.history.get(id).filter(|e| e.exists()).ok_or(format!("Файла #{id} нет (или он удалён)."));
    }
    let found = app.history.search(arg, 10);
    match found.len() {
        0 => Err(format!("Ничего не нашлось по «{arg}».")),
        1 => Ok(found.into_iter().next().unwrap()),
        _ => {
            let mut s = "Нашлось несколько файлов, уточните номер:\n".to_string();
            for e in &found {
                s += &line(app, e);
                s.push('\n');
            }
            Err(s)
        }
    }
}

/// Отправляет файл с Мака (только из истории бота — никаких произвольных путей)
async fn get(app: &App, msg: &Message, arg: &str) -> Result<(), Error> {
    if arg.is_empty() {
        msg.reply("Укажите номер или часть имени: /get 12. Номера — в /list").await?;
        return Ok(());
    }
    let entry = match pick(app, arg) {
        Ok(e) => e,
        Err(text) => {
            msg.reply(text).await?;
            return Ok(());
        }
    };
    let path = Path::new(&entry.path);
    let inside = match (path.canonicalize(), app.cfg.save_dir.canonicalize()) {
        (Ok(p), Ok(d)) => p.starts_with(d),
        _ => false,
    };
    if !inside {
        msg.reply("Этот файл лежит вне папки бота — отправлять его не буду.").await?;
        return Ok(());
    }
    let notice = if entry.size > 10 << 20 {
        Some(msg.reply(format!("⏫ Отправляю {} ({})…", entry.name, human_size(entry.size))).await?)
    } else {
        None
    };
    let uploaded = app.client.upload_file(path).await?;
    msg.reply(InputMessage::new().text(format!("#{} {}", entry.id, entry.name)).document(uploaded)).await?;
    if let Some(n) = notice {
        let _ = n.delete().await;
    }
    log::info!("Отправлен файл {}", entry.path);
    Ok(())
}

/// Перемещает файл в Корзину macOS (его можно восстановить)
async fn delete(app: &App, user_id: i64, arg: &str) -> String {
    let entry = if arg.is_empty() {
        match app.history.last_by_user(user_id) {
            Some(e) => e,
            None => return "Нечего удалять.".into(),
        }
    } else {
        match pick(app, arg) {
            Ok(e) => e,
            Err(t) => return t,
        }
    };
    let path = entry.path.clone();
    match tokio::task::spawn_blocking(move || trash::delete(&path)).await {
        Ok(Ok(())) => {
            app.history.update(entry.id, |e| e.deleted = true);
            log::info!("Файл {} перемещён в Корзину", entry.path);
            format!("🗑 {} (#{}) перемещён в Корзину.", entry.name, entry.id)
        }
        Ok(Err(e)) => format!("Не удалось удалить: {e}"),
        Err(e) => format!("Не удалось удалить: {e}"),
    }
}

fn disk(app: &App) -> String {
    let free = storage::free_space(&app.cfg.save_dir);
    let (count, total) = app.history.totals();
    let mut s = match storage::ensure_dir(&app.cfg.save_dir) {
        Ok(()) => String::from("✅ Папка для сохранения доступна\n"),
        Err(e) => format!("❌ Папка для сохранения недоступна: {e}\n"),
    };
    s += "💽 ";
    match free {
        Some(f) => s += &format!("Свободно на диске: {}", human_size(f)),
        None => s += "Не удалось узнать свободное место",
    }
    s += &format!(
        "\nОставляю свободными минимум: {}\nСохранено ботом: {} файлов, {}\nПапка: {}",
        human_size(app.cfg.min_free_bytes),
        count,
        human_size(total),
        app.cfg.save_dir.display()
    );
    s
}
