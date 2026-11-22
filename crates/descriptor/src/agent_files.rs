//! The files of a plugin's agents: what a plugin's directory and its
//! package hold next to the program.
//!
//! ```text
//! agents/<agent>/soul.md                     the agent's system prompt
//! agents/<agent>/skills/<skill>/SKILL.md …   its own skills (text files; sub-directories allowed)
//! agents/<agent>/mcp.json                    its extra MCP servers
//! skills/<skill>/SKILL.md …                  shared skills, which an agent lists by name (`skills`)
//! ```
//!
//! [`read_dir`] reads them from a directory (a plugin's source tree or an unpacked package); [`layout`] checks
//! them against the descriptor and says what each agent's runs get. `celobus plugin check` runs both, the
//! platform runs [`layout`] on the files of the archive at publish and both on the unpacked package when a run
//! starts. Problems are [`Error`]s like the descriptor's: an `E_*` code and the path of the file.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::{Descriptor, Error};

/// The directory of the agents' folders, in a plugin's directory and in its package.
pub const AGENTS_DIR: &str = "agents";
/// The directory of the shared skills.
pub const SKILLS_DIR: &str = "skills";
/// An agent's system prompt.
pub const SOUL: &str = "soul.md";
/// An agent's extra MCP servers.
pub const MCP: &str = "mcp.json";
/// The file every skill has.
pub const SKILL_FILE: &str = "SKILL.md";

/// The largest file of an agent.
pub const MAX_FILE: usize = 256 * 1024;
/// The most an agent's files weigh together: its folder and the shared skills it lists. Also the most one
/// shared skill weighs.
pub const MAX_AGENT_BYTES: usize = 2 * 1024 * 1024;
/// The most skills an agent has, its own and the shared ones.
pub const MAX_SKILLS: usize = 32;
/// The most extra MCP servers an agent has.
pub const MAX_MCP_SERVERS: usize = 16;
/// The most files `agents/` and `skills/` hold together.
pub const MAX_FILES: usize = 1024;
/// The MCP server every run has: the workspace's own. No agent may name a server so.
pub const RESERVED_SERVER: &str = "celobus";
/// How deep a skill's directories go.
const MAX_DEPTH: usize = 8;

/// One file under `agents/` or `skills/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// Relative to the plugin's directory (the package's root), with `/`: `agents/assistant/soul.md`.
    pub path: String,
    pub content: Vec<u8>,
}

/// One file of a skill.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct SkillFile {
    /// Relative to the skill's directory, with `/`: `SKILL.md`, `references/money.md`.
    pub path: String,
    pub content: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub files: Vec<SkillFile>,
}

/// An extra MCP server of an agent, as `mcp.json` declares it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum McpServer {
    /// Streamable HTTP.
    Http { url: String },
    /// A process the engine starts and talks to over stdio.
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
}

/// What the runs of one agent get from its plugin's package.
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentFiles {
    /// `soul.md`; empty without one.
    pub system_prompt: String,
    /// The agent's own skills by name, then the shared ones in the order it lists them.
    pub skills: Vec<Skill>,
    /// `mcpServers` of `mcp.json`.
    pub mcp_servers: BTreeMap<String, McpServer>,
}

/// The agents' files of a plugin, checked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// By agent name: every agent the descriptor declares.
    pub agents: BTreeMap<String, AgentFiles>,
    /// Every problem; empty when the files are valid. Whatever a problem is about is left out of `agents`.
    pub errors: Vec<Error>,
}

/// A skill's name: lowercase letters, digits and single `-` between them, at most 64 characters
/// (`price-answers`; the Agent Skills rule).
pub fn skill_name_ok(s: &str) -> bool {
    dashed(s, 64)
}

/// An MCP server's name: like a skill's, at most 32 characters, and not the platform's.
pub fn server_name_ok(s: &str) -> bool {
    dashed(s, 32) && s != RESERVED_SERVER
}

