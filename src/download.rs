//! Загрузка файлов из Telegram: несколько потоков параллельно + отчёт о прогрессе.

use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use grammers_client::Client;
use grammers_client::media::Media;

type Error = Box<dyn std::error::Error + Send + Sync>;

/// Максимальный размер куска, который Telegram отдаёт за один запрос
const CHUNK: usize = 512 * 1024;
const WORKERS: usize = 8;
const RETRIES: usize = 4;

/// Скачивает `media` размером `size` в `path`. `progress` получает число скачанных байт.
/// Для больших файлов возвращает SHA-256, посчитанный прямо во время загрузки
/// (чтобы не перечитывать файл с диска ради поиска дублей).
pub async fn download(
    client: &Client,
    media: &Media,
    size: u64,
    path: &Path,
    progress: Arc<AtomicU64>,
) -> Result<Option<String>, Error> {
    // Маленькие файлы и фото — обычной загрузкой библиотеки
    if size < 4 * CHUNK as u64 || matches!(media, Media::Photo(_)) {
        client.download_media(media, path).await?;
        progress.store(size, Ordering::Relaxed);
        return Ok(None);
    }

    let file = std::fs::File::create(path)?;
    file.set_len(size)?;
    let file = Arc::new(file);
    let chunks = (size as usize).div_ceil(CHUNK);
    let next = Arc::new(AtomicUsize::new(0));

    // Куски приходят не по порядку — хешер складывает их по номерам и считает SHA-256 подряд
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, Vec<u8>)>();
    let hasher = tokio::spawn(async move {
        let mut h = OrderedHasher::default();
        while let Some((idx, data)) = rx.recv().await {
            h.push(idx, data);
        }
        h.finish(chunks)
    });

    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..WORKERS.min(chunks) {
        let (client, media, file, next, progress, tx) =
            (client.clone(), media.clone(), file.clone(), next.clone(), progress.clone(), tx.clone());
        workers.spawn(async move {
            loop {
                let idx = next.fetch_add(1, Ordering::Relaxed);
                if idx >= chunks {
                    return Ok::<(), Error>(());
                }
                let mut attempt = 0;
                let data = loop {
                    let mut it = client.iter_download(&media).chunk_size(CHUNK as i32).skip_chunks(idx as i32);
                    match it.next().await {
                        Ok(d) => break d.unwrap_or_default(),
                        Err(e) if attempt + 1 < RETRIES => {
                            attempt += 1;
                            log::warn!("Кусок {idx}: ошибка {e}, повтор {attempt}");
                            tokio::time::sleep(std::time::Duration::from_secs(attempt as u64)).await;
                        }
                        Err(e) => return Err(e.into()),
                    }
                };
                let f = file.clone();
                let offset = (idx * CHUNK) as u64;
                let len = data.len() as u64;
                let data = tokio::task::spawn_blocking(move || f.write_all_at(&data, offset).map(|_| data)).await??;
                progress.fetch_add(len, Ordering::Relaxed);
                let _ = tx.send((idx, data));
            }
        });
    }
    drop(tx);
    while let Some(res) = workers.join_next().await {
        if let Err(e) = res? {
            workers.abort_all();
            hasher.abort();
            return Err(e);
        }
    }
    file.sync_all()?;
    Ok(hasher.await.ok().flatten())
}

/// Совпадает ли файл в Telegram с файлом на диске (того же размера).
/// Скачивает только 3 куска — начало, середину и конец — и сравнивает побайтно.
pub async fn looks_same(client: &Client, media: &Media, size: u64, existing: &Path) -> Result<bool, Error> {
    let on_disk = std::fs::File::open(existing)?;
    if on_disk.metadata()?.len() != size {
        return Ok(false);
    }
    let chunks = (size as usize).div_ceil(CHUNK);
    let mut probe = vec![0, chunks / 2, chunks.saturating_sub(1)];
    probe.dedup();
    for idx in probe {
        let remote = client
            .iter_download(media)
            .chunk_size(CHUNK as i32)
            .skip_chunks(idx as i32)
            .next()
            .await?
            .unwrap_or_default();
        if remote.is_empty() {
            return Ok(false); // пустой ответ — не повод считать файлы одинаковыми
        }
        let mut local = vec![0u8; remote.len()];
        on_disk.read_exact_at(&mut local, (idx * CHUNK) as u64)?;
        if local != remote {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Текст прогресса: «45% · 230 из 512 МБ · 12.3 МБ/с»
pub fn progress_text(done: u64, total: u64, elapsed: f64) -> String {
    let pct = if total > 0 { done * 100 / total } else { 0 };
    let speed = if elapsed > 0.0 { done as f64 / elapsed } else { 0.0 };
    format!(
        "{pct}% · {} из {} · {}/с",
        crate::storage::human_size(done),
        crate::storage::human_size(total),
        crate::storage::human_size(speed as u64)
    )
}

/// SHA-256 по кускам, которые приходят в произвольном порядке
#[derive(Default)]
struct OrderedHasher {
    sha: sha2::Sha256,
    pending: std::collections::BTreeMap<usize, Vec<u8>>,
    next: usize,
}

impl OrderedHasher {
    fn push(&mut self, idx: usize, data: Vec<u8>) {
        use sha2::Digest;
        self.pending.insert(idx, data);
        while let Some(d) = self.pending.remove(&self.next) {
            self.sha.update(&d);
            self.next += 1;
        }
    }

    /// Хеш, если пришли все `total` кусков
    fn finish(self, total: usize) -> Option<String> {
        use sha2::Digest;
        (self.next == total && self.pending.is_empty()).then(|| hex::encode(self.sha.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    #[test]
    fn ordered_hash_matches_plain_hash() {
        let data: Vec<u8> = (0..10_000u32).flat_map(|i| i.to_le_bytes()).collect();
        let chunks: Vec<Vec<u8>> = data.chunks(1000).map(|c| c.to_vec()).collect();
        // Перемешанный порядок прихода
        let order = [3, 0, 7, 1, 2, 9, 5, 4, 8, 6, 12, 10, 11, 15, 13, 14, 19, 16, 18, 17, 20, 21, 25, 22, 24, 23, 26, 27, 30, 28, 29, 31, 33, 32, 34, 35, 39, 36, 37, 38];
        let mut h = OrderedHasher::default();
        for &i in &order {
            h.push(i, chunks[i].clone());
        }
        assert_eq!(h.finish(chunks.len()), Some(hex::encode(sha2::Sha256::digest(&data))));
    }

    #[test]
    fn missing_chunk_gives_no_hash() {
        let mut h = OrderedHasher::default();
        h.push(0, vec![1]);
        h.push(2, vec![3]);
        assert_eq!(h.finish(3), None);
    }
}
