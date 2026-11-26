use std::collections::{BTreeMap, BTreeSet};

use secrecy::{ExposeSecret, SecretSlice, SecretString};

use crate::domain::{
    Entry, FileMode, HomeFile, HomeFiles, HomePath, Kept, SecretId, StoredValue, Target, Template,
    VariableName,
};
use crate::ports::{Boot, VariableMap, home_staging};
use crate::{Error, Result};

/// The most a file may hold, rendered.
pub const FILE_LIMIT: usize = 256 << 10;
/// The most one variable may take as `NAME=value` with its terminating NUL: Linux refuses to
/// start a program with a longer environment string (`MAX_ARG_STRLEN`).
pub const VARIABLE_LIMIT: usize = 128 << 10;
/// The most a sandbox's files may take once staged for its start, which an engine may keep in
/// one Kubernetes Secret.
pub const FILES_LIMIT: usize = 1 << 20;
/// The most a sandbox's variables may take, names included, which an engine may keep in one
/// Kubernetes Secret.
pub const VARIABLES_LIMIT: usize = 1 << 20;

const PLAIN_FILE: u32 = 0o644;
const PLAIN_PROGRAM: u32 = 0o755;
const SECRET_PROGRAM: u32 = 0o700;

/// A sandbox's boot, and the names each of its template files refers to that are not variables
/// attached to it. Such a file is left unrendered.
pub(super) struct Composed {
    pub boot: Boot,
    pub unknown: BTreeMap<HomePath, BTreeSet<VariableName>>,
}

/// What the entries of `whose`, such as `sandbox main`, give it, each with the value it has there,
/// next to the file Corral itself writes. A secret file, or a template that takes a secret, is
/// readable by the sandbox user alone.
pub(super) fn compose(
    whose: &str,
    effective: &[(&Entry, &StoredValue)],
    secrets: &BTreeMap<SecretId, SecretSlice<u8>>,
    own_file: (HomePath, HomeFile),
) -> Result<Composed> {
    check_collisions(whose, effective)?;
    let mut env = VariableMap::new();
    let mut secret_variables = BTreeSet::new();
    let mut files = Vec::new();
    for (entry, value) in effective {
        let bytes = bytes_of(entry, value, secrets)?;
        match &entry.target {
            Target::Variable(name) => {
                let text = std::str::from_utf8(bytes).map_err(|_| {
                    Error::internal("setup", format!("variable {name} is not text"))
                })?;
                if entry.is_secret() || value.is_secret() {
                    secret_variables.insert(name.clone());
                }
                env.insert(name.clone(), SecretString::from(text));
            }
            Target::File(path) => files.push((path, *entry, *value, bytes)),
        }
    }
    let variables: usize = env
        .iter()
        .map(|(name, value)| name.as_str().len() + value.expose_secret().len())
        .sum();
    if variables > VARIABLES_LIMIT {
        return Err(Error::LimitReached(format!(
            "the variables of {whose} take {variables} bytes, more than the {} a sandbox's \
             variables may take",
            amount(VARIABLES_LIMIT)
        )));
    }

    let mut home = HomeFiles::new();
    let mut unknown = BTreeMap::new();
    for (path, entry, value, bytes) in files {
        let mut secret = entry.is_secret() || value.is_secret();
        let content = if entry.template {
            let template = std::str::from_utf8(bytes)
                .map(Template::parse)
                .map_err(|_| Error::internal("setup", format!("template {path} is not text")))?;
            secret |= template
                .names()
                .iter()
                .any(|name| secret_variables.contains(*name));
            match template.render(|name| env.get(name).map(ExposeSecret::expose_secret)) {
                Ok(rendered) => rendered.into_bytes(),
                Err(names) => {
                    unknown.insert(path.clone(), names);
                    bytes.to_vec()
                }
            }
        } else {
            bytes.to_vec()
        };
        if content.len() > FILE_LIMIT {
            return Err(Error::LimitReached(format!(
                "file {path} takes {} bytes in {whose}, more than the {} a file may hold",
                content.len(),
                amount(FILE_LIMIT)
            )));
        }
        let mode = match (secret, entry.executable) {
            (false, false) => FileMode::new(PLAIN_FILE)?,
            (false, true) => FileMode::new(PLAIN_PROGRAM)?,
            (true, false) => FileMode::OWNER_ONLY,
            (true, true) => FileMode::new(SECRET_PROGRAM)?,
        };
        home.insert(
            path.clone(),
            HomeFile {
                mode,
                content: SecretSlice::from(content),
            },
        );
    }
    let (own_path, own) = own_file;
    home.insert(own_path, own);
    let staged = home_staging::stage(&home).size();
    if staged > FILES_LIMIT {
        return Err(Error::LimitReached(format!(
            "the files of {whose} take {staged} bytes once staged, more than the {} a sandbox's \
             files may take",
            amount(FILES_LIMIT)
        )));
    }
    Ok(Composed {
        boot: Boot { env, files: home },
        unknown,
    })
}