fn dashed(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

fn kib(bytes: usize) -> usize {
    bytes.div_ceil(1024)
}

/// Reads `agents/` and `skills/` of `root`: every regular file with its content, sorted by path. Entries whose
/// name starts with a dot are skipped (`.DS_Store`); a link, anything that is neither a file nor a directory and
/// a file over [`MAX_FILE`] are problems and are not read.
pub fn read_dir(root: &Path) -> (Vec<File>, Vec<Error>) {
    fn go(dir: &Path, rel: &str, depth: usize, files: &mut Vec<File>, errors: &mut Vec<Error>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            errors.push(Error::new(
                "E_AGENT_FILE",
                rel,
                "the directory cannot be read",
            ));
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if files.len() > MAX_FILES {
                return;
            }
            let Some(name) = e.file_name().to_str().map(str::to_string) else {
                errors.push(
                    Error::new("E_AGENT_FILE", rel, "a file's name here is not UTF-8 text")
                        .help("rename it"),
                );
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let path = format!("{rel}/{name}");
            let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
                errors.push(Error::new("E_AGENT_FILE", path, "the file cannot be read"));
                continue;
            };
            if meta.file_type().is_symlink() {
                errors.push(
                    Error::new(
                        "E_AGENT_FILE_LINK",
                        path,
                        "a link; an agent's files are files",
                    )
                    .help("replace the link by the file it points to: a package holds no links"),
                );
            } else if meta.is_dir() {
                if depth >= MAX_DEPTH + 3 {
                    errors.push(Error::new(
                        "E_AGENT_FILE",
                        path,
                        format!("more than {MAX_DEPTH} directories deep"),
                    ));
                } else {
                    go(&e.path(), &path, depth + 1, files, errors);
                }
            } else if !meta.is_file() {
                errors.push(Error::new(
                    "E_AGENT_FILE_LINK",
                    path,
                    "neither a file nor a directory",
                ));
            } else if meta.len() > MAX_FILE as u64 {
                errors.push(too_large(&path, meta.len() as usize));
            } else {
                match std::fs::read(e.path()) {
                    Ok(content) => files.push(File { path, content }),
                    Err(_) => {
                        errors.push(Error::new("E_AGENT_FILE", path, "the file cannot be read"))
                    }
                }
            }
        }
    }
    let (mut files, mut errors) = (vec![], vec![]);
    for top in [AGENTS_DIR, SKILLS_DIR] {
        match std::fs::symlink_metadata(root.join(top)) {
            Err(_) => {}
            Ok(m) if m.is_dir() => go(&root.join(top), top, 1, &mut files, &mut errors),
            Ok(_) => errors.push(
                Error::new(
                    "E_AGENT_FILE_LINK",
                    top,
                    format!("{top} is not a directory"),
                )
                .help(format!(
                    "{top}/ of a plugin holds its agents' files; name this one otherwise"
                )),
            ),
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    (files, errors)
}

/// `E_AGENT_FILE_SIZE` for a file of `bytes`.
pub fn too_large(path: &str, bytes: usize) -> Error {
    Error::new(
        "E_AGENT_FILE_SIZE",
        path,
        format!(
            "{} KiB; a file of an agent is at most {} KiB",
            kib(bytes),
            MAX_FILE / 1024
        ),
    )
    .help("skills are instructions and small references; keep data and binaries in the program")
}

/// `E_AGENT_FILES_MANY`: more files than [`MAX_FILES`].
pub fn too_many() -> Error {
    Error::new(
        "E_AGENT_FILES_MANY",
        "",
        format!("agents/ and skills/ hold more than {MAX_FILES} files"),
    )
}

#[derive(Default)]
struct SkillDir<'a> {
    /// (path inside the skill, the file)
    files: Vec<(String, &'a File)>,
}

impl SkillDir<'_> {
    fn bytes(&self) -> usize {
        self.files.iter().map(|(_, f)| f.content.len()).sum()
    }
}

#[derive(Default)]
struct AgentDir<'a> {
    soul: Option<&'a File>,
    mcp: Option<&'a File>,
    skills: BTreeMap<String, SkillDir<'a>>,
}

