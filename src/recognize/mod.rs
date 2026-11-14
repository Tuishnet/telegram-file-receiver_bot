//! Распознавание: речь (Whisper) и текст на картинках/в PDF (macOS Vision).

pub mod audio;
pub mod ocr;
pub mod whisper;
mod whisper_model;

use std::path::Path;

use crate::config::Config;
use crate::storage::Kind;

/// Максимальная длина аудио для распознавания
const MAX_AUDIO_SECONDS: u32 = 2 * 60 * 60;

pub enum Recognized {
    /// `truncated` — расшифрована только первая часть (запись длиннее лимита)
    Speech { text: String, lang: String, truncated_at_min: Option<u32> },
    Text(String),
}

impl Recognized {
    pub fn text(&self) -> &str {
        match self {
            Recognized::Speech { text, .. } | Recognized::Text(text) => text,
        }
    }
}

/// Что можно распознать в файле такого типа (с учётом настроек)
pub fn applicable(cfg: &Config, kind: Kind, path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match kind {
        Kind::Voice | Kind::Audio | Kind::VideoNote => cfg.transcribe,
        Kind::Video => cfg.transcribe && cfg.transcribe_video,
        Kind::Photo => cfg.ocr && matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp"),
        Kind::Document => cfg.ocr && ext == "pdf",
        _ => false,
    }
}

/// Блокирующая функция — вызывать из `spawn_blocking`
pub fn run(cfg: &Config, kind: Kind, path: &Path) -> Result<Option<Recognized>, String> {
    match kind {
        Kind::Voice | Kind::Audio | Kind::VideoNote | Kind::Video => {
            if !whisper::model_ready(&cfg.whisper_model) {
                return Err("модель распознавания речи не скачана — скачайте её в приложении «Настройки бота»".into());
            }
            let max_seconds = if kind == Kind::Video { cfg.video_max_minutes * 60 } else { MAX_AUDIO_SECONDS };
            let pcm = match audio::decode_to_16k_mono(path, max_seconds) {
                Ok(p) => p,
                // У видео может не быть звука или он в неподдерживаемом кодеке — это не ошибка бота
                Err(e) if kind == Kind::Video => {
                    log::info!("Видео {} не расшифровано: {e}", path.display());
                    return Ok(None);
                }
                Err(e) => return Err(e),
            };
            let seconds = pcm.len() as f32 / audio::TARGET_RATE as f32;
            let truncated_at_min = (seconds >= max_seconds as f32 - 1.0).then_some(max_seconds / 60);
            let started = std::time::Instant::now();
            let (text, lang) = whisper::transcribe(&cfg.whisper_model, &pcm, &cfg.whisper_lang, cfg.whisper_gpu)?;
            log::info!("Речь распознана за {:.1} с ({seconds:.0} с аудио)", started.elapsed().as_secs_f32());
            Ok((!text.trim().is_empty()).then_some(Recognized::Speech { text, lang, truncated_at_min }))
        }
        Kind::Photo | Kind::Document => {
            let started = std::time::Instant::now();
            let t = if kind == Kind::Photo { ocr::image_text(path)? } else { ocr::pdf_text(path)? };
            log::info!("Текст распознан за {:.1} с ({} символов)", started.elapsed().as_secs_f32(), t.chars().count());
            Ok((!t.trim().is_empty()).then_some(Recognized::Text(t)))
        }
        _ => Ok(None),
    }
}
