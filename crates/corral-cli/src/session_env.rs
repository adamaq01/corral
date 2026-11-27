//! The sandbox's variables as the engine hands them to SSH sessions, read back from the session
//! env file: sshd `SetEnv` lines of `"NAME=value"` words, with `"` and `\` escaped by `\`.

use std::collections::BTreeMap;

pub const PATH: &str = "/etc/corral/sshd_env.conf";

/// The variables of the `SetEnv` lines in `config`.
pub fn parse(config: &str) -> BTreeMap<String, String> {
    config
        .lines()
        .filter_map(|line| line.strip_prefix("SetEnv "))
        .flat_map(words)
        .filter_map(|word| {
            let (name, value) = word.split_once('=')?;
            Some((name.to_owned(), value.to_owned()))
        })
        .collect()
}

/// The words of `line`, split at spaces outside double quotes.
fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                word.get_or_insert_default();
            }
            '\\' if quoted && matches!(chars.peek(), Some('"' | '\\')) => {
                word.get_or_insert_default().extend(chars.next());
            }
            ' ' | '\t' if !quoted => words.extend(word.take()),
            c => word.get_or_insert_default().push(c),
        }
    }
    words.extend(word);
    words
}
