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

    /// Файлы, в которых расшифровывается речь
    pub fn is_speech(self) -> bool {
        matches!(self, Kind::Voice | Kind::Audio | Kind::VideoNote | Kind::Video)
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

/// Атомарно занимает свободное имя: создаёт `<имя>.part` только если его ещё нет (O_EXCL)
/// и только потом проверяет, что готового файла с таким именем тоже нет.
/// Так два параллельных обработчика не могут получить одно и то же имя.
/// Возвращает (итоговый путь, путь к созданному временному файлу).
pub fn reserve_path(dir: &Path, name: &str) -> std::io::Result<(PathBuf, PathBuf)> {
    let p = Path::new(name);
    let stem = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for i in 0..100_000u32 {
        let candidate = if i == 0 { dir.join(name) } else { dir.join(format!("{stem} ({i}){ext}")) };
        let tmp = part_path(&candidate);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(_) => {
                if candidate.exists() {
                    // Имя уже занято готовым файлом — отпускаем и пробуем следующее
                    let _ = std::fs::remove_file(&tmp);
                    continue;
                }
                return Ok((candidate, tmp));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("не удалось подобрать свободное имя"))
}

pub fn part_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".part");
    s.into()
}

/// Проверяет, что в папку можно сохранять, и создаёт её при необходимости.
/// Возвращает понятное описание проблемы: диск не подключён, нет прав и т. п.
pub fn ensure_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    // Папка на внешнем диске: /Volumes/<диск>/… — диск должен быть подключён.
    // Иначе create_dir_all попытался бы создать папку на системном диске.
    let mut parts = dir.components();
    if let (Some(std::path::Component::RootDir), Some(first), Some(second)) = (parts.next(), parts.next(), parts.next()) {
        if first.as_os_str() == "Volumes" {
            let volume = Path::new("/Volumes").join(second.as_os_str());
            let name = second.as_os_str().to_string_lossy();
            let is_link = std::fs::symlink_metadata(&volume).is_ok_and(|m| m.file_type().is_symlink());
            let mounted = if is_link {
                // «Macintosh HD» — ссылка на системный диск: подключён, если ссылка куда-то ведёт
                std::fs::metadata(&volume).is_ok()
            } else {
                match (std::fs::metadata(&volume), std::fs::metadata("/Volumes")) {
                    // Точка монтирования лежит на другом устройстве, чем сама /Volumes
                    (Ok(v), Ok(root)) => v.dev() != root.dev(),
                    _ => false,
                }
            };
            if !mounted {
                return Err(format!("диск «{name}» не подключён"));
            }
        }
    }

    if let Err(e) = std::fs::create_dir_all(dir) {
        return Err(match e.kind() {
            std::io::ErrorKind::PermissionDenied => format!("нет прав на создание папки {}", dir.display()),
            _ => format!("не удалось создать папку {}: {e}", dir.display()),
        });
    }
    if !dir.is_dir() {
        return Err(format!("{} — это не папка", dir.display()));
    }
    // Реальная проверка записи: создаём и сразу удаляем пустой файл
    let probe = dir.join(format!(".file_receiver_write_test_{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err(format!("нет прав на запись в папку {}", dir.display()))
        }
        Err(e) => Err(format!("нельзя записать в папку {}: {e}", dir.display())),
    }
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
    {
        let d = dir.clone();
        tokio::task::spawn_blocking(move || ensure_dir(&d)).await.map_err(|e| e.to_string())??;
    }
    let rel = path.strip_prefix(&cfg.save_dir).unwrap_or(Path::new(path.file_name().unwrap_or_default()));
    let dest_dir = dir.join(rel.parent().unwrap_or(Path::new("")));
    tokio::fs::create_dir_all(&dest_dir).await.map_err(|e| e.to_string())?;
    let (dest, tmp) =
        reserve_path(&dest_dir, &rel.file_name().unwrap_or_default().to_string_lossy()).map_err(|e| e.to_string())?;
    if let Err(e) = tokio::fs::copy(path, &tmp).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e.to_string());
    }
    tokio::fs::rename(&tmp, &dest).await.map_err(|e| e.to_string())?;
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
    fn parallel_reservations_get_distinct_names() {
        let dir = std::env::temp_dir().join(format!("fr-reserve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("photo.jpg"), b"old").unwrap();
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let d = dir.clone();
                std::thread::spawn(move || reserve_path(&d, "photo.jpg").unwrap())
            })
            .collect();
        let mut names: Vec<PathBuf> = handles.into_iter().map(|h| h.join().unwrap().0).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 16, "все имена разные");
        assert!(!names.contains(&dir.join("photo.jpg")), "существующий файл не занят повторно");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folder_checks() {
        let base = std::env::temp_dir().join(format!("fr-ensure-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // Обычная несуществующая папка создаётся
        assert!(ensure_dir(&base.join("a/b")).is_ok());
        assert!(base.join("a/b").is_dir());
        // Неподключённый внешний диск — понятная ошибка, а не попытка создать папку
        let e = ensure_dir(Path::new("/Volumes/НетТакогоДиска/Telegram")).unwrap_err();
        assert!(e.contains("не подключён"), "{e}");
        // Системный диск через ссылку /Volumes/Macintosh HD — подключён
        if Path::new("/Volumes/Macintosh HD").exists() {
            let via_link = Path::new("/Volumes/Macintosh HD").join(base.strip_prefix("/").unwrap()).join("c");
            assert!(ensure_dir(&via_link).is_ok());
        }
        // Папка без прав на запись
        use std::os::unix::fs::PermissionsExt;
        let ro = base.join("ro");
        std::fs::create_dir_all(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o500)).unwrap();
        let e = ensure_dir(&ro.join("sub")).unwrap_err();
        assert!(e.contains("нет прав"), "{e}");
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
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
