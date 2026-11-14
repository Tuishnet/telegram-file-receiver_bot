//! История принятых файлов: `history.jsonl` (одна запись — одна строка JSON).
//! Её же читает приложение «Настройки бота» для вкладки «История».

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub time: String,
    pub user_id: i64,
    pub chat_id: i64,
    #[serde(default)]
    pub chat: String,
    pub kind: String,
    pub name: String,
    pub path: String,
    pub size: u64,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub media_id: i64,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

impl Entry {
    pub fn exists(&self) -> bool {
        !self.deleted && Path::new(&self.path).exists()
    }
}

pub struct History {
    path: PathBuf,
    entries: Mutex<Vec<Entry>>,
}

impl History {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let entries = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        History { path, entries: Mutex::new(entries) }
    }

    pub fn next_id(&self) -> u64 {
        self.entries.lock().unwrap().iter().map(|e| e.id).max().unwrap_or(0) + 1
    }

    pub fn add(&self, mut entry: Entry) -> u64 {
        let mut entries = self.entries.lock().unwrap();
        entry.id = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        let id = entry.id;
        if let Ok(line) = serde_json::to_string(&entry) {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(&self.path) {
                let _ = writeln!(f, "{line}");
            }
        }
        entries.push(entry);
        id
    }

    pub fn update(&self, id: u64, f: impl FnOnce(&mut Entry)) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
            f(e);
            Self::rewrite(&self.path, &entries);
        }
    }

    fn rewrite(path: &Path, entries: &[Entry]) {
        let text: String = entries
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .map(|l| l + "\n")
            .collect();
        use std::os::unix::fs::PermissionsExt;
        let tmp = path.with_extension("jsonl.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            // В истории есть распознанный текст — читать её может только владелец
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            let _ = std::fs::rename(&tmp, path);
        }
    }

    pub fn get(&self, id: u64) -> Option<Entry> {
        self.entries.lock().unwrap().iter().find(|e| e.id == id).cloned()
    }

    /// Последние существующие файлы (новые первыми)
    pub fn recent(&self, n: usize) -> Vec<Entry> {
        self.entries.lock().unwrap().iter().rev().filter(|e| e.exists()).take(n).cloned().collect()
    }

    /// Последний файл, присланный пользователем
    pub fn last_by_user(&self, user_id: i64) -> Option<Entry> {
        self.entries.lock().unwrap().iter().rev().find(|e| e.user_id == user_id && e.exists()).cloned()
    }

    /// Поиск по имени и распознанному тексту
    pub fn search(&self, query: &str, n: usize) -> Vec<Entry> {
        let q = query.to_lowercase();
        self.entries
            .lock()
            .unwrap()
            .iter()
            .rev()
            .filter(|e| e.exists())
            .filter(|e| {
                e.name.to_lowercase().contains(&q)
                    || e.text.as_deref().is_some_and(|t| t.to_lowercase().contains(&q))
            })
            .take(n)
            .cloned()
            .collect()
    }

    pub fn find_media(&self, media_id: i64) -> Option<Entry> {
        if media_id == 0 {
            return None;
        }
        self.entries.lock().unwrap().iter().rev().find(|e| e.media_id == media_id && e.exists()).cloned()
    }

    /// Существующие файлы ровно такого размера (кандидаты в дубли)
    pub fn with_size(&self, size: u64) -> Vec<Entry> {
        self.entries.lock().unwrap().iter().rev().filter(|e| e.size == size && e.exists()).take(5).cloned().collect()
    }

    pub fn find_hash(&self, sha: &str, except: &Path) -> Option<Entry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|e| e.sha256 == sha && Path::new(&e.path) != except && e.exists())
            .cloned()
    }

    pub fn totals(&self) -> (usize, u64) {
        let entries = self.entries.lock().unwrap();
        let live: Vec<_> = entries.iter().filter(|e| e.exists()).collect();
        (live.len(), live.iter().map(|e| e.size).sum())
    }
}
