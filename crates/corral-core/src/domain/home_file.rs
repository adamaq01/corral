use std::collections::BTreeMap;

use secrecy::SecretSlice;

use crate::{Error, Result};

const MAX_LEN: usize = 1024;

validated_string!(HomePath, check);

impl HomePath {
    /// Whether the two are one path, or one is a directory the other sits in.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        let inside = |path: &str, dir: &str| {
            path.strip_prefix(dir)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        };
        inside(self.as_str(), other.as_str()) || inside(other.as_str(), self.as_str())
    }
}

/// What sits at a path in the home: its permission bits and its bytes.
#[derive(Clone, Debug)]
pub struct HomeFile {
    pub mode: FileMode,
    pub content: SecretSlice<u8>,
}

/// The files a sandbox gets in its home at a start, one per path.
pub type HomeFiles = BTreeMap<HomePath, HomeFile>;

/// Permission bits only: no setuid, setgid or sticky bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileMode(u32);

impl FileMode {
    pub const OWNER_ONLY: Self = Self(0o600);

    pub fn new(bits: u32) -> Result<Self> {
        if bits & !0o777 != 0 {
            return Err(Error::Invalid(format!(
                "file mode {bits:o} may only have permission bits (0777)"
            )));
        }
        Ok(Self(bits))
    }

    #[must_use]
    pub fn bits(self) -> u32 {
        self.0
    }
}

/// Relative to the home and inside it: no leading slash, no empty, `.` or `..` part, no control
/// character, and no leading or trailing space.
fn check(value: &str) -> Result<()> {
    let inside = !value.is_empty()
        && value.len() <= MAX_LEN
        && value
            .split('/')
            .all(|part| !matches!(part, "" | "." | ".."))
        && !value.chars().any(char::is_control)
        && value.trim() == value;
    if inside {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "{value:?} must be a path inside the home, such as .config/tool/config.toml"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{FileMode, HomePath};
    use crate::Error;

    #[test]
    fn home_paths_stay_inside_the_home() {
        for good in [
            ".config/corral/sandbox.json",
            "notes.txt",
            "a dir/with spaces/file",
            "..hidden",
        ] {
            assert_eq!(HomePath::parse(good).unwrap().as_str(), good);
        }
        let long = "a".repeat(1025);
        for bad in [
            "",
            "/etc/passwd",
            "../outside",
            "a/../../outside",
            "a//b",
            "a/./b",
            "dir/",
            "line\nbreak",
            "nul\0byte",
            " leading",
            "trailing ",
            long.as_str(),
        ] {
            assert!(
                matches!(HomePath::parse(bad), Err(Error::Invalid(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn file_modes_are_permission_bits_only() {
        assert_eq!(FileMode::new(0o644).unwrap().bits(), 0o644);
        assert_eq!(FileMode::OWNER_ONLY.bits(), 0o600);
        for bad in [0o4755, 0o1777, 0o10000] {
            assert!(matches!(FileMode::new(bad), Err(Error::Invalid(_))));
        }
    }
}