/// Checks the files against the descriptor and says what each declared agent gets.
pub fn layout(d: &Descriptor, files: &[File]) -> Layout {
    let mut errors: Vec<Error> = vec![];
    if files.len() > MAX_FILES {
        errors.push(too_many());
    }
    let declared: BTreeSet<&str> = d.agents.iter().map(|a| a.name.as_str()).collect();
    let mut agents: BTreeMap<&str, AgentDir> = BTreeMap::new();
    let mut shared: BTreeMap<String, SkillDir> = BTreeMap::new();
    let mut unknown_folders: BTreeSet<&str> = BTreeSet::new();
    let mut bad_skills: BTreeSet<String> = BTreeSet::new();
    let misplaced = |f: &File| {
        Error::new(
            "E_AGENT_FILE_UNKNOWN",
            &f.path,
            "not a file of an agent",
        )
        .help(
            "an agent has agents/<agent>/soul.md, agents/<agent>/mcp.json and agents/<agent>/skills/<skill>/…; \
             shared skills are skills/<skill>/…",
        )
    };
    for f in files {
        let parts: Vec<&str> = f.path.split('/').collect();
        if parts
            .iter()
            .any(|p| p.is_empty() || matches!(*p, "." | "..") || p.contains('\\'))
        {
            errors.push(misplaced(f));
            continue;
        }
        // (the agent, the skill, the path inside the skill)
        let skill: (Option<&str>, &str, String) = match parts.as_slice() {
            [AGENTS_DIR, agent, rest @ ..] if !rest.is_empty() => {
                let agent = *agent;
                if !declared.contains(agent) {
                    if unknown_folders.insert(agent) {
                        errors.push(
                            Error::new(
                                "E_AGENT_FOLDER_UNKNOWN",
                                format!("{AGENTS_DIR}/{agent}"),
                                format!("the plugin declares no agent {agent:?}"),
                            )
                            .help(
                                "a folder of agents/ is named like an agent of the descriptor (snake_case); declare \
                                 the agent or remove the folder",
                            ),
                        );
                    }
                    continue;
                }
                let dir = agents.entry(agent).or_default();
                match rest {
                    [SOUL] => {
                        dir.soul = checked(f, &mut errors).map(|_| f);
                        continue;
                    }
                    [MCP] => {
                        dir.mcp = checked(f, &mut errors).map(|_| f);
                        continue;
                    }
                    [SKILLS_DIR, skill, inside @ ..] if !inside.is_empty() => {
                        (Some(agent), *skill, inside.join("/"))
                    }
                    _ => {
                        errors.push(misplaced(f));
                        continue;
                    }
                }
            }
            [SKILLS_DIR, skill, inside @ ..] if !inside.is_empty() => {
                (None, *skill, inside.join("/"))
            }
            _ => {
                errors.push(misplaced(f));
                continue;
            }
        };
        let (agent, name, inside) = skill;
        let dir_path = match agent {
            Some(a) => format!("{AGENTS_DIR}/{a}/{SKILLS_DIR}/{name}"),
            None => format!("{SKILLS_DIR}/{name}"),
        };
        if !skill_name_ok(name) {
            if bad_skills.insert(dir_path.clone()) {
                errors.push(
                    Error::new(
                        "E_SKILL_NAME",
                        dir_path,
                        format!("{name:?} is not a skill name"),
                    )
                    .help("lowercase letters, digits and -, at most 64 characters: price-answers"),
                );
            }
            continue;
        }
        if inside.split('/').count() > MAX_DEPTH {
            errors.push(Error::new(
                "E_AGENT_FILE",
                &f.path,
                format!("more than {MAX_DEPTH} directories deep"),
            ));
            continue;
        }
        if checked(f, &mut errors).is_none() {
            // a skill with a file that cannot be given is not given
            bad_skills.insert(dir_path);
        }
        let dir = match agent {
            Some(a) => agents
                .entry(a)
                .or_default()
                .skills
                .entry(name.to_string())
                .or_default(),
            None => shared.entry(name.to_string()).or_default(),
        };
        dir.files.push((inside, f));
    }

    // every skill has its SKILL.md with a name and a description
    let usable = |dir_path: String, name: &str, s: &SkillDir, errors: &mut Vec<Error>| -> bool {
        let Some((_, file)) = s.files.iter().find(|(p, _)| p == SKILL_FILE) else {
            errors.push(
                Error::new(
                    "E_SKILL_MISSING",
                    format!("{dir_path}/{SKILL_FILE}"),
                    format!("the skill {name:?} has no {SKILL_FILE}"),
                )
                .help("a skill is a folder with SKILL.md: front matter (name, description), then the instructions"),
            );
            return false;
        };
        let front = std::str::from_utf8(&file.content)
            .ok()
            .and_then(front_matter)
            .unwrap_or_default();
        let get = |key: &str| {
            front
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
                .unwrap_or_default()
        };
        let description = get("description");
        if get("name") != name || description.is_empty() || description.chars().count() > 1024 {
            errors.push(
                Error::new(
                    "E_SKILL_FRONTMATTER",
                    format!("{dir_path}/{SKILL_FILE}"),
                    "SKILL.md starts with front matter that names the skill like its folder and describes it",
                )
                .help(format!(
                    "start the file with:\n---\nname: {name}\ndescription: <what the skill does and when to use it, \
                     at most 1024 characters>\n---"
                )),
            );
            return false;
        }
        !bad_skills.contains(&dir_path)
    };
    let mut shared_ok: BTreeSet<&str> = BTreeSet::new();
    for (name, s) in &shared {
        let dir_path = format!("{SKILLS_DIR}/{name}");
        if s.bytes() > MAX_AGENT_BYTES {
            errors.push(Error::new(
                "E_AGENT_FILES_SIZE",
                dir_path.clone(),
                format!(
                    "{} KiB; a skill is at most {} KiB",
                    kib(s.bytes()),
                    MAX_AGENT_BYTES / 1024
                ),
            ));
        } else if usable(dir_path, name, s, &mut errors) {
            shared_ok.insert(name);
        }
    }
    let skill_of = |name: &str, s: &SkillDir| {
        let mut files: Vec<SkillFile> = s
            .files
            .iter()
            .filter_map(|(path, f)| {
                Some(SkillFile {
                    path: path.clone(),
                    content: String::from_utf8(f.content.clone()).ok()?,
                })
            })
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Skill {
            name: name.to_string(),
            files,
        }
    };

    let mut out: BTreeMap<String, AgentFiles> = BTreeMap::new();
    let empty = AgentDir::default();
    for a in &d.agents {
        let dir = agents.get(a.name.as_str()).unwrap_or(&empty);
        let folder = format!("{AGENTS_DIR}/{}", a.name);
        let mut got = AgentFiles::default();
        let mut bytes = 0;
        if let Some(soul) = dir.soul {
            bytes += soul.content.len();
            got.system_prompt = String::from_utf8_lossy(&soul.content).into_owned();
        }
        if let Some(mcp) = dir.mcp {
            bytes += mcp.content.len();
            got.mcp_servers = mcp_servers(&mcp.path, &mcp.content, &mut errors);
        }
        for (name, s) in &dir.skills {
            bytes += s.bytes();
            if usable(
                format!("{folder}/{SKILLS_DIR}/{name}"),
                name,
                s,
                &mut errors,
            ) {
                got.skills.push(skill_of(name, s));
            }
        }
        for (j, name) in a.skills.iter().enumerate() {
            if dir.skills.contains_key(name) {
                errors.push(
                    Error::new(
                        "E_AGENT_SKILL_DUP",
                        format!("{folder}/{SKILLS_DIR}/{name}"),
                        format!("the agent also lists the shared skill {name:?}"),
                    )
                    .help("an agent has one skill of a name: rename one, or drop the shared one from `skills`"),
                );
                continue;
            }
            let Some(s) = shared.get(name) else {
                errors.push(
                    Error::new(
                        "E_AGENT_SKILL_UNKNOWN",
                        format!("agents[{}].skills[{j}]", a.name),
                        format!("there is no shared skill {name:?}"),
                    )
                    .help(format!(
                        "a shared skill is the folder {SKILLS_DIR}/{name}/ of the plugin, with {SKILL_FILE}"
                    )),
                );
                continue;
            };
            bytes += s.bytes();
            if shared_ok.contains(name.as_str()) && !a.skills[..j].contains(name) {
                got.skills.push(skill_of(name, s));
            }
        }
        if dir.skills.len() + a.skills.len() > MAX_SKILLS {
            errors.push(Error::new(
                "E_AGENT_SKILLS_MANY",
                folder.clone(),
                format!(
                    "{} skills; an agent has at most {MAX_SKILLS}, its own and the shared ones",
                    dir.skills.len() + a.skills.len()
                ),
            ));
            got.skills.truncate(MAX_SKILLS);
        }
        if bytes > MAX_AGENT_BYTES {
            errors.push(
                Error::new(
                    "E_AGENT_FILES_SIZE",
                    folder,
                    format!(
                        "{} KiB; an agent's files are at most {} KiB together, the shared skills it lists included",
                        kib(bytes),
                        MAX_AGENT_BYTES / 1024
                    ),
                )
                .help("every run of the agent is given all of them"),
            );
            got.skills.clear();
        }
        out.insert(a.name.clone(), got);
    }
    Layout {
        agents: out,
        errors,
    }
}

