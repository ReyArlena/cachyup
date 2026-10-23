//! Where cachyup keeps its files (XDG base directories).
//!
//!   ~/.config/cachyup/config.toml       tasks, update steps, cleanup settings
//!   ~/.config/cachyup/selection.toml    on/off for every app and tweak
//!   ~/.config/cachyup/user-apps.toml    the user's own additions (optional)
//!   ~/.local/state/cachyup/state.toml   what cachyup has seen and installed
//!   ~/.local/state/cachyup/runs/<id>/   one directory per run: output.log, run.json
//!   ~/.cache/cachyup/info.json          package descriptions read from the sources

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub home: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        let xdg = |var: &str, fallback: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(fallback))
                .join("cachyup")
        };
        Self {
            config_dir: xdg("XDG_CONFIG_HOME", ".config"),
            state_dir: xdg("XDG_STATE_HOME", ".local/state"),
            cache_dir: xdg("XDG_CACHE_HOME", ".cache"),
            home: home.clone(),
        }
    }

    /// Everything under one directory. Used by tests.
    pub fn under(base: &Path) -> Self {
        Self {
            config_dir: base.join("config"),
            state_dir: base.join("state"),
            cache_dir: base.join("cache"),
            home: base.join("home"),
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn selection_file(&self) -> PathBuf {
        self.config_dir.join("selection.toml")
    }

    pub fn user_apps_file(&self) -> PathBuf {
        self.config_dir.join("user-apps.toml")
    }

    pub fn state_file(&self) -> PathBuf {
        self.state_dir.join("state.toml")
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.state_dir.join("runs")
    }

    pub fn info_cache(&self) -> PathBuf {
        self.cache_dir.join("info.json")
    }

    /// Expand a leading `~/` to the home directory.
    pub fn expand(&self, path: &str) -> PathBuf {
        match path.strip_prefix("~/") {
            Some(rest) => self.home.join(rest),
            None if path == "~" => self.home.clone(),
            None => PathBuf::from(path),
        }
    }

    /// Show a path with the home directory as `~`.
    pub fn tilde(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }
}

/// Write a file, creating parent directories. Writes through symlinks (the
/// file is opened, never replaced), so a stow-managed dotfile stays a link.
pub fn write_file(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expands_and_collapses() {
        let paths = Paths::under(Path::new("/tmp/x"));
        let p = paths.expand("~/.config/a.json");
        assert_eq!(p, PathBuf::from("/tmp/x/home/.config/a.json"));
        assert_eq!(paths.tilde(&p), "~/.config/a.json");
        assert_eq!(paths.expand("/etc/x"), PathBuf::from("/etc/x"));
    }

    #[cfg(unix)]
    #[test]
    fn write_file_goes_through_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.json");
        let link = dir.path().join("sub/link.json");
        std::fs::write(&real, "old").unwrap();
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_file(&link, "new").unwrap();
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
    }
}
