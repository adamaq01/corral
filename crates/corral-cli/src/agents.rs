//! `corral agents-apply`, which `corral-init` runs at each start after the home files: applies the
//! MCP servers of `~/.config/corral-agents/mcp.json` and the session's gateway addresses to the
//! files Claude Code, Codex, Pi and oh-my-pi read. In each file it replaces only the keys Corral
//! owns, and it writes no file it has nothing to change in.

use std::fs::{OpenOptions, Permissions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use serde_json::{Map, Value};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TomlError};

use crate::session_env;

const SERVERS: &str = ".config/corral-agents/mcp.json";
const CODEX: &str = ".codex/config.toml";
const PI_MODELS: &str = ".pi/agent/models.json";
const MCP_JSON: [(&str, &str); 3] = [
    ("Claude Code", ".claude.json"),
    ("Pi", ".pi/agent/mcp.json"),
    ("oh-my-pi", ".omp/agent/mcp.json"),
];
/// The order the notices name the agents in.
const AGENTS: [&str; 4] = ["Claude Code", "Codex", "Pi", "oh-my-pi"];

type Object = Map<String, Value>;

/// Applies Corral's MCP servers and gateway addresses to the agents' files in `home`, with the
/// variables of the session env file at `session_env`, and says how it went.
pub fn apply(home: &Path, session_env: &Path) -> Vec<String> {
    let mut notices = Vec::new();
    let servers = read(&home.join(SERVERS))
        .and_then(|text| {
            text.as_deref()
                .map_or_else(|| Ok(Object::new()), json_object)
        })
        .unwrap_or_else(|why| {
            notices.push(format!(
                "Could not read the MCP servers from ~/{SERVERS}: {why}"
            ));
            Object::new()
        });
    let variables = session_env::parse(&std::fs::read_to_string(session_env).unwrap_or_default());
    let anthropic = variables.get("ANTHROPIC_BASE_URL").map(String::as_str);
    let openai = variables.get("OPENAI_BASE_URL").map(String::as_str);
    let servers_named = match servers.len() {
        1 => "1 MCP server".to_owned(),
        count => format!("{count} MCP servers"),
    };
    let mut with_servers = Report::new(&servers_named, "the MCP servers");
    let mut with_address = Report::new("the gateway address", "the gateway address");

    if !servers.is_empty() {
        for (agent, file) in MCP_JSON {
            let applied = edit_json(&home.join(file), |root| {
                root.insert("mcpServers".to_owned(), Value::Object(servers.clone()));
                Ok(())
            });
            with_servers.record(agent, file, &applied);
        }
    }
    let codex = edit_toml(&home.join(CODEX), |document| {
        if !servers.is_empty() {
            document.insert("mcp_servers", codex_servers(&servers));
        }
        match openai {
            Some(address) => {
                document.insert("openai_base_url", toml_edit::value(address));
            }
            None => {
                document.remove("openai_base_url");
            }
        }
    });
    if !servers.is_empty() {
        with_servers.record("Codex", CODEX, &codex);
    }
    if openai.is_some() {
        with_address.record("Codex", CODEX, &codex);
    }
    let pi = edit_json(&home.join(PI_MODELS), |root| {
        pi_addresses(root, [("anthropic", anthropic), ("openai", openai)])
    });
    if anthropic.is_some() || openai.is_some() {
        with_address.record("Pi", PI_MODELS, &pi);
        with_address.agents.push("oh-my-pi");
    }
    if anthropic.is_some() {
        with_address.agents.push("Claude Code");
    }

    notices.extend(with_servers.notices());
    notices.extend(with_address.notices());
    notices
}

/// What one thing Corral applies reached: the agents that have it, and why the others do not.
struct Report<'a> {
    applied: &'a str,
    failed: &'a str,
    agents: Vec<&'static str>,
    failures: Vec<String>,
}

impl<'a> Report<'a> {
    fn new(applied: &'a str, failed: &'a str) -> Self {
        Self {
            applied,
            failed,
            agents: Vec::new(),
            failures: Vec::new(),
        }
    }

    fn record(&mut self, agent: &'static str, file: &str, result: &Result<(), String>) {
        match result {
            Ok(()) => self.agents.push(agent),
            Err(why) => self.failures.push(format!(
                "Could not apply {} to ~/{file}: {why}",
                self.failed
            )),
        }
    }

