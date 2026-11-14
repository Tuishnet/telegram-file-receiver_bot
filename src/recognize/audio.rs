//! Декодирование аудио (Opus, AAC, MP3, FLAC, …) в моно 16 кГц для Whisper. Чистый Rust.

use std::path::Path;
use std::sync::OnceLock;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::common::Tier;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

pub const TARGET_RATE: u32 = 16_000;

fn codecs() -> &'static CodecRegistry {
    static REG: OnceLock<CodecRegistry> = OnceLock::new();
    REG.get_or_init(|| {
        let mut r = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut r);
        // Opus (голосовые Telegram) — отдельным чисто-Rust декодером
        r.register_audio_decoder_at_tier::<symphonia_opus::OpusDecoder>(Tier::Fallback);
        r
    })
}

/// Возвращает моно-сэмплы 16 кГц. `max_seconds` — ограничение длины.
pub fn decode_to_16k_mono(path: &Path, max_seconds: u32) -> Result<Vec<f32>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("неизвестный формат аудио: {e}"))?;
    let track = format.default_track(TrackType::Audio).ok_or("в файле нет звуковой дорожки")?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("нет параметров аудио")?
        .clone();
    let mut decoder = codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| format!("кодек не поддерживается: {e}"))?;

    let mut mono: Vec<f32> = Vec::new();
    let mut rate = params.sample_rate.unwrap_or(48_000);
    let mut interleaved: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(format!("ошибка чтения аудио: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = buf.spec();
                rate = spec.rate();
                let ch = spec.channels().count().max(1);
                interleaved.resize(buf.samples_interleaved(), 0.0);
                buf.copy_to_slice_interleaved(&mut interleaved);
                mono.extend(interleaved.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32));
                if mono.len() as u64 > max_seconds as u64 * rate as u64 {
                    break;
                }
            }
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(format!("ошибка декодирования: {e}")),
        }
    }
    if mono.is_empty() {
        return Err("звук не найден".into());
    }
    Ok(resample(&mono, rate, TARGET_RATE))
}

/// Передискретизация с простым сглаживающим фильтром (для речи достаточно)
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    // Усреднение по окну ≈ ratio — защищает от наложения частот при понижении
    let win = ratio.ceil().max(1.0) as usize;
    let out_len = (input.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let start = (i as f64 * ratio) as usize;
            let end = (start + win).min(input.len());
            let s = &input[start.min(input.len() - 1)..end.max(start + 1).min(input.len())];
            s.iter().sum::<f32>() / s.len().max(1) as f32
        })
        .collect()
}
