//! What the overlay has to tell the sandbox user about the sandbox's last start, such as a home
//! file `corral-init` could not write or how the first start's clone went, one line each, which
//! `corral-shell` and `corral status` show. The file is absent when there is nothing.

use std::io::Write;
use std::path::Path;

const PATH: &str = ".config/corral/notices";

/// The notices of the sandbox's last start, as lines.
pub fn read(home: &Path) -> Vec<String> {
    std::fs::read_to_string(home.join(PATH))
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

/// Adds `line` after the notices `corral-init` wrote at this start.
pub fn append(home: &Path, line: &str) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join(PATH))?;
    writeln!(file, "{line}")
}

#[cfg(test)]
mod tests {
    use super::{append, read};

    fn home(test: &str) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!("corral-cli-{test}-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".config/corral")).unwrap();
        home
    }

    #[test]
    fn notices_are_the_lines_corral_init_left() {
        let home = home("notices");
        assert_eq!(read(&home), Vec::<String>::new());
        std::fs::write(
            home.join(".config/corral/notices"),
            "Corral could not write ~/x: a directory is in the way.\n\n~/y was left.\n",
        )
        .unwrap();
        assert_eq!(
            read(&home),
            [
                "Corral could not write ~/x: a directory is in the way.",
                "~/y was left."
            ]
        );
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_notice_is_added_after_the_others_or_alone() {
        let home = home("append");
        append(&home, "Cloned file:///srv/repo.git into ~/repo").unwrap();
        assert_eq!(read(&home), ["Cloned file:///srv/repo.git into ~/repo"]);
        append(&home, "A second line").unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join(".config/corral/notices")).unwrap(),
            "Cloned file:///srv/repo.git into ~/repo\nA second line\n"
        );
        std::fs::remove_dir_all(&home).unwrap();
    }
}
