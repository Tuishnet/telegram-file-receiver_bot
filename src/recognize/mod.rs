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
    Speech { text: String, lang: String },
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
        Kind::Photo => cfg.ocr && matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp"),
        Kind::Document => cfg.ocr && ext == "pdf",
        _ => false,
    }
}

/// Блокирующая функция — вызывать из `spawn_blocking`
pub fn run(cfg: &Config, kind: Kind, path: &Path) -> Result<Option<Recognized>, String> {
    match kind {
        Kind::Voice | Kind::Audio | Kind::VideoNote => {
            if !whisper::model_ready(&cfg.whisper_model) {
                return Err("модель распознавания речи не скачана — скачайте её в приложении «Настройки бота»".into());
            }
            let pcm = audio::decode_to_16k_mono(path, MAX_AUDIO_SECONDS)?;
            let started = std::time::Instant::now();
            let (text, lang) = whisper::transcribe(&cfg.whisper_model, &pcm, &cfg.whisper_lang, cfg.whisper_gpu)?;
            log::info!(
                "Речь распознана за {:.1} с ({:.0} с аудио)",
                started.elapsed().as_secs_f32(),
                pcm.len() as f32 / audio::TARGET_RATE as f32
            );
            Ok((!text.trim().is_empty()).then_some(Recognized::Speech { text, lang }))
        }
        Kind::Photo => {
            let t = ocr::image_text(path)?;
            Ok((!t.trim().is_empty()).then_some(Recognized::Text(t)))
        }
        Kind::Document => {
            let t = ocr::pdf_text(path)?;
            Ok((!t.trim().is_empty()).then_some(Recognized::Text(t)))
        }
        _ => Ok(None),
    }
}
