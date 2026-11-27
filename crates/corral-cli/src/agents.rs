//! `corral agents-apply`, which `corral-init` runs at each start after the home files: applies the
//! MCP servers of `~/.config/corral-agents/mcp.json` and the session's gateway addresses to the
//! files Claude Code, Codex, Pi and oh-my-pi read. In each file it sets only what Corral owns, its
//! MCP servers by name and the gateway addresses, and it writes no file it has nothing to change
//! in. It records the servers it applied and the addresses it wrote to each file, so that a later
//! start removes those the setup no longer has and no others.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{OpenOptions, Permissions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TomlError};

use crate::session_env;

const SERVERS: &str = ".config/corral-agents/mcp.json";
const APPLIED: &str = ".config/corral/agents-applied.json";
const CODEX: &str = ".codex/config.toml";
const PI_MODELS: &str = ".pi/agent/models.json";
const ANTHROPIC: &str = "ANTHROPIC_BASE_URL";
const OPENAI: &str = "OPENAI_BASE_URL";
const MCP_JSON: [(&str, &str); 3] = [
    ("Claude Code", ".claude.json"),
    ("Pi", ".pi/agent/mcp.json"),
    ("oh-my-pi", ".omp/agent/mcp.json"),
];
/// The order the notices name the agents in.
const AGENTS: [&str; 4] = ["Claude Code", "Codex", "Pi", "oh-my-pi"];

type Object = Map<String, Value>;

/// What Corral does to the agents' MCP servers at this start.
enum Servers<'a> {
    /// Sets each of `set` under its name, replacing a server of that name, and removes `stale`,
    /// the names an earlier start applied that `set` no longer has. Other servers stay.
    Merge {
        set: &'a Object,
        stale: BTreeSet<String>,
    },
    Keep,
}

/// The gateway addresses Corral wrote to one agent file, each by the variable it came from.
type Addresses = BTreeMap<String, String>;

/// What Corral applied at a start, recorded in `~/.config/corral/agents-applied.json`: the names
/// of its MCP servers, and the addresses each agent file holds, by its path in the home.
#[derive(Default, PartialEq, Serialize, Deserialize)]
struct Applied {
    #[serde(
        rename = "mcpServers",
        default,
        skip_serializing_if = "BTreeSet::is_empty"
    )]
    servers: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    addresses: BTreeMap<String, Addresses>,
}

impl Applied {
    /// The record for `file` after this start: the `addresses` set in it when its edit
    /// succeeded, else the record from before, since the file still holds those.
    fn addresses_in(
        &self,
        file: &str,
        edited: &Result<(), String>,
        addresses: &[(&str, Address)],
    ) -> Option<(String, Addresses)> {
        let held: Addresses = match edited {
            Ok(()) => addresses
                .iter()
                .filter_map(|(variable, address)| match address {
                    Address::Set(address) => Some(((*variable).to_owned(), (*address).to_owned())),
                    Address::Unset(_) | Address::Keep => None,
                })
                .collect(),
            Err(_) => self.addresses.get(file).cloned().unwrap_or_default(),
        };
        (!held.is_empty()).then(|| (file.to_owned(), held))
    }
}

