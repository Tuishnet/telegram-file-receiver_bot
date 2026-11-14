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
        // enclosed_name отбрасывает абсолютные пути и выходы за пределы папки
        let Some(rel) = entry.enclosed_name() else { continue };
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
    fn stops_zip_bomb() {
        let d = tmpdir("bomb");
        let big = vec![0u8; 2 << 20];
        let zip = make_zip(&d, &[("big.bin", &big)]);
        assert!(unzip(&zip, 1 << 20).is_err());
        assert!(!d.join("test").exists(), "частично распакованное удалено");
        fs::remove_dir_all(&d).unwrap();
    }
}