/// The file's text when it is one of an agent's: within [`MAX_FILE`] and UTF-8.
fn checked<'a>(f: &'a File, errors: &mut Vec<Error>) -> Option<&'a str> {
    if f.content.len() > MAX_FILE {
        errors.push(too_large(&f.path, f.content.len()));
        return None;
    }
    match std::str::from_utf8(&f.content) {
        Ok(text) if !text.contains('\0') => Some(text),
        _ => {
            errors.push(
                Error::new("E_AGENT_FILE_TEXT", &f.path, "not UTF-8 text")
                    .help("an agent's files are text: instructions, references, scripts"),
            );
            None
        }
    }
}

impl Skill {
    /// What the skill does and when to use it: `description` of the front matter of its `SKILL.md`. Empty when
    /// it has none. It is what an agent's card says of the skill; the rest of the skill is the agent's own.
    pub fn description(&self) -> String {
        self.files
            .iter()
            .find(|f| f.path == SKILL_FILE)
            .and_then(|f| front_matter(&f.content))
            .and_then(|fm| fm.into_iter().find(|(k, _)| k == "description"))
            .map(|(_, v)| v)
            .unwrap_or_default()
    }
}

/// The top-level keys of the YAML front matter a `SKILL.md` starts with (`---` … `---`), each with its value on
/// one line (a block or wrapped scalar joined by spaces). `None` without front matter.
fn front_matter(text: &str) -> Option<Vec<(String, String)>> {
    let mut lines = text.strip_prefix('\u{feff}').unwrap_or(text).lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let mut out: Vec<(String, String)> = vec![];
    for line in lines {
        let line = line.trim_end();
        if line == "---" {
            return Some(out);
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            // the value above goes on (a block scalar, a wrapped one); a nested mapping is nobody's value here
            if let Some((_, value)) = out.last_mut() {
                if !value.is_empty() {
                    value.push(' ');
                }
                value.push_str(line.trim());
            }
            continue;
        }
        let (key, value) = line.split_once(':')?;
        let value = value.trim();
        let value = match value {
            ">" | "|" | ">-" | "|-" | ">+" | "|+" => "",
            v => v
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(v),
        };
        out.push((key.trim().to_string(), value.to_string()));
    }
    None
}