fn check_collisions(whose: &str, effective: &[(&Entry, &StoredValue)]) -> Result<()> {
    for (index, (entry, _)) in effective.iter().enumerate() {
        if let Some((other, _)) = effective[index + 1..]
            .iter()
            .find(|(other, _)| other.target.collides(&entry.target))
        {
            return Err(Error::Conflict(format!(
                "{} and {} cannot both be attached to {whose}",
                entry.target, other.target
            )));
        }
    }
    Ok(())
}

/// The variables each template among `effective` refers to that are not among `effective`. A
/// secret missing from `secrets` counts as text without references.
pub(super) fn unresolved(
    effective: &[(&Entry, &StoredValue)],
    secrets: &BTreeMap<SecretId, SecretSlice<u8>>,
) -> BTreeMap<HomePath, BTreeSet<VariableName>> {
    let variables: BTreeSet<&VariableName> = effective
        .iter()
        .filter_map(|(entry, _)| match &entry.target {
            Target::Variable(name) => Some(name),
            Target::File(_) => None,
        })
        .collect();
    effective
        .iter()
        .filter_map(|(entry, value)| match &entry.target {
            Target::File(path) if entry.template => {
                let missing: BTreeSet<VariableName> = template_of(value, secrets)
                    .names()
                    .into_iter()
                    .filter(|name| !variables.contains(name))
                    .cloned()
                    .collect();
                (!missing.is_empty()).then(|| (path.clone(), missing))
            }
            _ => None,
        })
        .collect()
}

/// The value read as a template.
pub(super) fn template_of(
    value: &StoredValue,
    secrets: &BTreeMap<SecretId, SecretSlice<u8>>,
) -> Template {
    let bytes = match &value.kept {
        Kept::Plain(content) => content.as_slice(),
        Kept::Secret(secret) => secrets
            .get(&secret.id)
            .map_or(&[][..], ExposeSecret::expose_secret),
    };
    Template::parse(&String::from_utf8_lossy(bytes))
}

fn bytes_of<'a>(
    entry: &Entry,
    value: &'a StoredValue,
    secrets: &'a BTreeMap<SecretId, SecretSlice<u8>>,
) -> Result<&'a [u8]> {
    match &value.kept {
        Kept::Plain(content) => Ok(content),
        Kept::Secret(secret) => secrets
            .get(&secret.id)
            .map(ExposeSecret::expose_secret)
            .ok_or_else(|| {
                Error::internal(
                    "setup",
                    format!("the secret store has no value for {}", entry.target),
                )
            }),
    }
}

