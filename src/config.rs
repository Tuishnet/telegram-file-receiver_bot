//! Настройки бота из `.env` (их редактирует приложение «Настройки бота»).

use std::collections::HashSet;
use std::path::PathBuf;

/// Модель распознавания речи (Whisper Small); путь относительно папки проекта
pub const WHISPER_MODEL_DIR: &str = "models/whisper-small";

pub struct Config {
    pub token: String,
    pub api_id: i32,
    pub api_hash: String,
    pub allowed_users: HashSet<i64>,

    // Файлы
    pub save_dir: PathBuf,
    pub sort_by_type: bool,
    pub sort_by_date: bool,
    pub group_folders: bool,
    pub caption_as_name: bool,
    pub unzip: bool,
    pub backup_dir: Option<PathBuf>,

    // Защита
    pub min_free_bytes: u64,
    pub dedup: bool,

    // Распознавание
    pub transcribe: bool,
    pub whisper_model: PathBuf,
    pub whisper_lang: String,
    pub whisper_gpu: bool,
    pub whisper_unload_minutes: u64,
    pub transcribe_video: bool,
    pub video_max_minutes: u32,
    pub ocr: bool,
    pub save_text_files: bool,
    pub reply_text: bool,

    // Прочее
    pub notify: bool,
    pub log_max_bytes: u64,
}

fn var(k: &str) -> String {
    std::env::var(k).unwrap_or_default().trim().to_string()
}

fn flag(k: &str, default: bool) -> bool {
    match var(k).to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

fn number(k: &str, default: f64) -> f64 {
    var(k).replace(',', ".").parse().unwrap_or(default)
}

fn expand(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let token = var("TELOXIDE_TOKEN");
        if token.is_empty() {
            return Err("Не задан токен бота (TELOXIDE_TOKEN)".into());
        }
        let api_id_raw = var("API_ID");
        if api_id_raw.is_empty() {
            return Err("Не задан API_ID — получите его на my.telegram.org → API development tools".into());
        }
        let api_id = api_id_raw
            .parse()
            .map_err(|_| format!("API_ID должен быть числом, а сейчас: «{api_id_raw}»"))?;
        let api_hash = var("API_HASH");
        if api_hash.is_empty() {
            return Err("Не задан API_HASH — получите его на my.telegram.org → API development tools".into());
        }

        let save_dir = var("SAVE_DIR");
        let save_dir = expand(if save_dir.is_empty() { "files" } else { &save_dir });
        let backup = var("BACKUP_DIR");
        let lang = var("WHISPER_LANG");

        Ok(Config {
            token,
            api_id,
            api_hash,
            allowed_users: var("ALLOWED_USERS")
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect(),
            save_dir,
            sort_by_type: flag("SORT_BY_TYPE", true),
            sort_by_date: flag("SORT_BY_DATE", false),
            group_folders: flag("GROUP_FOLDERS", true),
            caption_as_name: flag("CAPTION_AS_NAME", true),
            unzip: flag("UNZIP", false),
            backup_dir: (!backup.is_empty()).then(|| expand(&backup)),
            min_free_bytes: (number("MIN_FREE_GB", 5.0).max(0.0) * 1024.0 * 1024.0 * 1024.0) as u64,
            dedup: flag("DEDUP", true),
            transcribe: flag("TRANSCRIBE", true),
            whisper_model: PathBuf::from(WHISPER_MODEL_DIR),
            whisper_lang: if lang.is_empty() { "auto".into() } else { lang },
            whisper_gpu: flag("WHISPER_GPU", true),
            whisper_unload_minutes: number("WHISPER_UNLOAD_MIN", 10.0).max(0.0) as u64,
            transcribe_video: flag("TRANSCRIBE_VIDEO", true),
            video_max_minutes: number("VIDEO_MAX_MIN", 60.0).clamp(1.0, 240.0) as u32,
            ocr: flag("OCR", true),
            save_text_files: flag("SAVE_TEXT_FILES", true),
            reply_text: flag("REPLY_TEXT", true),
            notify: flag("NOTIFY", true),
            log_max_bytes: (number("LOG_MAX_MB", 10.0).max(1.0) * 1024.0 * 1024.0) as u64,
        })
    }
}
