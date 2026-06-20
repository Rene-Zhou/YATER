use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct IssueLog {
    path: PathBuf,
}

impl IssueLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn from_env() -> Option<Self> {
        let xdg_state_home = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
        let home = std::env::var_os("HOME").map(PathBuf::from);

        issue_log_path_from_env(xdg_state_home.as_deref(), home.as_deref()).map(Self::new)
    }

    pub fn append(
        &self,
        timestamp: impl AsRef<str>,
        message: impl AsRef<str>,
    ) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        use std::io::Write;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{} {}", timestamp.as_ref(), message.as_ref())
    }
}

pub fn issue_log_path_from_env(
    xdg_state_home: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    xdg_state_home
        .map(|path| path.join("yater").join("yater.log"))
        .or_else(|| {
            home.map(|path| {
                path.join(".local")
                    .join("state")
                    .join("yater")
                    .join("yater.log")
            })
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{IssueLog, issue_log_path_from_env};

    #[test]
    fn issue_log_path_prefers_xdg_state_home() {
        let path =
            issue_log_path_from_env(Some(Path::new("/state")), Some(Path::new("/home/user")));

        assert_eq!(
            path,
            Some(Path::new("/state/yater/yater.log").to_path_buf())
        );
    }

    #[test]
    fn issue_log_path_falls_back_to_home_local_state() {
        let path = issue_log_path_from_env(None, Some(Path::new("/home/user")));

        assert_eq!(
            path,
            Some(Path::new("/home/user/.local/state/yater/yater.log").to_path_buf())
        );
    }

    #[test]
    fn appends_non_fatal_issues_to_log_file() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let log_path = tempdir.path().join("state/yater/yater.log");
        let log = IssueLog::new(log_path.clone());

        log.append("100", "bad image: cover.png")
            .expect("append first issue");
        log.append("101", "malformed HTML: chapter.xhtml")
            .expect("append second issue");

        let contents = std::fs::read_to_string(log_path).expect("read log");
        assert_eq!(
            contents,
            "100 bad image: cover.png\n101 malformed HTML: chapter.xhtml\n"
        );
    }
}