    fn notices(self) -> impl Iterator<Item = String> {
        let agents: Vec<&str> = AGENTS
            .into_iter()
            .filter(|agent| self.agents.contains(agent))
            .collect();
        let applied = match agents.split_last() {
            None => None,
            Some((last, [])) => Some(format!("Applied {} to {last}", self.applied)),
            Some((last, rest)) => Some(format!(
                "Applied {} to {} and {last}",
                self.applied,
                rest.join(", ")
            )),
        };
        applied.into_iter().chain(self.failures)
    }
}

/// Sets each provider's `baseUrl` in Pi's `models.json` to its address, or removes it, and the
/// provider with it once nothing else is left in it, when the address is absent.
fn pi_addresses(root: &mut Object, addresses: [(&str, Option<&str>); 2]) -> Result<(), String> {
    for (provider, address) in addresses {
        if let Some(address) = address {
            let providers = object_at(root, "providers").ok_or("providers is not an object")?;
            object_at(providers, provider)
                .ok_or_else(|| format!("providers.{provider} is not an object"))?
                .insert("baseUrl".to_owned(), address.into());
            continue;
        }
        let Some(providers) = root.get_mut("providers").and_then(Value::as_object_mut) else {
            continue;
        };
        let emptied = providers
            .get_mut(provider)
            .and_then(Value::as_object_mut)
            .is_some_and(|entry| entry.remove("baseUrl").is_some() && entry.is_empty());
        if emptied {
            providers.remove(provider);
        }
    }
    Ok(())
}

fn object_at<'a>(object: &'a mut Object, key: &str) -> Option<&'a mut Object> {
    object
        .entry(key)
        .or_insert_with(|| Value::Object(Object::new()))
        .as_object_mut()
}

fn codex_servers(servers: &Object) -> Item {
    let mut table = Table::new();
    table.set_implicit(true);
    for (name, server) in servers {
        let mut entry = Table::new();
        for (json, toml) in [
            ("command", "command"),
            ("args", "args"),
            ("env", "env"),
            ("url", "url"),
            ("headers", "http_headers"),
        ] {
            if let Some(value) = server.get(json).and_then(toml_value) {
                entry.insert(toml, Item::Value(value));
            }
        }
        table.insert(name, Item::Table(entry));
    }
    Item::Table(table)
}

fn toml_value(json: &Value) -> Option<toml_edit::Value> {
    Some(match json {
        Value::Null => return None,
        Value::Bool(flag) => (*flag).into(),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => integer.into(),
            None => number.as_f64()?.into(),
        },
        Value::String(text) => text.as_str().into(),
        Value::Array(items) => items
            .iter()
            .filter_map(toml_value)
            .collect::<Array>()
            .into(),
        Value::Object(fields) => fields
            .iter()
            .filter_map(|(key, value)| Some((key.as_str(), toml_value(value)?)))
            .collect::<InlineTable>()
            .into(),
    })
}

