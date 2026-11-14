//! Где и под каким именем сохранять файлы, свободное место, резервные копии.

use std::path::{Path, PathBuf};

use crate::config::Config;

/// Тип файла — по нему выбирается подпапка
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Photo,
    Video,
    VideoNote,
    Audio,
    Voice,
    Document,
    Archive,
    Other,
}

impl Kind {
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Photo => "Фото",
            Kind::Video | Kind::VideoNote => "Видео",
            Kind::Audio => "Аудио",
            Kind::Voice => "Голосовые",
            Kind::Document => "Документы",
            Kind::Archive => "Архивы",
            Kind::Other => "Другое",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Kind::Photo => "photo",
            Kind::Video => "video",
            Kind::VideoNote => "video_note",
            Kind::Audio => "audio",
            Kind::Voice => "voice",
            Kind::Document => "document",
            Kind::Archive => "archive",
            Kind::Other => "other",
        }
    }

    pub fn from_code(code: &str) -> Kind {
        match code {
            "photo" => Kind::Photo,
            "video" => Kind::Video,
            "video_note" => Kind::VideoNote,
            "audio" => Kind::Audio,
            "voice" => Kind::Voice,
            "document" => Kind::Document,
            "archive" => Kind::Archive,
            _ => Kind::Other,
        }
    }

    pub fn from_file(name: &str, mime: &str) -> Kind {
        let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" => return Kind::Archive,
            "jpg" | "jpeg" | "png" | "heic" | "heif" | "webp" | "gif" | "bmp" | "tif" | "tiff" => return Kind::Photo,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "rtf" | "odt" | "ods" | "csv"
            | "pages" | "numbers" | "key" | "md" | "epub" | "fb2" | "djvu" => return Kind::Document,
            _ => {}
        }
        match mime.split('/').next().unwrap_or("") {
            "image" => Kind::Photo,
            "video" => Kind::Video,
            "audio" if mime == "audio/ogg" => Kind::Voice,
            "audio" => Kind::Audio,
            "text" => Kind::Document,
            _ => Kind::Other,
        }
    }
}

/// Папка для нового файла с учётом настроек сортировки
pub fn target_dir(cfg: &Config, kind: Kind, group_title: Option<&str>) -> PathBuf {
    let mut dir = cfg.save_dir.clone();
    if cfg.group_folders {
        if let Some(title) = group_title {
            dir = dir.join("Группы").join(sanitize(title));
        }
    }
    if cfg.sort_by_type {
        dir = dir.join(kind.folder());
    }
    if cfg.sort_by_date {
        dir = dir.join(chrono::Local::now().format("%Y-%m").to_string());
    }
    dir
}

/// Убирает из имени разделители путей и прочие опасные символы
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim().to_string();
    if cleaned.is_empty() { "file".into() } else { cleaned }
}

/// Имя из подписи к файлу: первая строка, не длиннее 100 символов, с расширением исходного файла
pub fn name_from_caption(caption: &str, original: &str) -> Option<String> {
    let line = caption.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    let stem: String = sanitize(line).chars().take(100).collect();
    let stem = stem.trim_end();
    let ext = Path::new(original).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    // Подпись уже с тем же расширением («отчёт.pdf») — второй раз не добавляем
    if !ext.is_empty() && stem.to_lowercase().ends_with(&ext.to_lowercase()) {
        return Some(stem.to_string());
    }
    Some(format!("{stem}{ext}"))
}

/// Если файл с таким именем уже есть — добавляет (1), (2), ...
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() && !part_path(&candidate).exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|c| !c.exists() && !part_path(c).exists())
        .unwrap()
}

pub fn part_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".part");
    s.into()
}

/// Свободное место на диске, где лежит `path` (или ближайшая существующая папка)
pub fn free_space(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let mut p = path.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    let c = CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0).then(|| st.f_bavail as u64 * st.f_frsize as u64)
}

/// Копия файла в резервную папку (iCloud Drive, Google Drive и т. п.) с той же структурой папок
pub async fn backup(cfg: &Config, path: &Path) -> Result<PathBuf, String> {
    let Some(dir) = &cfg.backup_dir else { return Err("не задана".into()) };
    let rel = path.strip_prefix(&cfg.save_dir).unwrap_or(Path::new(path.file_name().unwrap_or_default()));
    let dest_dir = dir.join(rel.parent().unwrap_or(Path::new("")));
    tokio::fs::create_dir_all(&dest_dir).await.map_err(|e| e.to_string())?;
    let dest = unique_path(&dest_dir, &rel.file_name().unwrap_or_default().to_string_lossy());
    tokio::fs::copy(path, &dest).await.map_err(|e| e.to_string())?;
    Ok(dest)
}

pub fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} ГБ", b / 1024.0 / 1024.0 / 1024.0)
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} МБ", b / 1024.0 / 1024.0)
    } else if b >= 1024.0 {
        format!("{:.1} КБ", b / 1024.0)
    } else {
        format!("{bytes} Б")
    }
}

/// SHA-256 файла (для поиска дублей)
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize("  .hidden "), "hidden");
        assert_eq!(sanitize("a:b?c"), "a_b_c");
        assert_eq!(sanitize(""), "file");
    }

    #[test]
    fn caption_becomes_name() {
        assert_eq!(name_from_caption("Паспорт\nвторая строка", "photo_1.jpg").as_deref(), Some("Паспорт.jpg"));
        assert_eq!(name_from_caption("отчёт/2026", "doc.pdf").as_deref(), Some("отчёт_2026.pdf"));
        assert_eq!(name_from_caption("   ", "a.jpg"), None);
        assert_eq!(name_from_caption("1015.pdf", "scan.pdf").as_deref(), Some("1015.pdf"));
        assert_eq!(name_from_caption("Отчёт.PDF", "x.pdf").as_deref(), Some("Отчёт.PDF"));
        assert_eq!(name_from_caption("фото.png", "photo.jpg").as_deref(), Some("фото.png.jpg"));
    }

    #[test]
    fn detects_kinds() {
        assert_eq!(Kind::from_file("a.pdf", "application/pdf"), Kind::Document);
        assert_eq!(Kind::from_file("a.zip", ""), Kind::Archive);
        assert_eq!(Kind::from_file("x", "audio/ogg"), Kind::Voice);
        assert_eq!(Kind::from_file("song.mp3", "audio/mpeg"), Kind::Audio);
        assert_eq!(Kind::from_file("clip.mov", "video/quicktime"), Kind::Video);
    }
}