/// What Corral does to one gateway address in an agent's file at this start.
#[derive(Clone, Copy)]
enum Address<'a> {
    Set(&'a str),
    /// Removes the address if it is still the one an earlier start set.
    Unset(&'a str),
    /// Leaves whatever is there: Corral never set it.
    Keep,
}

impl<'a> Address<'a> {
    fn of(
        variable: &str,
        session: &'a BTreeMap<String, String>,
        applied: Option<&'a Addresses>,
    ) -> Self {
        match (
            session.get(variable),
            applied.and_then(|applied| applied.get(variable)),
        ) {
            (Some(address), _) => Self::Set(address),
            (None, Some(address)) => Self::Unset(address),
            (None, None) => Self::Keep,
        }
    }

    fn is_set(self) -> bool {
        matches!(self, Self::Set(_))
    }
}

/// Applies Corral's MCP servers and gateway addresses to the agents' files in `home`, with the
/// variables of the session env file at `session_env`, and says how it went.
pub fn apply(home: &Path, session_env: &Path) -> Vec<String> {
    let mut notices = Vec::new();
    let applied = applied_before(home);
    let servers = read(&home.join(SERVERS))
        .and_then(|text| {
            text.as_deref()
                .map_or_else(|| Ok(Object::new()), json_object)
        })
        .inspect_err(|why| {
            notices.push(format!(
                "Could not read the MCP servers from ~/{SERVERS}: {why}"
            ));
        })
        .ok();
    let action = match &servers {
        Some(set) => {
            let stale: BTreeSet<String> = applied
                .servers
                .iter()
                .filter(|name| !set.contains_key(*name))
                .cloned()
                .collect();
            if set.is_empty() && stale.is_empty() {
                Servers::Keep
            } else {
                Servers::Merge { set, stale }
            }
        }
        None => Servers::Keep,
    };
    let variables = session_env::parse(&std::fs::read_to_string(session_env).unwrap_or_default());
    let codex_openai = Address::of(OPENAI, &variables, applied.addresses.get(CODEX));
    let pi_anthropic = Address::of(ANTHROPIC, &variables, applied.addresses.get(PI_MODELS));
    let pi_openai = Address::of(OPENAI, &variables, applied.addresses.get(PI_MODELS));
    let servers_named = match &action {
        Servers::Merge { set, .. } if set.len() == 1 => Some("1 MCP server".to_owned()),
        Servers::Merge { set, .. } if set.len() > 1 => Some(format!("{} MCP servers", set.len())),
        Servers::Merge { .. } | Servers::Keep => None,
    };
    let mut with_servers = Report::new(servers_named.as_deref(), "the MCP servers");
    let mut with_address = Report::new(Some("the gateway address"), "the gateway address");

    if let Servers::Merge { set, stale } = &action {
        for (agent, file) in MCP_JSON {
            let edited = edit_json(&home.join(file), |root| {
                merge_json(root, set, stale);
                Ok(())
            });
            with_servers.record(agent, file, &edited);
        }
    }
    let codex = edit_toml(&home.join(CODEX), |document| {
        if let Servers::Merge { set, stale } = &action {
            merge_codex(document, set, stale);
        }
        codex_address(document, codex_openai);
    });
    if !matches!(action, Servers::Keep) {
        with_servers.record("Codex", CODEX, &codex);
    }
    if codex_openai.is_set() {
        with_address.record("Codex", CODEX, &codex);
    }
    let pi = edit_json(&home.join(PI_MODELS), |root| {
        pi_addresses(root, [("anthropic", pi_anthropic), ("openai", pi_openai)])
    });
    if pi_anthropic.is_set() || pi_openai.is_set() {
        with_address.record("Pi", PI_MODELS, &pi);
        with_address.agents.push("oh-my-pi");
    }
    if pi_anthropic.is_set() {
        with_address.agents.push("Claude Code");
    }

    let mut now = Applied {
        servers: servers.as_ref().map_or_else(
            || applied.servers.clone(),
            |servers| servers.keys().cloned().collect(),
        ),
        addresses: [
            applied.addresses_in(CODEX, &codex, &[(OPENAI, codex_openai)]),
            applied.addresses_in(
                PI_MODELS,
                &pi,
                &[(ANTHROPIC, pi_anthropic), (OPENAI, pi_openai)],
            ),
        ]
        .into_iter()
        .flatten()
        .collect(),
    };
    if !with_servers.failures.is_empty() {
        now.servers.extend(applied.servers.iter().cloned());
    }
    if let Err(why) = record_applied(home, &applied, &now) {
        notices.push(format!(
            "Could not record the MCP servers and addresses applied in ~/{APPLIED}: {why}"
        ));
    }
    notices.extend(with_servers.notices());
    notices.extend(with_address.notices());
    notices
}

/// What an earlier start applied, nothing when it applied nothing or the record cannot be read.
fn applied_before(home: &Path) -> Applied {
    read(&home.join(APPLIED))
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Replaces the record of what was applied, `before`, with `now`, removing it when it is empty.
fn record_applied(home: &Path, before: &Applied, now: &Applied) -> Result<(), String> {
    let path = home.join(APPLIED);
    if *now == Applied::default() {
        return match std::fs::remove_file(&path) {
            Err(error) if error.kind() != ErrorKind::NotFound => Err(error.to_string()),
            _ => Ok(()),
        };
    }
    if now == before {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(now).map_err(|error| error.to_string())?;
    write(&path, &format!("{text}\n"))
}

/// What one thing Corral applies reached: the agents that have it, and why the others do not.
/// Without `applied`, as for a removal, only the failures are told.
struct Report<'a> {
    applied: Option<&'a str>,
    failed: &'a str,
    agents: Vec<&'static str>,
    failures: Vec<String>,
}

impl<'a> Report<'a> {
    fn new(applied: Option<&'a str>, failed: &'a str) -> Self {
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
        let applied = self.applied.and_then(|applied| match agents.split_last() {
            None => None,
            Some((last, [])) => Some(format!("Applied {applied} to {last}")),
            Some((last, rest)) => Some(format!(
                "Applied {applied} to {} and {last}",
                rest.join(", ")
            )),
        });
        applied.into_iter().chain(self.failures)
    }
}

/// Sets or removes `openai_base_url` in Codex's `config.toml` as the address says.
fn codex_address(document: &mut DocumentMut, address: Address) {
    match address {
        Address::Set(address) => {
            document.insert("openai_base_url", toml_edit::value(address));
        }
        Address::Unset(address) => {
            if document.get("openai_base_url").and_then(Item::as_str) == Some(address) {
                document.remove("openai_base_url");
            }
        }
        Address::Keep => {}
    }
}

/// Sets or removes each provider's `baseUrl` in Pi's `models.json` as its address says, removing
/// the provider too once nothing else is left in it.
fn pi_addresses(root: &mut Object, addresses: [(&str, Address); 2]) -> Result<(), String> {
    for (provider, address) in addresses {
        match address {
            Address::Set(address) => {
                let providers = object_at(root, "providers").ok_or("providers is not an object")?;
                object_at(providers, provider)
                    .ok_or_else(|| format!("providers.{provider} is not an object"))?
                    .insert("baseUrl".to_owned(), address.into());
            }
            Address::Unset(address) => {
                let Some(providers) = root.get_mut("providers").and_then(Value::as_object_mut)
                else {
                    continue;
                };
                let emptied = providers
                    .get_mut(provider)
                    .and_then(Value::as_object_mut)
                    .filter(|entry| entry.get("baseUrl").and_then(Value::as_str) == Some(address))
                    .is_some_and(|entry| {
                        entry.remove("baseUrl");
                        entry.is_empty()
                    });
                if emptied {
                    providers.remove(provider);
                }
            }
            Address::Keep => {}
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

fn merge_json(root: &mut Object, set: &Object, stale: &BTreeSet<String>) {
    if !set.is_empty() && !root.get("mcpServers").is_some_and(Value::is_object) {
        root.insert("mcpServers".to_owned(), Value::Object(Object::new()));
    }
    if let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) {
        servers.retain(|name, _| !stale.contains(name));
        servers.extend(set.clone());
    }
}

fn merge_codex(document: &mut DocumentMut, set: &Object, stale: &BTreeSet<String>) {
    if !set.is_empty() && !document.get("mcp_servers").is_some_and(Item::is_table_like) {
        let mut servers = Table::new();
        servers.set_implicit(true);
        document.insert("mcp_servers", Item::Table(servers));
    }
    if let Some(servers) = document
        .get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
    {
        for name in stale {
            servers.remove(name);
        }
        for (name, server) in set {
            servers.insert(name, codex_server(server));
        }
    }
}

fn codex_server(server: &Value) -> Item {
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
    Item::Table(entry)
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
        for (file, mut expected) in [
            (".claude.json", claude),
            (".pi/agent/mcp.json", pi),
            (".omp/agent/mcp.json", omp),
        ] {
            for (name, server) in servers().as_object().unwrap() {
                expected["mcpServers"][name] = server.clone();
            }
            assert_eq!(json_at(&home, file), expected, "{file}");
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
    fn merges_codex_mcp_servers_and_keeps_the_rest() {
        let (home, env) = sandbox("codex-servers");
        put(&home, MCP, SERVERS);
        put(
            &home,
            CODEX,
            "# Mine.\nmodel = \"gpt-5\"\n\n[projects.\"/home/sandbox/x\"]\n\
             trust_level = \"trusted\"\n\n[mcp_servers.old]\ncommand = \"old\"\n",
        );
        let expected = "# Mine.\nmodel = \"gpt-5\"\n\n[projects.\"/home/sandbox/x\"]\n\
             trust_level = \"trusted\"\n\n[mcp_servers.old]\ncommand = \"old\"\n\n\
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
    fn an_address_corral_never_set_stays_when_the_variable_is_absent() {
        let (home, env) = sandbox("never-set");
        let codex = "model = \"gpt-5\"\nopenai_base_url = \"https://mine.example/openai\"\n";
        let pi = json!({"providers": {
            "anthropic": {"baseUrl": "https://mine.example/anthropic"},
            "openai": {"baseUrl": "https://mine.example/openai"},
        }});
        put(&home, CODEX, codex);
        put(&home, PI_MODELS, &pi.to_string());
        std::fs::write(&env, "SetEnv \"OPENAI_API_KEY=sk-1\"\n").unwrap();

        assert_eq!(apply(&home, &env), Vec::<String>::new());
        assert_eq!(text_at(&home, CODEX), codex);
        assert_eq!(json_at(&home, PI_MODELS), pi);

        std::fs::write(&env, BOTH_ADDRESSES).unwrap();
        apply(&home, &env);
        put(&home, CODEX, codex);
        put(&home, PI_MODELS, &pi.to_string());
        std::fs::write(&env, "").unwrap();
        assert_eq!(apply(&home, &env), Vec::<String>::new());
        assert_eq!(text_at(&home, CODEX), codex);
        assert_eq!(json_at(&home, PI_MODELS), pi);
        clean(&home);
    }

    #[test]
    fn an_address_corral_set_goes_when_the_variable_goes() {
        let (home, env) = sandbox("set-goes");
        put(&home, CODEX, "model = \"gpt-5\"\n");
        std::fs::write(&env, BOTH_ADDRESSES).unwrap();
        apply(&home, &env);

        std::fs::write(&env, "").unwrap();
        assert_eq!(apply(&home, &env), Vec::<String>::new());
        assert_eq!(text_at(&home, CODEX), "model = \"gpt-5\"\n");
        assert_eq!(json_at(&home, PI_MODELS), json!({"providers": {}}));

        let mine = "model = \"gpt-5\"\nopenai_base_url = \"https://llm.home.example/openai\"\n";
        put(&home, CODEX, mine);
        assert_eq!(apply(&home, &env), Vec::<String>::new());
        assert_eq!(text_at(&home, CODEX), mine);
        clean(&home);
    }

    #[test]
    fn a_failed_write_keeps_the_address_the_file_still_holds() {
        let (home, env) = sandbox("failed-write");
        put(&home, CODEX, "model = \"gpt-5\"\n");
        std::fs::write(&env, BOTH_ADDRESSES).unwrap();
        apply(&home, &env);

        let corrupt =
            "model = \"gpt-5\"\nopenai_base_url = \"https://llm.home.example/openai\"\n[x\n";
        put(&home, CODEX, corrupt);
        std::fs::write(
            &env,
            "SetEnv \"ANTHROPIC_BASE_URL=https://llm2.home.example/anthropic\" \
             \"OPENAI_BASE_URL=https://llm2.home.example/openai\"\n",
        )
        .unwrap();
        let notices = apply(&home, &env);
        assert_eq!(notices.len(), 2, "{notices:?}");
        assert_eq!(
            notices[0],
            "Applied the gateway address to Claude Code, Pi and oh-my-pi"
        );
        assert!(
            notices[1].starts_with("Could not apply the gateway address to ~/.codex/config.toml: "),
            "{}",
            notices[1]
        );
        assert_eq!(text_at(&home, CODEX), corrupt);
        assert_eq!(
            json_at(&home, PI_MODELS),
            json!({"providers": {
                "anthropic": {"baseUrl": "https://llm2.home.example/anthropic"},
                "openai": {"baseUrl": "https://llm2.home.example/openai"},
            }})
        );

        put(
            &home,
            CODEX,
            "model = \"gpt-5\"\nopenai_base_url = \"https://llm.home.example/openai\"\n",
        );
        std::fs::write(&env, "").unwrap();
        assert_eq!(apply(&home, &env), Vec::<String>::new());
        assert_eq!(text_at(&home, CODEX), "model = \"gpt-5\"\n");
        assert_eq!(json_at(&home, PI_MODELS), json!({"providers": {}}));
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
    fn removed_servers_leave_the_agent_files() {
        let (home, env) = sandbox("removed");
        let claude = json!({"numStartups": 3});
        put(&home, ".claude.json", &claude.to_string());
        put(&home, CODEX, "model = \"gpt-5\"\n");
        put(&home, MCP, SERVERS);
        apply(&home, &env);

        put(
            &home,
            MCP,
            r#"{"fetch": {"command": "uvx", "args": ["mcp-server-fetch"]}}"#,
        );
        assert_eq!(
            apply(&home, &env),
            ["Applied 1 MCP server to Claude Code, Codex, Pi and oh-my-pi"]
        );
        let fetch = json!({"fetch": {"command": "uvx", "args": ["mcp-server-fetch"]}});
        for file in MCP_JSON {
            assert_eq!(json_at(&home, file)["mcpServers"], fetch, "{file}");
        }

        std::fs::remove_file(home.join(MCP)).unwrap();
        assert_eq!(apply(&home, &env), Vec::<String>::new());
        let mut emptied = claude.clone();
        emptied["mcpServers"] = json!({});
        assert_eq!(json_at(&home, ".claude.json"), emptied);
        for file in [".pi/agent/mcp.json", ".omp/agent/mcp.json"] {
            assert_eq!(json_at(&home, file), json!({"mcpServers": {}}), "{file}");
        }
        assert_eq!(text_at(&home, CODEX), "model = \"gpt-5\"\n");

        let modified = std::fs::metadata(home.join(CODEX))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(apply(&home, &env), Vec::<String>::new());
        let unchanged = std::fs::metadata(home.join(CODEX))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(unchanged, modified);
        clean(&home);
    }

    #[test]
    fn servers_the_member_added_by_hand_stay() {
        let (home, env) = sandbox("by-hand");
        put(&home, MCP, r#"{"fetch": {"command": "uvx"}}"#);
        apply(&home, &env);
        let mine = json!({"command": "mine"});
        for file in MCP_JSON {
            let mut added = json_at(&home, file);
            added["mcpServers"]["mine"] = mine.clone();
            put(&home, file, &added.to_string());
        }
        let codex = format!(
            "{}\n[mcp_servers.mine]\ncommand = \"mine\"\n",
            text_at(&home, CODEX)
        );
        put(&home, CODEX, &codex);

        std::fs::remove_file(home.join(MCP)).unwrap();
        apply(&home, &env);
        for file in MCP_JSON {
            assert_eq!(
                json_at(&home, file),
                json!({"mcpServers": {"mine": mine}}),
                "{file}"
            );
        }
        assert_eq!(
            text_at(&home, CODEX),
            "\n[mcp_servers.mine]\ncommand = \"mine\"\n"
        );
        clean(&home);
    }

    #[test]
    fn servers_added_by_hand_survive_each_start() {
        let (home, env) = sandbox("survive");
        put(&home, MCP, SERVERS);
        apply(&home, &env);
        let local = json!({"command": "local"});
        for file in MCP_JSON {
            let mut added = json_at(&home, file);
            added["mcpServers"]["local"] = local.clone();
            put(&home, file, &added.to_string());
        }
        let codex = format!(
            "{}\n[mcp_servers.local]\ncommand = \"local\"\n",
            text_at(&home, CODEX)
        );
        put(&home, CODEX, &codex);

        let fetch = json!({"command": "uvx", "args": ["mcp-server-fetch", "--ignore-robots-txt"]});
        put(&home, MCP, &json!({ "fetch": fetch }).to_string());
        for _ in 0..2 {
            assert_eq!(
                apply(&home, &env),
                ["Applied 1 MCP server to Claude Code, Codex, Pi and oh-my-pi"]
            );
            for file in MCP_JSON {
                assert_eq!(
                    json_at(&home, file),
                    json!({"mcpServers": {"fetch": fetch, "local": local}}),
                    "{file}"
                );
            }
            assert_eq!(
                text_at(&home, CODEX),
                "[mcp_servers.fetch]\ncommand = \"uvx\"\n\
                 args = [\"mcp-server-fetch\", \"--ignore-robots-txt\"]\n\n\
                 [mcp_servers.local]\ncommand = \"local\"\n"
            );
        }
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