/// Edits the JSON object in `path` and writes it back if that changed it. An absent file is an
/// empty object, created only if the edit puts something in it.
fn edit_json(
    path: &Path,
    edit: impl FnOnce(&mut Object) -> Result<(), String>,
) -> Result<(), String> {
    let before = read(path)?
        .as_deref()
        .map_or_else(|| Ok(Object::new()), json_object)?;
    let mut after = before.clone();
    edit(&mut after)?;
    if after == before {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(&after).map_err(|error| error.to_string())?;
    write(path, &format!("{text}\n"))
}

/// Edits the TOML document in `path`, keeping its formatting, and writes it back if that changed
/// it. An absent file is an empty document, created only if the edit puts something in it.
fn edit_toml(path: &Path, edit: impl FnOnce(&mut DocumentMut)) -> Result<(), String> {
    let before = read(path)?.unwrap_or_default();
    let mut document: DocumentMut = before
        .parse()
        .map_err(|error| toml_reason(&before, &error))?;
    edit(&mut document);
    let after = document.to_string();
    if after == before {
        return Ok(());
    }
    write(path, &after)
}

/// Why `text` is not valid TOML, in one line. TOML's own display quotes the line, which may hold
/// a token.
fn toml_reason(text: &str, error: &TomlError) -> String {
    let at = error.span().map_or(0, |span| span.start);
    let before = text.get(..at).unwrap_or_default();
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count()
        + 1;
    format!(
        "{} at line {line} column {column}",
        error.message().trim().replace('\n', " ")
    )
}

fn json_object(text: &str) -> Result<Object, String> {
    match serde_json::from_str(text).map_err(|error| error.to_string())? {
        Value::Object(object) => Ok(object),
        _ => Err("it is not a JSON object".to_owned()),
    }
}

fn read(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Replaces the file at `path`, or the file a link there points to, with one holding `text`. It
/// keeps the mode of the file it replaces, and a new one is the sandbox user's alone, since the
/// servers may carry tokens.
fn write(path: &Path, text: &str) -> Result<(), String> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".corral-new");
    let new = path.with_file_name(name);
    let mode = std::fs::metadata(&path).map_or(0o600, |metadata| metadata.permissions().mode());
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&new)
        })
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .and_then(|()| std::fs::set_permissions(&new, Permissions::from_mode(mode)))
        .and_then(|()| std::fs::rename(&new, &path));
    written.map_err(|error| {
        let _ = std::fs::remove_file(&new);
        error.to_string()
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use corral_core::domain::VariableName;
    use corral_core::ports::session_env::sshd_config;
    use secrecy::SecretString;
    use serde_json::{Value, json};

    use super::apply;
    use crate::session_env;

    const MCP: &str = ".config/corral-agents/mcp.json";
    const CODEX: &str = ".codex/config.toml";
    const PI_MODELS: &str = ".pi/agent/models.json";
    const MCP_JSON: [&str; 3] = [".claude.json", ".pi/agent/mcp.json", ".omp/agent/mcp.json"];
    const SERVERS: &str = r#"{
        "fetch": {"command": "uvx", "args": ["mcp-server-fetch"], "env": {"FETCH_TOKEN": "t0ken"}},
        "docs": {"type": "http", "url": "https://docs.example/mcp",
                 "headers": {"Authorization": "Bearer t0ken"}}
    }"#;
    const BOTH_ADDRESSES: &str = "SetEnv \"ANTHROPIC_API_KEY=sk-ant\" \
        \"ANTHROPIC_BASE_URL=https://llm.home.example/anthropic\" \
        \"OPENAI_BASE_URL=https://llm.home.example/openai\"\n";

    /// An empty home, and where its session env file goes, none written yet.
    fn sandbox(test: &str) -> (PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("corral-cli-agents-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        (dir.join("home"), dir.join("sshd_env.conf"))
    }

    fn clean(home: &Path) {
        std::fs::remove_dir_all(home.parent().unwrap()).unwrap();
    }

    fn put(home: &Path, file: &str, text: &str) {
        let path = home.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn text_at(home: &Path, file: &str) -> String {
        std::fs::read_to_string(home.join(file)).unwrap()
    }

    fn json_at(home: &Path, file: &str) -> Value {
        serde_json::from_str(&text_at(home, file)).unwrap()
    }

    fn servers() -> Value {
        serde_json::from_str(SERVERS).unwrap()
    }

    #[test]
    fn merges_mcp_servers_into_the_three_json_files_keeping_other_keys() {
        let (home, env) = sandbox("merges");
        put(&home, MCP, SERVERS);
        let claude = json!({
            "numStartups": 3,
            "projects": {"/home/sandbox/x": {"allowedTools": ["Bash"]}},
            "mcpServers": {"old": {"command": "old"}},
        });
        let pi =
            json!({"settings": {"toolPrefix": "mcp"}, "mcpServers": {"old": {"command": "old"}}});
        let omp = json!({"$schema": "https://example.com/omp-mcp.json"});
        put(&home, ".claude.json", &claude.to_string());
        put(&home, ".pi/agent/mcp.json", &pi.to_string());
        put(&home, ".omp/agent/mcp.json", &omp.to_string());

        assert_eq!(
            apply(&home, &env),
            ["Applied 2 MCP servers to Claude Code, Codex, Pi and oh-my-pi"]
        );
        for (file, mut before) in [
            (".claude.json", claude),
            (".pi/agent/mcp.json", pi),
            (".omp/agent/mcp.json", omp),
        ] {
            before["mcpServers"] = servers();
            assert_eq!(json_at(&home, file), before, "{file}");
        }
        clean(&home);
    }

    #[test]
    fn creates_the_json_files_when_absent() {
        let (home, env) = sandbox("creates");
        put(&home, MCP, SERVERS);

        assert_eq!(
            apply(&home, &env),
            ["Applied 2 MCP servers to Claude Code, Codex, Pi and oh-my-pi"]
        );
        for file in MCP_JSON {
            assert_eq!(
                json_at(&home, file),
                json!({"mcpServers": servers()}),
                "{file}"
            );
            let mode = std::fs::metadata(home.join(file))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{file}");
        }
        assert!(!home.join(PI_MODELS).exists());
        clean(&home);
    }

    #[test]
    fn replaces_codex_mcp_servers_and_keeps_the_rest() {
        let (home, env) = sandbox("codex-servers");
        put(&home, MCP, SERVERS);
        put(
            &home,
            CODEX,
            "# Mine.\nmodel = \"gpt-5\"\n\n[projects.\"/home/sandbox/x\"]\n\
             trust_level = \"trusted\"\n\n[mcp_servers.old]\ncommand = \"old\"\n",
        );
        let expected = "# Mine.\nmodel = \"gpt-5\"\n\n[projects.\"/home/sandbox/x\"]\n\
             trust_level = \"trusted\"\n\n\
             [mcp_servers.docs]\nurl = \"https://docs.example/mcp\"\n\
             http_headers = { Authorization = \"Bearer t0ken\" }\n\n\
             [mcp_servers.fetch]\ncommand = \"uvx\"\nargs = [\"mcp-server-fetch\"]\n\
             env = { FETCH_TOKEN = \"t0ken\" }\n";

        apply(&home, &env);
        assert_eq!(text_at(&home, CODEX), expected);
        apply(&home, &env);
        assert_eq!(text_at(&home, CODEX), expected);
        clean(&home);
    }

    #[test]
    fn sets_and_removes_openai_base_url() {
        let (home, env) = sandbox("codex-address");
        let mine =
            "model = \"gpt-5\"\n\n[projects.\"/home/sandbox/x\"]\ntrust_level = \"trusted\"\n";
        put(&home, CODEX, mine);

        std::fs::write(
            &env,
            "SetEnv \"OPENAI_API_KEY=sk-1\" \"OPENAI_BASE_URL=https://llm.home.example/openai\"\n",
        )
        .unwrap();
        assert_eq!(
            apply(&home, &env),
            ["Applied the gateway address to Codex, Pi and oh-my-pi"]
        );
        assert_eq!(
            text_at(&home, CODEX),
            "model = \"gpt-5\"\nopenai_base_url = \"https://llm.home.example/openai\"\n\n\
             [projects.\"/home/sandbox/x\"]\ntrust_level = \"trusted\"\n"
        );

        std::fs::write(&env, "SetEnv \"OPENAI_API_KEY=sk-1\"\n").unwrap();
        assert_eq!(apply(&home, &env), Vec::<String>::new());
        assert_eq!(text_at(&home, CODEX), mine);
        clean(&home);
    }

    #[test]
    fn sets_and_removes_pi_provider_addresses_keeping_other_providers() {
        let (home, env) = sandbox("pi-addresses");
        let mine = json!({"providers": {
            "ollama": {
                "baseUrl": "http://localhost:11434/v1",
                "api": "openai-completions",
                "models": [{"id": "llama3.1:8b"}],
            },
            "anthropic": {"headers": {"x-team": "corral"}},
        }});
        put(&home, PI_MODELS, &mine.to_string());

        std::fs::write(&env, BOTH_ADDRESSES).unwrap();
        assert_eq!(
            apply(&home, &env),
            ["Applied the gateway address to Claude Code, Codex, Pi and oh-my-pi"]
        );
        let mut addressed = mine.clone();
        addressed["providers"]["anthropic"]["baseUrl"] =
            json!("https://llm.home.example/anthropic");
        addressed["providers"]["openai"] = json!({"baseUrl": "https://llm.home.example/openai"});
        assert_eq!(json_at(&home, PI_MODELS), addressed);

        std::fs::write(&env, "").unwrap();
        assert_eq!(apply(&home, &env), Vec::<String>::new());
        assert_eq!(json_at(&home, PI_MODELS), mine);
        clean(&home);
    }

    #[test]
    fn touches_nothing_without_servers_or_addresses() {
        let (home, env) = sandbox("untouched");
        let files = [
            (
                ".claude.json",
                r#"{"numStartups": 3, "mcpServers": {"mine": {"command": "mine"}}}"#,
            ),
            (
                CODEX,
                "model = \"gpt-5\"\n\n[mcp_servers.mine]\ncommand = \"mine\"\n",
            ),
            (
                PI_MODELS,
                r#"{"providers": {"ollama": {"baseUrl": "http://localhost:11434/v1"}}}"#,
            ),
        ];
        for (file, text) in files {
            put(&home, file, text);
        }
        let modified = |file: &str| {
            std::fs::metadata(home.join(file))
                .unwrap()
                .modified()
                .unwrap()
        };
        let before = files.map(|(file, _)| modified(file));
        std::fs::write(&env, "SetEnv \"GH_TOKEN=gh\"\n").unwrap();

        assert_eq!(apply(&home, &env), Vec::<String>::new());
        for ((file, text), before) in files.into_iter().zip(before) {
            assert_eq!(text_at(&home, file), text, "{file}");
            assert_eq!(modified(file), before, "{file}");
        }
        assert!(!home.join(".pi/agent/mcp.json").exists());
        assert!(!home.join(".omp").exists());
        clean(&home);

        let (home, env) = sandbox("untouched-empty");
        assert_eq!(apply(&home, &env), Vec::<String>::new());
        assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0);
        clean(&home);
    }

    #[test]
    fn a_corrupt_agent_file_is_left_alone_with_a_notice() {
        let (home, env) = sandbox("corrupt");
        put(&home, MCP, r#"{"fetch": {"command": "uvx"}}"#);
        let claude = r#"{"numStartups": 3, "oauthAccount": "#;
        let codex = "model = \"gpt-5\"\n[mcp_servers.x]\n\
                     http_headers = { Authorization = \"Bearer s3cret }\n";
        put(&home, ".claude.json", claude);
        put(&home, CODEX, codex);
        std::fs::write(
            &env,
            "SetEnv \"OPENAI_BASE_URL=https://llm.home.example/openai\"\n",
        )
        .unwrap();

        let notices = apply(&home, &env);
        assert_eq!(notices.len(), 5, "{notices:?}");
        assert_eq!(notices[0], "Applied 1 MCP server to Pi and oh-my-pi");
        assert_eq!(notices[3], "Applied the gateway address to Pi and oh-my-pi");
        for (notice, failed) in [
            (
                &notices[1],
                "Could not apply the MCP servers to ~/.claude.json: ",
            ),
            (
                &notices[2],
                "Could not apply the MCP servers to ~/.codex/config.toml: ",
            ),
            (
                &notices[4],
                "Could not apply the gateway address to ~/.codex/config.toml: ",
            ),
        ] {
            let why = notice.strip_prefix(failed);
            assert!(why.is_some_and(|why| !why.is_empty()), "{notice}");
            assert!(
                !notice.contains("s3cret") && !notice.contains('\n'),
                "{notice}"
            );
        }
        assert_eq!(text_at(&home, ".claude.json"), claude);
        assert_eq!(text_at(&home, CODEX), codex);
        clean(&home);
    }

    #[test]
    fn session_env_parse_round_trips_sshd_config() {
        let variables = [
            ("ANTHROPIC_BASE_URL", "https://llm.home.example/anthropic"),
            ("EMPTY", ""),
            ("ODD_TOKEN", r#"p#ss "word" it's $HOME \x\"#),
            ("SPACED", "a  b"),
        ];
        let env = variables
            .iter()
            .map(|(name, value)| {
                (
                    VariableName::parse(name).unwrap(),
                    SecretString::from(*value),
                )
            })
            .collect();

        assert_eq!(
            session_env::parse(&sshd_config(&env)),
            variables
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>()
        );
        assert_eq!(session_env::parse(""), BTreeMap::new());
    }
}