/// A limit in KiB or MiB, as people read it.
pub(super) fn amount(bytes: usize) -> String {
    if bytes.is_multiple_of(1 << 20) {
        format!("{} MiB", bytes >> 20)
    } else {
        format!("{} KiB", bytes >> 10)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use chrono::Utc;
    use secrecy::{ExposeSecret, SecretSlice};

    use super::{FILE_LIMIT, FILES_LIMIT, VARIABLES_LIMIT, compose};
    use crate::Error;
    use crate::domain::{
        Entry, EntryId, FileMode, Glimpse, HomeFile, HomePath, Kept, Owner, SecretId, SecretRef,
        StoredValue, Target, UserId, VariableName,
    };
    use crate::fakes::exposed;

    struct Fixture {
        entries: Vec<(Entry, StoredValue)>,
        secrets: BTreeMap<SecretId, SecretSlice<u8>>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                entries: Vec::new(),
                secrets: BTreeMap::new(),
            }
        }

        fn add(&mut self, target: Target, secret: bool, template: bool, value: &[u8]) {
            let kept = if secret {
                let id = SecretId::generate();
                self.secrets.insert(id, SecretSlice::from(value.to_vec()));
                Kept::Secret(SecretRef {
                    id,
                    size: value.len(),
                    glimpse: Some(Glimpse::of("x")),
                })
            } else {
                Kept::Plain(value.to_vec())
            };
            let value = StoredValue {
                kept,
                set_at: Utc::now(),
            };
            let entry = Entry {
                id: EntryId::generate(),
                owner: Owner::Member(UserId(uuid::Uuid::new_v4())),
                target,
                template,
                executable: false,
                apply_to_new: true,
                value: value.clone(),
                created_at: Utc::now(),
            };
            self.entries.push((entry, value));
        }

        fn compose(&self) -> crate::Result<super::Composed> {
            let effective: Vec<(&Entry, &StoredValue)> =
                self.entries.iter().map(|(e, v)| (e, v)).collect();
            compose("sandbox main", &effective, &self.secrets, own_file())
        }
    }

    fn own_file() -> (HomePath, HomeFile) {
        (
            path(".config/corral/sandbox.json"),
            HomeFile {
                mode: FileMode::OWNER_ONLY,
                content: SecretSlice::from(b"{}".to_vec()),
            },
        )
    }

    fn path(value: &str) -> HomePath {
        HomePath::parse(value).unwrap()
    }

    fn variable(value: &str) -> Target {
        Target::Variable(VariableName::parse(value).unwrap())
    }

    fn file(value: &str) -> Target {
        Target::File(path(value))
    }

    #[test]
    fn variables_and_files_reach_the_boot_with_the_mode_their_secrecy_gives() {
        let mut fixture = Fixture::new();
        fixture.add(variable("PLAIN"), false, false, b"shown");
        fixture.add(variable("SECRET"), true, false, b"hidden");
        fixture.add(file("notes.txt"), false, false, b"notes");
        fixture.add(file(".ssh/id"), true, false, &[0, 159, 146, 150]);
        fixture.add(file(".plain-template"), false, true, b"plain={{ PLAIN }}");
        fixture.add(file(".secret-template"), false, true, b"secret={{SECRET}}");
        let composed = fixture.compose().unwrap();
        assert!(composed.unknown.is_empty());
        assert_eq!(
            exposed(&composed.boot.env),
            ["PLAIN=shown", "SECRET=hidden"]
        );
        let files: Vec<(&str, u32, &[u8])> = composed
            .boot
            .files
            .iter()
            .map(|(path, file)| {
                (
                    path.as_str(),
                    file.mode.bits(),
                    file.content.expose_secret(),
                )
            })
            .collect();
        assert_eq!(
            files,
            [
                (".config/corral/sandbox.json", 0o600, b"{}".as_slice()),
                (".plain-template", 0o644, b"plain=shown".as_slice()),
                (".secret-template", 0o600, b"secret=hidden".as_slice()),
                (".ssh/id", 0o600, [0, 159, 146, 150].as_slice()),
                ("notes.txt", 0o644, b"notes".as_slice()),
            ]
        );
    }

    #[test]
    fn a_template_naming_a_variable_not_attached_is_reported_and_left_unrendered() {
        let mut fixture = Fixture::new();
        fixture.add(variable("KNOWN"), false, false, b"k");
        fixture.add(file("t"), false, true, b"{{ KNOWN }} {{ GONE }} {{ LOST }}");
        let composed = fixture.compose().unwrap();
        assert_eq!(
            composed.unknown,
            BTreeMap::from([(
                path("t"),
                BTreeSet::from([
                    VariableName::parse("GONE").unwrap(),
                    VariableName::parse("LOST").unwrap()
                ])
            )])
        );
    }

    #[test]
    fn colliding_entries_are_refused() {
        for (a, b) in [
            (variable("A"), variable("A")),
            (file(".config"), file(".config/tool")),
        ] {
            let mut fixture = Fixture::new();
            fixture.add(a, false, false, b"1");
            fixture.add(b, true, false, b"2");
            assert!(
                matches!(fixture.compose(), Err(Error::Conflict(message)) if message.contains("main")),
            );
        }
    }

    #[test]
    fn limits_are_refused_naming_them() {
        let mut big_file = Fixture::new();
        big_file.add(variable("V"), false, false, &[b'v'; 10]);
        big_file.add(file("big"), false, true, &vec![b'x'; FILE_LIMIT - 4]);
        assert!(big_file.compose().is_ok());
        big_file.entries.pop();
        big_file.add(
            file("big"),
            false,
            true,
            format!("{}{{{{ V }}}}", "x".repeat(FILE_LIMIT - 4)).as_bytes(),
        );
        let message = limit(big_file.compose());
        assert!(
            message.contains("file big") && message.contains("256 KiB"),
            "{message}"
        );

        let mut many_files = Fixture::new();
        for index in 0..4 {
            many_files.add(
                file(&format!("f{index}")),
                false,
                false,
                &vec![0; FILE_LIMIT],
            );
        }
        let message = limit(many_files.compose());
        assert!(
            message.contains("files of sandbox main") && message.contains("1 MiB"),
            "{message}"
        );

        let mut exactly = Fixture::new();
        let own = "manifest".len() + "0 0600 .config/corral/sandbox.json\n".len() + 1 + 2;
        let mut left = FILES_LIMIT - own;
        for index in 1..=4 {
            let line = format!("{index} 0644 f{index}\n").len() + 1;
            let size = (left - line).min(FILE_LIMIT);
            exactly.add(file(&format!("f{index}")), false, false, &vec![0; size]);
            left -= line + size;
        }
        assert_eq!(left, 0);
        assert!(
            exactly.compose().is_ok(),
            "the staging overhead counts, and fits"
        );
        if let Kept::Plain(content) = &mut exactly.entries.last_mut().unwrap().1.kept {
            content.push(0);
        }
        let message = limit(exactly.compose());
        assert!(message.contains("files of sandbox main"), "{message}");

        let mut variables = Fixture::new();
        variables.add(
            variable("A"),
            false,
            false,
            &vec![b'a'; VARIABLES_LIMIT / 2],
        );
        variables.add(variable("B"), true, false, &vec![b'b'; VARIABLES_LIMIT / 2]);
        let message = limit(variables.compose());
        assert!(
            message.contains("variables of sandbox main") && message.contains("1 MiB"),
            "{message}"
        );
    }

    fn limit(composed: crate::Result<super::Composed>) -> String {
        match composed {
            Err(Error::LimitReached(message)) => message,
            Err(other) => panic!("{other:?}"),
            Ok(_) => panic!("fits"),
        }
    }
}