/// `mcpServers` of an agent's `mcp.json`; a server with a problem is left out.
fn mcp_servers(path: &str, content: &[u8], errors: &mut Vec<Error>) -> BTreeMap<String, McpServer> {
    let mut out = BTreeMap::new();
    let shape = "{\"mcpServers\": {\"<name>\": {\"url\": \"https://…\"} | {\"command\": \"…\", \"args\": […], \"env\": {…}}}}";
    let mut bad = |at: String, message: String, help: &str| {
        let at = if at.is_empty() {
            path.to_string()
        } else {
            format!("{path}:{at}")
        };
        errors.push(Error::new("E_AGENT_MCP", at, message).help(help));
    };
    let root: Value = match serde_json::from_slice(content) {
        Ok(v) => v,
        Err(e) => {
            bad(String::new(), format!("not JSON: {e}"), shape);
            return out;
        }
    };
    let servers = match root.as_object() {
        Some(o) if o.len() == 1 => o.get("mcpServers").and_then(Value::as_object),
        _ => None,
    };
    let Some(servers) = servers else {
        bad(
            String::new(),
            "mcp.json is one object with the member mcpServers".into(),
            shape,
        );
        return out;
    };
    if servers.len() > MAX_MCP_SERVERS {
        bad(
            "mcpServers".into(),
            format!(
                "{} servers; an agent has at most {MAX_MCP_SERVERS}",
                servers.len()
            ),
            "the workspace's own server, celobus, is always there and is not counted",
        );
        return out;
    }
    // text an engine would expand from its own environment, which holds the run's credentials
    let literal = |s: &str| !s.contains("${") && !s.contains('\0');
    let no_secrets = "placeholders are not resolved and no secrets are kept for an MCP server: write the value itself, \
                      and leave a server that needs a credential out";
    for (name, server) in servers {
        let at = format!("mcpServers.{name}");
        if name.eq_ignore_ascii_case(RESERVED_SERVER) {
            bad(
                at,
                format!("{RESERVED_SERVER} is the workspace's own MCP server"),
                "every run has it already; name this server otherwise",
            );
            continue;
        }
        if !server_name_ok(name) {
            bad(
                at,
                format!("{name:?} is not an MCP server name"),
                "lowercase letters, digits and -, at most 32 characters: product-docs",
            );
            continue;
        }
        let Some(o) = server.as_object() else {
            bad(at, "a server is an object".into(), shape);
            continue;
        };
        let before = out.len();
        let mut ok = true;
        for key in o.keys() {
            match key.as_str() {
                "type" | "url" | "command" | "args" | "env" => {}
                "headers" | "oauth" => {
                    ok = false;
                    bad(
                        format!("{at}.{key}"),
                        format!("{key} are not supported"),
                        no_secrets,
                    );
                }
                _ => {
                    ok = false;
                    bad(
                        format!("{at}.{key}"),
                        format!("a server has no member {key:?}"),
                        shape,
                    );
                }
            }
        }
        let text = |key: &str| o.get(key).and_then(Value::as_str);
        let kind = match (o.contains_key("url"), o.contains_key("command")) {
            (true, false) => "http",
            (false, true) => "stdio",
            _ => {
                bad(at, "a server has either a url or a command".into(), shape);
                continue;
            }
        };
        if o.get("type").is_some_and(|t| t.as_str() != Some(kind)) {
            ok = false;
            bad(
                format!("{at}.type"),
                format!("the type of this server is {kind:?}"),
                "leave type out: a url is an HTTP server, a command a stdio one",
            );
        }
        if kind == "http" {
            let url = text("url").unwrap_or_default();
            let rest = url
                .strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"));
            let authority = rest.map(|r| r.split(['/', '?', '#']).next().unwrap_or_default());
            if o.contains_key("args") || o.contains_key("env") {
                ok = false;
                bad(at.clone(), "args and env belong to a command".into(), shape);
            }
            match authority {
                Some(a)
                    if !a.is_empty()
                        && !a.contains('@')
                        && url.len() <= 2048
                        && literal(url)
                        && !url.chars().any(|c| c.is_whitespace() || c.is_control()) =>
                {
                    if ok {
                        out.insert(name.clone(), McpServer::Http { url: url.into() });
                    }
                }
                _ => bad(
                    format!("{at}.url"),
                    "the url is an http(s) address without credentials or placeholders".into(),
                    no_secrets,
                ),
            }
            continue;
        }
        let command = text("command").unwrap_or_default();
        if command.trim().is_empty() || command.len() > 1024 || !literal(command) {
            ok = false;
            bad(
                format!("{at}.command"),
                "the command is one executable, without placeholders".into(),
                "its arguments go into args; it is not a shell line",
            );
        }
        let mut args = vec![];
        match o.get("args") {
            None => {}
            Some(Value::Array(list))
                if list.len() <= 64
                    && list
                        .iter()
                        .all(|a| a.as_str().is_some_and(|a| a.len() <= 4096 && literal(a))) =>
            {
                args = list
                    .iter()
                    .filter_map(|a| a.as_str().map(str::to_string))
                    .collect();
            }
            Some(_) => {
                ok = false;
                bad(
                    format!("{at}.args"),
                    "args are at most 64 strings without placeholders".into(),
                    no_secrets,
                );
            }
        }
        let mut env = BTreeMap::new();
        match o.get("env") {
            None => {}
            Some(Value::Object(vars)) if vars.len() <= 32 => {
                for (var, value) in vars {
                    let named = var.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                        && var.bytes().next().is_some_and(|c| !c.is_ascii_digit());
                    match value.as_str() {
                        // `!…` is a command to Pi
                        Some(v)
                            if named && v.len() <= 4096 && literal(v) && !v.starts_with('!') =>
                        {
                            env.insert(var.clone(), v.to_string());
                        }
                        _ => {
                            ok = false;
                            bad(
                                format!("{at}.env.{var}"),
                                "a variable has a name of letters, digits and _ and a literal text value".into(),
                                no_secrets,
                            );
                        }
                    }
                }
            }
            Some(_) => {
                ok = false;
                bad(
                    format!("{at}.env"),
                    "env is an object of at most 32 variables".into(),
                    shape,
                );
            }
        }
        if ok && out.len() == before {
            out.insert(
                name.clone(),
                McpServer::Stdio {
                    command: command.into(),
                    args,
                    env,
                },
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn descriptor(agents: Value) -> Descriptor {
        serde_json::from_value(
            json!({ "schema": 1, "id": "shop", "version": "1.0.0", "agents": agents }),
        )
        .unwrap()
    }

    fn file(path: &str, content: &str) -> File {
        File {
            path: path.into(),
            content: content.as_bytes().to_vec(),
        }
    }

    fn skill_md(name: &str) -> String {
        format!("---\nname: {name}\ndescription: What {name} is for.\n---\n\n# {name}\n")
    }

    /// (code, path) of every problem, sorted.
    fn codes(l: &Layout) -> Vec<(&'static str, &str)> {
        let mut codes: Vec<_> = l.errors.iter().map(|e| (e.code, e.path.as_str())).collect();
        codes.sort();
        codes
    }

    #[test]
    fn an_agent_gets_its_soul_its_skills_then_the_shared_ones_and_its_servers() {
        let d = descriptor(json!([
            { "name": "assistant", "skills": ["money", "tone"] },
            { "name": "bare" },
        ]));
        let files = [
            file(
                "agents/assistant/soul.md",
                "You are the shop's assistant.\n",
            ),
            file(
                "agents/assistant/mcp.json",
                r#"{"mcpServers": {
                    "docs": {"type": "http", "url": "https://docs.example.com/mcp"},
                    "files": {"command": "npx", "args": ["-y", "server-files@1.2.3", "."], "env": {"LOG": "warn"}},
                    "bare": {"command": "/opt/tools/bare"}
                }}"#,
            ),
            file(
                "agents/assistant/skills/prices/SKILL.md",
                &skill_md("prices"),
            ),
            file(
                "agents/assistant/skills/prices/references/tiers.md",
                "gold\n",
            ),
            file("skills/tone/SKILL.md", &skill_md("tone")),
            file("skills/money/SKILL.md", &skill_md("money")),
            file("skills/money/format.md", "$1,234.50\n"),
            // a shared skill nobody lists stays in the package and reaches no run
            file("skills/unused/SKILL.md", &skill_md("unused")),
        ];
        let l = layout(&d, &files);
        assert_eq!(l.errors, vec![]);
        let a = &l.agents["assistant"];
        assert_eq!(a.system_prompt, "You are the shop's assistant.\n");
        let names: Vec<&str> = a.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["prices", "money", "tone"]);
        let paths: Vec<&str> = a.skills[0].files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["SKILL.md", "references/tiers.md"]);
        assert_eq!(a.skills[1].files[1].content, "$1,234.50\n");
        assert_eq!(
            serde_json::to_value(a).unwrap()["mcp_servers"],
            json!({
                "docs": { "url": "https://docs.example.com/mcp" },
                "files": { "command": "npx", "args": ["-y", "server-files@1.2.3", "."], "env": { "LOG": "warn" } },
                "bare": { "command": "/opt/tools/bare", "args": [], "env": {} },
            })
        );
        // an agent without a folder has nothing, and is there
        assert_eq!(l.agents["bare"], AgentFiles::default());
    }

    #[test]
    fn folders_files_and_skills_that_do_not_belong() {
        let d = descriptor(json!([{ "name": "assistant", "skills": ["money", "gone"] }]));
        let files = [
            file("agents/helper/soul.md", "x"),
            file("agents/helper/skills/a/SKILL.md", "x"),
            file("agents/README.md", "x"),
            file("agents/assistant/notes.txt", "x"),
            file("agents/assistant/skills/README.md", "x"),
            file("skills/README.md", "x"),
            file(
                "agents/assistant/skills/Prices/SKILL.md",
                &skill_md("Prices"),
            ),
            file("agents/assistant/skills/no-file/notes.md", "x"),
            file(
                "agents/assistant/skills/no-front/SKILL.md",
                "# No front matter\n",
            ),
            file(
                "agents/assistant/skills/other-name/SKILL.md",
                &skill_md("another"),
            ),
            file(
                "agents/assistant/skills/no-description/SKILL.md",
                "---\nname: no-description\n---\n",
            ),
            file("agents/assistant/skills/money/SKILL.md", &skill_md("money")),
            file("skills/money/SKILL.md", &skill_md("money")),
        ];
        let l = layout(&d, &files);
        assert_eq!(
            codes(&l),
            [
                ("E_AGENT_FILE_UNKNOWN", "agents/README.md"),
                ("E_AGENT_FILE_UNKNOWN", "agents/assistant/notes.txt"),
                ("E_AGENT_FILE_UNKNOWN", "agents/assistant/skills/README.md"),
                ("E_AGENT_FILE_UNKNOWN", "skills/README.md"),
                ("E_AGENT_FOLDER_UNKNOWN", "agents/helper"),
                ("E_AGENT_SKILL_DUP", "agents/assistant/skills/money"),
                ("E_AGENT_SKILL_UNKNOWN", "agents[assistant].skills[1]"),
                (
                    "E_SKILL_FRONTMATTER",
                    "agents/assistant/skills/no-description/SKILL.md"
                ),
                (
                    "E_SKILL_FRONTMATTER",
                    "agents/assistant/skills/no-front/SKILL.md"
                ),
                (
                    "E_SKILL_FRONTMATTER",
                    "agents/assistant/skills/other-name/SKILL.md"
                ),
                (
                    "E_SKILL_MISSING",
                    "agents/assistant/skills/no-file/SKILL.md"
                ),
                ("E_SKILL_NAME", "agents/assistant/skills/Prices"),
            ]
        );
        // what is valid is still given
        let names: Vec<&str> = l.agents["assistant"]
            .skills
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["money"]);
    }

    #[test]
    fn sizes_text_and_counts() {
        let d = descriptor(json!([{ "name": "assistant", "skills": ["big"] }]));
        let mut files = vec![
            File {
                path: "agents/assistant/soul.md".into(),
                content: vec![b'a'; MAX_FILE + 1],
            },
            File {
                path: "agents/assistant/skills/prices/logo.png".into(),
                content: vec![0x89, b'P', b'N', b'G', 0xff],
            },
            file(
                "agents/assistant/skills/prices/SKILL.md",
                &skill_md("prices"),
            ),
            file("skills/big/SKILL.md", &skill_md("big")),
        ];
        // files within the limit that are too much together
        for i in 0..9 {
            files.push(File {
                path: format!("skills/big/part-{i}.md"),
                content: vec![b'a'; MAX_FILE],
            });
        }
        let l = layout(&d, &files);
        assert_eq!(
            codes(&l),
            [
                ("E_AGENT_FILES_SIZE", "agents/assistant"),
                ("E_AGENT_FILES_SIZE", "skills/big"),
                ("E_AGENT_FILE_SIZE", "agents/assistant/soul.md"),
                (
                    "E_AGENT_FILE_TEXT",
                    "agents/assistant/skills/prices/logo.png"
                ),
            ]
        );
        assert_eq!(l.agents["assistant"].system_prompt, "");

        let many: Vec<Value> = (0..MAX_SKILLS + 1)
            .map(|i| json!(format!("s{i}")))
            .collect();
        let d = descriptor(json!([{ "name": "assistant", "skills": many }]));
        let files: Vec<File> = (0..MAX_SKILLS + 1)
            .map(|i| {
                file(
                    &format!("skills/s{i}/SKILL.md"),
                    &skill_md(&format!("s{i}")),
                )
            })
            .collect();
        let l = layout(&d, &files);
        assert_eq!(codes(&l), [("E_AGENT_SKILLS_MANY", "agents/assistant")]);
        assert_eq!(l.agents["assistant"].skills.len(), MAX_SKILLS);

        let files: Vec<File> = (0..MAX_FILES + 1)
            .map(|i| file(&format!("skills/s0/f{i}.md"), ""))
            .collect();
        assert!(codes(&layout(&d, &files)).contains(&("E_AGENT_FILES_MANY", "")));
    }

    #[test]
    fn mcp_servers_are_named_literal_and_never_the_platforms() {
        let d = descriptor(json!([{ "name": "assistant" }]));
        let errors = |mcp: Value| -> Vec<String> {
            let l = layout(&d, &[file("agents/assistant/mcp.json", &mcp.to_string())]);
            assert!(
                l.errors.iter().all(|e| e.code == "E_AGENT_MCP"),
                "{:?}",
                l.errors
            );
            assert!(l.agents["assistant"].mcp_servers.is_empty(), "{mcp}");
            l.errors.into_iter().map(|e| e.path).collect()
        };
        let at = |rest: &str| vec![format!("agents/assistant/mcp.json:{rest}")];
        assert_eq!(errors(json!([])), ["agents/assistant/mcp.json"]);
        assert_eq!(
            errors(json!({ "mcpServers": {}, "inputs": [] })),
            ["agents/assistant/mcp.json"]
        );
        for name in ["celobus", "Celobus"] {
            assert_eq!(
                errors(json!({ "mcpServers": { name: { "url": "https://x.example/mcp" } } })),
                at(&format!("mcpServers.{name}"))
            );
        }
        assert_eq!(
            errors(json!({ "mcpServers": { "my_docs": { "url": "https://x.example/mcp" } } })),
            at("mcpServers.my_docs")
        );
        assert_eq!(
            errors(
                json!({ "mcpServers": { "docs": { "url": "https://x.example/mcp", "command": "x" } } })
            ),
            at("mcpServers.docs")
        );
        assert_eq!(
            errors(
                json!({ "mcpServers": { "docs": { "url": "https://x.example/mcp", "type": "stdio" } } })
            ),
            at("mcpServers.docs.type")
        );
        for url in [
            "ftp://x.example/mcp",
            "https://user:pass@x.example/mcp",
            "https://x.example/mcp?key=${DOCS_KEY}",
            "https://",
        ] {
            assert_eq!(
                errors(json!({ "mcpServers": { "docs": { "url": url } } })),
                at("mcpServers.docs.url"),
                "{url}"
            );
        }
        assert_eq!(
            errors(json!({ "mcpServers": { "docs": {
                "url": "https://x.example/mcp", "headers": { "Authorization": "Bearer x" } } } })),
            at("mcpServers.docs.headers")
        );
        assert_eq!(
            errors(
                json!({ "mcpServers": { "git": { "command": "git-mcp", "args": ["--token", "${GH_TOKEN}"] } } })
            ),
            at("mcpServers.git.args")
        );
        for (var, value) in [
            ("TOKEN", "${CELOBUS_CLIENT_SECRET}"),
            ("TOKEN", "!cat key"),
            ("1X", "v"),
        ] {
            assert_eq!(
                errors(
                    json!({ "mcpServers": { "git": { "command": "git-mcp", "env": { var: value } } } })
                ),
                at(&format!("mcpServers.git.env.{var}"))
            );
        }
        let many: serde_json::Map<String, Value> = (0..MAX_MCP_SERVERS + 1)
            .map(|i| (format!("s{i}"), json!({ "command": "x" })))
            .collect();
        assert_eq!(errors(json!({ "mcpServers": many })), at("mcpServers"));
        // one bad server leaves the others
        let l = layout(
            &d,
            &[file(
                "agents/assistant/mcp.json",
                r#"{"mcpServers": {"celobus": {"url": "https://x.example"}, "docs": {"url": "http://docs.local/mcp"}}}"#,
            )],
        );
        assert_eq!(l.errors.len(), 1);
        assert_eq!(
            l.agents["assistant"].mcp_servers["docs"],
            McpServer::Http {
                url: "http://docs.local/mcp".into()
            }
        );
    }

    #[test]
    fn front_matter_values() {
        let fm = front_matter(
            "---\nname: \"prices\"\n# a comment\ndescription: >\n  How to answer\n  a price question.\nmetadata:\n  author: x\n---\nbody: not a key\n",
        )
        .unwrap();
        assert_eq!(
            fm,
            [
                ("name".to_string(), "prices".to_string()),
                (
                    "description".to_string(),
                    "How to answer a price question.".to_string()
                ),
                ("metadata".to_string(), "author: x".to_string()),
            ]
        );
        assert_eq!(front_matter("# Prices\n"), None);
        assert_eq!(front_matter("---\nname: prices\n"), None);
        assert!(front_matter("---\r\nname: prices\r\n---\r\n").is_some());
    }

    #[test]
    fn names() {
        for ok in ["prices", "price-answers", "a", "s3-sync", "2fa"] {
            assert!(skill_name_ok(ok), "{ok}");
        }
        for bad in [
            "",
            "Prices",
            "price_answers",
            "-prices",
            "prices-",
            "a--b",
            "a b",
            "prix-é",
        ] {
            assert!(!skill_name_ok(bad), "{bad}");
        }
        assert!(!skill_name_ok(&"a".repeat(65)));
        assert!(server_name_ok("product-docs"));
        assert!(!server_name_ok("celobus"));
        assert!(!server_name_ok(&"a".repeat(33)));
    }

    #[test]
    fn a_directory_is_read_without_links_dot_files_and_large_files() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("agents/assistant/skills/prices/references")).unwrap();
        std::fs::create_dir_all(p.join("skills/money")).unwrap();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(p.join("agents/assistant/soul.md"), "soul").unwrap();
        std::fs::write(p.join("agents/assistant/.DS_Store"), [0u8, 159]).unwrap();
        std::fs::write(p.join("agents/assistant/skills/prices/SKILL.md"), "s").unwrap();
        std::fs::write(
            p.join("agents/assistant/skills/prices/references/tiers.md"),
            "t",
        )
        .unwrap();
        std::fs::write(p.join("skills/money/SKILL.md"), "m").unwrap();
        let (files, errors) = read_dir(p);
        assert_eq!(errors, vec![]);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "agents/assistant/skills/prices/SKILL.md",
                "agents/assistant/skills/prices/references/tiers.md",
                "agents/assistant/soul.md",
                "skills/money/SKILL.md",
            ]
        );
        assert_eq!(files[2].content, b"soul");

        std::fs::write(p.join("skills/money/big.md"), vec![b'a'; MAX_FILE + 1]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", p.join("skills/money/passwd")).unwrap();
        let (files, errors) = read_dir(p);
        assert_eq!(files.len(), 4);
        let mut got: Vec<(&str, &str)> = errors.iter().map(|e| (e.code, e.path.as_str())).collect();
        got.sort();
        let mut want = vec![("E_AGENT_FILE_SIZE", "skills/money/big.md")];
        if cfg!(unix) {
            want.insert(0, ("E_AGENT_FILE_LINK", "skills/money/passwd"));
        }
        assert_eq!(got, want);

        // a plugin without the two directories has no agent files
        let bare = tempfile::tempdir().unwrap();
        assert_eq!(read_dir(bare.path()), (vec![], vec![]));
    }
}
