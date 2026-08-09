use std::path::{Path, PathBuf};

/// Falls back to `/` when the home directory cannot be determined, so path
/// navigation degrades to the filesystem root instead of panicking. Unlike
/// `persistence::config_dir`, this runs inside the render loop where a panic
/// would tear down the terminal mid-frame.
#[allow(dead_code)]
pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

#[allow(dead_code)]
pub fn expand_tilde(s: &str) -> PathBuf {
    if s == "~" {
        return home_dir();
    }
    match s.strip_prefix("~/") {
        Some(rest) => home_dir().join(rest),
        None => PathBuf::from(s),
    }
}

#[allow(dead_code)]
pub fn shorten_home(path: &Path) -> String {
    match path.strip_prefix(home_dir()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_bare() {
        assert_eq!(expand_tilde("~"), home_dir());
    }

    #[test]
    fn expand_tilde_with_subpath() {
        assert_eq!(expand_tilde("~/projects"), home_dir().join("projects"));
    }

    #[test]
    fn expand_tilde_leaves_absolute_paths_alone() {
        assert_eq!(expand_tilde("/usr/local"), PathBuf::from("/usr/local"));
    }

    #[test]
    fn expand_tilde_does_not_expand_mid_string_tilde() {
        assert_eq!(expand_tilde("/tmp/~foo"), PathBuf::from("/tmp/~foo"));
    }

    #[test]
    fn shorten_home_replaces_home_prefix() {
        let p = home_dir().join("projects").join("tws");
        assert_eq!(shorten_home(&p), "~/projects/tws");
    }

    #[test]
    fn shorten_home_of_home_itself() {
        assert_eq!(shorten_home(&home_dir()), "~");
    }

    #[test]
    fn shorten_home_leaves_outside_paths_alone() {
        assert_eq!(shorten_home(Path::new("/usr/local")), "/usr/local");
    }

    #[test]
    fn shorten_home_round_trips_with_expand_tilde() {
        let p = home_dir().join("a").join("b");
        assert_eq!(expand_tilde(&shorten_home(&p)), p);
    }
}
