//! Распаковка ZIP-архивов с защитой от «zip-бомб» и путей вида `../../`.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 20_000;

pub struct Unpacked {
    pub dir: PathBuf,
    pub files: usize,
    pub bytes: u64,
}

/// Распаковывает `zip` в папку рядом с ним. `max_bytes` — сколько всего можно распаковать.
pub fn unzip(zip_path: &Path, max_bytes: u64) -> Result<Unpacked, String> {
    let file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("не ZIP-архив: {e}"))?;
    if archive.len() > MAX_ENTRIES {
        return Err(format!("слишком много файлов в архиве ({})", archive.len()));
    }

    let parent = zip_path.parent().unwrap_or(Path::new("."));
    let stem = zip_path.file_stem().unwrap_or_default().to_string_lossy();
    let dir = crate::storage::unique_path(parent, &stem);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let mut total = 0u64;
    let mut files = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        // Имя в правильной кодировке и только безопасные части пути (без «..» и абсолютных путей)
        let Some(rel) = safe_relative_path(&decode_name(entry.name_raw())) else { continue };
        let out = dir.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if entry.is_symlink() {
            continue;
        }
        if let Some(p) = out.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let remaining = max_bytes.saturating_sub(total);
        let mut f = fs::File::create(&out).map_err(|e| e.to_string())?;
        // Читаем не больше, чем осталось лимита (+1 байт, чтобы заметить превышение)
        let written = io::copy(&mut (&mut entry).take(remaining + 1), &mut f).map_err(|e| e.to_string())?;
        total += written;
        if total > max_bytes {
            drop(f);
            let _ = fs::remove_dir_all(&dir);
            return Err("архив распаковывается в слишком большой объём — распаковка отменена".into());
        }
        files += 1;
    }
    Ok(Unpacked { dir, files, bytes: total })
}

/// Имя файла в архиве. Стандарт ZIP без пометки «UTF-8» подразумевает CP437, но на деле
/// macOS пишет UTF-8 без пометки, а Windows с русской локалью — CP866. Пробуем UTF-8, затем CP866.
fn decode_name(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => raw.iter().map(|&b| cp866(b)).collect(),
    }
}

fn cp866(b: u8) -> char {
    match b {
        0x00..=0x7F => b as char,
        0x80..=0x9F => char::from_u32(0x0410 + (b - 0x80) as u32).unwrap(), // А–Я
        0xA0..=0xAF => char::from_u32(0x0430 + (b - 0xA0) as u32).unwrap(), // а–п
        0xE0..=0xEF => char::from_u32(0x0440 + (b - 0xE0) as u32).unwrap(), // р–я
        0xF0 => 'Ё',
        0xF1 => 'ё',
        0xF2 => 'Є',
        0xF3 => 'є',
        0xF4 => 'Ї',
        0xF5 => 'ї',
        0xF6 => 'Ў',
        0xF7 => 'ў',
        _ => '_',
    }
}

/// Относительный путь внутри папки распаковки: отбрасываем пустые части, «.», «..» и корень
fn safe_relative_path(name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return None,
            p => out.push(crate::storage::sanitize(p)),
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(dir: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join("test.zip");
        let mut z = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, data) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
        path
    }

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fr-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn unpacks_and_blocks_path_traversal() {
        let d = tmpdir("traversal");
        let zip = make_zip(&d, &[("ok/a.txt", b"hello"), ("../evil.txt", b"x"), ("/abs.txt", b"y")]);
        let u = unzip(&zip, 1 << 20).unwrap();
        let mut found = Vec::new();
        for e in walk(&u.dir) {
            found.push(e.strip_prefix(&u.dir).unwrap().to_string_lossy().to_string());
        }
        found.sort();
        eprintln!("распаковано: {found:?}");
        // «../evil.txt» отброшен; «/abs.txt» безопасно лёг внутрь папки
        assert!(!found.iter().any(|f| f.contains("evil")));
        assert!(found.iter().all(|f| !f.starts_with('/') && !f.contains("..")));
        assert_eq!(fs::read_to_string(u.dir.join("ok/a.txt")).unwrap(), "hello");
        assert!(!d.join("evil.txt").exists());
        assert!(!Path::new("/abs.txt").exists() || fs::read("/abs.txt").map(|b| b != b"y").unwrap_or(true));
        fs::remove_dir_all(&d).unwrap();
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() { out.extend(walk(&p)) } else { out.push(p) }
        }
        out
    }

    #[test]
    fn decodes_names_from_macos_and_windows() {
        assert_eq!(decode_name("Документы/Договор 7.txt".as_bytes()), "Документы/Договор 7.txt");
        // «Отчёт.txt» в CP866, как его пишет архиватор Windows с русской локалью
        assert_eq!(decode_name(&[0x8E, 0xE2, 0xE7, 0xF1, 0xE2, b'.', b't', b'x', b't']), "Отчёт.txt");
        assert_eq!(safe_relative_path("../evil.txt"), None);
        assert_eq!(safe_relative_path("/abs/a.txt"), Some(PathBuf::from("abs/a.txt")));
        assert_eq!(safe_relative_path("a\\b\\c.txt"), Some(PathBuf::from("a/b/c.txt")));
    }

    #[test]
    fn stops_zip_bomb() {
        let d = tmpdir("bomb");
        let big = vec![0u8; 2 << 20];
        let zip = make_zip(&d, &[("big.bin", &big)]);
        assert!(unzip(&zip, 1 << 20).is_err());
        assert!(!d.join("test").exists(), "частично распакованное удалено");
        fs::remove_dir_all(&d).unwrap();
    }
}
