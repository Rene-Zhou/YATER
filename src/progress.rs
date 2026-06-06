use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub block_index: usize,
    pub sentence_offset: usize,
    pub timestamp: String,
}

#[derive(Debug, Clone)]
pub struct ProgressStore {
    path: PathBuf,
}

impl ProgressStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn from_env() -> Option<Self> {
        let xdg_data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
        let home = std::env::var_os("HOME").map(PathBuf::from);

        progress_path_from_env(xdg_data_home.as_deref(), home.as_deref()).map(Self::new)
    }

    pub fn save(&self, book_path: &Path, progress: Progress) -> io::Result<()> {
        let mut progress_by_book = match self.load_all() {
            Ok(progress_by_book) => progress_by_book,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => HashMap::new(),
            Err(error) => return Err(error),
        };
        progress_by_book.insert(progress_key(book_path), progress);

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = serde_json::to_string_pretty(&progress_by_book).map_err(io::Error::other)?;
        fs::write(&self.path, contents)
    }

    pub fn load(&self, book_path: &Path) -> io::Result<Option<Progress>> {
        Ok(self.load_all()?.remove(&progress_key(book_path)))
    }

    fn load_all(&self) -> io::Result<HashMap<String, Progress>> {
        match fs::read_to_string(&self.path) {
            Ok(contents) => serde_json::from_str(&contents)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(error) => Err(error),
        }
    }
}

fn progress_key(book_path: &Path) -> String {
    fs::canonicalize(book_path)
        .unwrap_or_else(|_| book_path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

pub fn progress_path_from_env(
    xdg_data_home: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(xdg_data_home) = xdg_data_home {
        return Some(xdg_data_home.join("yater").join("progress.json"));
    }

    home.map(|home| {
        home.join(".local")
            .join("share")
            .join("yater")
            .join("progress.json")
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{Progress, ProgressStore};

    #[test]
    fn saves_and_loads_progress_by_book_path() {
        let tempdir = tempdir().expect("temp dir");
        let store = ProgressStore::new(tempdir.path().join("progress.json"));
        let book_path = Path::new("/books/example.epub");
        let progress = Progress {
            block_index: 42,
            sentence_offset: 7,
            timestamp: "2026-06-03T12:00:00Z".to_string(),
        };

        store.save(book_path, progress.clone()).expect("save progress");

        assert_eq!(
            store.load(book_path).expect("load progress"),
            Some(progress)
        );
    }

    #[test]
    fn saving_one_book_preserves_other_book_progress() {
        let tempdir = tempdir().expect("temp dir");
        let store = ProgressStore::new(tempdir.path().join("progress.json"));
        let first_book = Path::new("/books/first.epub");
        let second_book = Path::new("/books/second.epub");
        let first_progress = Progress {
            block_index: 1,
            sentence_offset: 2,
            timestamp: "2026-06-03T12:00:00Z".to_string(),
        };
        let second_progress = Progress {
            block_index: 3,
            sentence_offset: 4,
            timestamp: "2026-06-03T12:01:00Z".to_string(),
        };

        store
            .save(first_book, first_progress.clone())
            .expect("save first progress");
        store
            .save(second_book, second_progress.clone())
            .expect("save second progress");

        assert_eq!(
            store.load(first_book).expect("load first progress"),
            Some(first_progress)
        );
        assert_eq!(
            store.load(second_book).expect("load second progress"),
            Some(second_progress)
        );
    }

    #[test]
    fn equivalent_existing_book_paths_share_progress() {
        let tempdir = tempdir().expect("temp dir");
        let book_path = tempdir.path().join("book.epub");
        std::fs::write(&book_path, "book").expect("write book");
        std::fs::create_dir(tempdir.path().join("subdir")).expect("create subdir");
        let aliased_book_path = tempdir.path().join("subdir/../book.epub");
        let store = ProgressStore::new(tempdir.path().join("progress.json"));
        let progress = Progress {
            block_index: 5,
            sentence_offset: 8,
            timestamp: "2026-06-06T12:00:00Z".to_string(),
        };

        store
            .save(&aliased_book_path, progress.clone())
            .expect("save progress");

        assert_eq!(
            store.load(&book_path).expect("load progress"),
            Some(progress)
        );
    }

    #[test]
    fn saving_replaces_malformed_progress_file() {
        let tempdir = tempdir().expect("temp dir");
        let progress_path = tempdir.path().join("progress.json");
        std::fs::write(&progress_path, "{broken json").expect("write malformed progress");
        let store = ProgressStore::new(progress_path);
        let book_path = Path::new("/books/example.epub");
        let progress = Progress {
            block_index: 7,
            sentence_offset: 11,
            timestamp: "2026-06-06T12:00:00Z".to_string(),
        };

        store
            .save(book_path, progress.clone())
            .expect("replace malformed progress");

        assert_eq!(
            store.load(book_path).expect("load replacement progress"),
            Some(progress)
        );
    }

    #[test]
    fn progress_path_prefers_xdg_data_home() {
        assert_eq!(
            super::progress_path_from_env(Some(Path::new("/xdg-data")), Some(Path::new("/home/me"))),
            Some(PathBuf::from("/xdg-data/yater/progress.json"))
        );
    }

    #[test]
    fn progress_path_falls_back_to_home_local_share() {
        assert_eq!(
            super::progress_path_from_env(None, Some(Path::new("/home/me"))),
            Some(PathBuf::from("/home/me/.local/share/yater/progress.json"))
        );
    }

    #[test]
    fn progress_path_is_absent_without_xdg_or_home() {
        assert_eq!(super::progress_path_from_env(None, None), None);
    }
}
