//! Распознавание речи моделью Whisper на candle (ML-фреймворк на Rust).
//! Модель (config.json, tokenizer.json, model.safetensors) скачивается приложением «Настройки бота».

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use candle_core::{D, DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self as m, Config, audio};

use super::whisper_model::Whisper;
use tokenizers::Tokenizer;

/// Коды языков Whisper в порядке токенов
const LANGUAGES: &[&str] = &[
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi",
    "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la",
    "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy",
    "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be",
    "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl",
    "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su", "yue",
];

/// Максимум токенов на 30-секундный отрезок
const MAX_TOKENS: usize = 224;

struct Engine {
    dir: PathBuf,
    gpu: bool,
    dtype: DType,
    last_used: Instant,
    model: Whisper,
    tokenizer: Tokenizer,
    config: Config,
    filters: Vec<f32>,
    device: Device,
    sot: u32,
    eot: u32,
    transcribe: u32,
    no_timestamps: u32,
    suppress: Tensor,
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

pub fn model_ready(dir: &Path) -> bool {
    ["config.json", "tokenizer.json", "model.safetensors"].iter().all(|f| dir.join(f).exists())
}

/// Распознаёт речь. Блокирующая функция — вызывать из `spawn_blocking`.
/// `gpu` — считать на видеокарте (Metal) в половинной точности. Возвращает (текст, язык).
pub fn transcribe(dir: &Path, pcm: &[f32], lang: &str, gpu: bool) -> Result<(String, String), String> {
    let mut guard = ENGINE.lock().unwrap();
    if guard.as_ref().is_none_or(|e| e.dir != dir || e.gpu != gpu) {
        *guard = None; // сначала освобождаем память старой модели
        let started = Instant::now();
        let engine = Engine::load(dir, gpu).map_err(|e| format!("не удалось загрузить модель: {e}"))?;
        log::info!(
            "Модель речи загружена за {:.1} с ({}, {:?})",
            started.elapsed().as_secs_f32(),
            if engine.device.is_metal() { "видеокарта" } else { "процессор" },
            engine.dtype
        );
        *guard = Some(engine);
    }
    let engine = guard.as_mut().unwrap();
    engine.last_used = Instant::now();
    let result = engine.run(pcm, lang).map_err(|e| e.to_string());
    engine.last_used = Instant::now();
    result
}

/// Выгружает модель из памяти, если ей не пользовались дольше `idle`
pub fn unload_if_idle(idle: Duration) {
    // try_lock: если сейчас идёт распознавание — модель точно нужна
    if let Ok(mut guard) = ENGINE.try_lock() {
        if guard.as_ref().is_some_and(|e| e.last_used.elapsed() >= idle) {
            *guard = None;
            log::info!("Модель речи выгружена из памяти (не использовалась {} мин)", idle.as_secs() / 60);
        }
    }
}

impl Engine {
    fn load(dir: &Path, gpu: bool) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if !model_ready(dir) {
            return Err(format!("модель не скачана ({})", dir.display()).into());
        }
        let device = if gpu {
            Device::new_metal(0).unwrap_or_else(|e| {
                log::warn!("Видеокарта недоступна ({e}), считаю на процессоре");
                Device::Cpu
            })
        } else {
            Device::Cpu
        };
        // На видеокарте — половинная точность: вдвое меньше памяти, качество почти то же
        let dtype = if device.is_metal() { DType::F16 } else { m::DTYPE };
        let config: Config = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| e.to_string())?;
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], dtype, &device)? };
        let model = Whisper::load(&vb, config.clone())?;

        let tok = |s: &str| tokenizer.token_to_id(s).ok_or_else(|| format!("нет токена {s}"));
        let (sot, eot) = (tok(m::SOT_TOKEN)?, tok(m::EOT_TOKEN)?);
        let (transcribe, no_timestamps) = (tok(m::TRANSCRIBE_TOKEN)?, tok(m::NO_TIMESTAMPS_TOKEN)?);

        // Запрещаем служебные токены (кроме конца текста) и список из config.json
        let suppress: Vec<f32> = (0..config.vocab_size as u32)
            .map(|i| {
                if config.suppress_tokens.contains(&i) || (i > eot) {
                    f32::NEG_INFINITY
                } else {
                    0.0
                }
            })
            .collect();
        let suppress = Tensor::new(suppress.as_slice(), &device)?;
        let filters = mel_filters(config.num_mel_bins, m::N_FFT, m::SAMPLE_RATE as f32);

        Ok(Engine {
            dir: dir.to_path_buf(),
            gpu,
            dtype,
            last_used: Instant::now(),
            model,
            tokenizer,
            config,
            filters,
            device,
            sot,
            eot,
            transcribe,
            no_timestamps,
            suppress,
        })
    }

    fn run(&mut self, pcm: &[f32], lang: &str) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
        let n_mels = self.config.num_mel_bins;
        let mel = audio::pcm_to_mel(&self.config, pcm, &self.filters);
        let frames = mel.len() / n_mels;
        let mel = Tensor::from_vec(mel, (1, n_mels, frames), &self.device)?.to_dtype(self.dtype)?;
        // pcm_to_mel дополняет запись тишиной — содержимое считаем по исходным сэмплам
        let content = (pcm.len() / m::HOP_LENGTH).clamp(1, frames);
        // Хвост короче секунды не распознаём: на тишине модель любит «придумывать» текст
        const MIN_TAIL: usize = 100;

        let mut language = lang.to_string();
        let mut text = String::new();
        let mut seek = 0;
        while seek < content {
            if seek > 0 && content - seek < MIN_TAIL {
                break;
            }
            let len = m::N_FRAMES.min(frames - seek);
            let segment = mel.narrow(2, seek, len)?;
            let features = self.model.encoder.forward(&segment, true)?;

            if language == "auto" || language.is_empty() {
                language = self.detect_language(&features)?;
                log::info!("Определён язык речи: {language}");
            }
            let lang_token = self.tokenizer.token_to_id(&format!("<|{language}|>"));
            let mut tokens = vec![self.sot];
            if let Some(t) = lang_token {
                tokens.push(t);
            }
            tokens.extend([self.transcribe, self.no_timestamps]);
            let prefix = tokens.len();

            for i in 0..MAX_TOKENS {
                let input = Tensor::new(tokens.as_slice(), &self.device)?.unsqueeze(0)?;
                let ys = self.model.decoder.forward(&input, &features, i == 0)?;
                let (_, seq, _) = ys.dims3()?;
                let logits = self.model.decoder.final_linear(&ys.i((..1, seq - 1..))?)?.i(0)?.i(0)?;
                let logits = logits.to_dtype(DType::F32)?.broadcast_add(&self.suppress)?;
                let next = logits.argmax(D::Minus1)?.to_scalar::<u32>()?;
                if next == self.eot || Self::looping(&tokens[prefix..], next) {
                    break;
                }
                tokens.push(next);
            }
            let piece = self.tokenizer.decode(&tokens[prefix..], true).map_err(|e| e.to_string())?;
            let piece = piece.trim();
            if !piece.is_empty() {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(piece);
            }
            seek += m::N_FRAMES;
        }
        Ok((text, language))
    }

    fn detect_language(&mut self, features: &Tensor) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let ids: Vec<(usize, u32)> = LANGUAGES
            .iter()
            .enumerate()
            .filter_map(|(i, l)| self.tokenizer.token_to_id(&format!("<|{l}|>")).map(|t| (i, t)))
            .collect();
        let input = Tensor::new(&[self.sot], &self.device)?.unsqueeze(0)?;
        let ys = self.model.decoder.forward(&input, features, true)?;
        let logits = self.model.decoder.final_linear(&ys.i(..1)?)?.i(0)?.i(0)?;
        let logits: Vec<f32> = logits.to_dtype(DType::F32)?.to_vec1()?;
        let best = ids
            .iter()
            .max_by(|a, b| logits[a.1 as usize].total_cmp(&logits[b.1 as usize]))
            .map(|(i, _)| LANGUAGES[*i])
            .unwrap_or("en");
        Ok(best.to_string())
    }

    /// Защита от зацикливания: один и тот же фрагмент повторяется много раз подряд
    fn looping(tokens: &[u32], next: u32) -> bool {
        for n in 1..=8 {
            let reps = 6.max(16 / n);
            let need = n * reps;
            if tokens.len() + 1 < need {
                continue;
            }
            let mut seq: Vec<u32> = tokens[tokens.len() + 1 - need..].to_vec();
            seq.push(next);
            let tail = &seq[seq.len() - need..];
            if tail.chunks(n).all(|c| c == &tail[..n]) {
                return true;
            }
        }
        false
    }
}

