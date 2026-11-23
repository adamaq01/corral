//! How an engine hands home files to the sandbox image, whose `corral-home-init` writes them into
//! the home as the sandbox user at every start: a directory holding a `manifest` and each file's
//! bytes under its key. The manifest has a `<key> <mode> <path>` line per file, the mode in octal.

use std::fmt::Write as _;

use secrecy::ExposeSecret;

use crate::domain::{HomeFile, HomeFiles};

pub const DIR: &str = "/etc/corral/home-files";
pub const MANIFEST: &str = "manifest";

pub struct Staged<'a> {
    pub manifest: String,
    /// Each file under its key, a name that is valid in any directory and as a Kubernetes Secret
    /// key.
    pub files: Vec<(String, &'a HomeFile)>,
}

#[must_use]
pub fn stage(files: &HomeFiles) -> Staged<'_> {
    let mut manifest = String::new();
    let mut staged = Vec::with_capacity(files.len());
    for (index, (path, file)) in files.iter().enumerate() {
        let key = index.to_string();
        let _ = writeln!(manifest, "{key} {:04o} {path}", file.mode.bits());
        staged.push((key, file));
    }
    Staged {
        manifest,
        files: staged,
    }
}

impl Staged<'_> {
    /// What the staging takes, counted as a Kubernetes Secret counts its data: each name and
    /// its bytes, the manifest's included.
    #[must_use]
    pub fn size(&self) -> usize {
        let files: usize = self
            .files
            .iter()
            .map(|(key, file)| key.len() + file.content.expose_secret().len())
            .sum();
        MANIFEST.len() + self.manifest.len() + files
    }
}

#[cfg(test)]
mod tests {
    use secrecy::{ExposeSecret, SecretSlice};

    use super::stage;
    use crate::domain::{FileMode, HomeFile, HomeFiles, HomePath};

    #[test]
    fn the_manifest_lists_each_file_by_key_mode_and_path() {
        let file = |mode, content: &str| HomeFile {
            mode: FileMode::new(mode).unwrap(),
            content: SecretSlice::from(content.as_bytes().to_vec()),
        };
        let files = HomeFiles::from([
            (
                HomePath::parse(".config/corral/sandbox.json").unwrap(),
                file(0o600, "{}"),
            ),
            (HomePath::parse("a dir/notes").unwrap(), file(0o644, "hi")),
        ]);
        let staged = stage(&files);
        assert_eq!(
            staged.manifest,
            "0 0600 .config/corral/sandbox.json\n1 0644 a dir/notes\n"
        );
        let keyed: Vec<(&str, &[u8])> = staged
            .files
            .iter()
            .map(|(key, file)| (key.as_str(), file.content.expose_secret()))
            .collect();
        assert_eq!(keyed, [("0", b"{}".as_slice()), ("1", b"hi".as_slice())]);
        assert_eq!(
            staged.size(),
            "manifest".len() + staged.manifest.len() + 1 + 2 + 1 + 2
        );
        assert_eq!(stage(&HomeFiles::new()).manifest, "");
        assert_eq!(stage(&HomeFiles::new()).size(), "manifest".len());
    }
}