/// Мел-фильтры как в librosa (шкала и нормировка Slaney) — такие же использует Whisper
fn mel_filters(n_mels: usize, n_fft: usize, sr: f32) -> Vec<f32> {
    let hz_to_mel = |f: f32| {
        let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
        let min_log_mel = min_log_hz / f_sp;
        let logstep = (6.4f32).ln() / 27.0;
        if f >= min_log_hz { min_log_mel + (f / min_log_hz).ln() / logstep } else { f / f_sp }
    };
    let mel_to_hz = |m: f32| {
        let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
        let min_log_mel = min_log_hz / f_sp;
        let logstep = (6.4f32).ln() / 27.0;
        if m >= min_log_mel { min_log_hz * (logstep * (m - min_log_mel)).exp() } else { f_sp * m }
    };
    let n_freq = n_fft / 2 + 1;
    let fft_freqs: Vec<f32> = (0..n_freq).map(|k| k as f32 * sr / n_fft as f32).collect();
    let (mmin, mmax) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let mel_f: Vec<f32> =
        (0..n_mels + 2).map(|i| mel_to_hz(mmin + (mmax - mmin) * i as f32 / (n_mels + 1) as f32)).collect();

    let mut w = vec![0f32; n_mels * n_freq];
    for i in 0..n_mels {
        let (lo, c, hi) = (mel_f[i], mel_f[i + 1], mel_f[i + 2]);
        let enorm = 2.0 / (hi - lo);
        for (k, &f) in fft_freqs.iter().enumerate() {
            let lower = (f - lo) / (c - lo);
            let upper = (hi - f) / (hi - c);
            w[i * n_freq + k] = lower.min(upper).max(0.0) * enorm;
        }
    }
    w
}
