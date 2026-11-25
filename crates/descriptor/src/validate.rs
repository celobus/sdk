//! The definition-time rules of a descriptor. Every problem is collected; none stops the others.

use std::collections::HashSet;

use crate::{
    AGENT_ENGINES, CAPABILITIES, Descriptor, Entity, Error, Field, FieldType, IMPLICIT_SCOPES,
    MAX_NOTIFICATION_KINDS, MAX_SEARCHABLE, RESERVED_NOTIFICATION_KINDS,
    RESERVED_NOTIFICATION_PREFIX, SEARCH_LANGUAGES, SYSTEM_COLUMNS, key_field_type,
};

/// The longest endpoint timeout (600 s).
pub const MAX_TIMEOUT_MS: u64 = 600_000;

/// The shortest and the longest time an agent's declaration may give its runs (one minute, six hours).
pub const AGENT_TIMEOUT_S: std::ops::RangeInclusive<u32> = 60..=6 * 3600;

/// Every problem of the descriptor (empty when it is valid).
pub fn validate(d: &Descriptor) -> Vec<Error> {
    let mut v = V { errors: vec![] };
    v.errors.extend(plugin_id(&d.id, "id"));
    if semver::Version::parse(&d.version).is_err() {
        v.push(
            Error::new(
                "E_PLUGIN_VERSION",
                "version",
                format!("{:?} is not a semantic version", d.version),
            )
            .help("use MAJOR.MINOR.PATCH, for example 0.1.0"),
        );
    }
    v.identity(d);
    let types = v.types(d);
    v.storage(d);
    v.encrypted(d);
    v.navigations(d);
    v.on_delete(d);
    v.endpoints(d, &types);
    v.errors.extend(crate::access::validate(d));
    v.errors.extend(crate::ui_check::validate_ui(d));
    v.notifications(d);
    v.jobs(d, &types);
    v.errors.extend(crate::view::validate(d));
    v.errors.extend(crate::lake::validate(d));
    v.errors.extend(crate::lake_model::validate(d));
    v.errors.extend(crate::load::validate(d));
    v.errors.extend(crate::history::validate(d));
    v.errors
}

/// `E_PLUGIN_ID`, `E_PLUGIN_ID_RESERVED` for one id.
pub(crate) fn plugin_id(id: &str, path: &str) -> Vec<Error> {
    if !celobus_manifest::plugin_name_shape_ok(id) {
        return vec![
            Error::new("E_PLUGIN_ID", path, format!("{id:?} is not a plugin id")).help(
                "2 to 16 lowercase letters and digits, starting with a letter (orders, crm2)",
            ),
        ];
    }
    if celobus_manifest::plugin_name_reserved(id) {
        return vec![
            Error::new(
                "E_PLUGIN_ID_RESERVED",
                path,
                format!("{id:?} is reserved by the platform"),
            )
            .help("pick another id; platform words and ids starting with pg are reserved"),
        ];
    }
    vec![]
}

/// A path segment for a named item: its name, or its index when the name cannot stand in a path.
fn at(name: &str, i: usize) -> String {
    if !name.is_empty()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':'))
    {
        name.to_string()
    } else {
        i.to_string()
    }
}

/// A role or scope key: `^[a-z][a-z0-9_]{1,31}$`.
fn key_ok(s: &str) -> bool {
    let b = s.as_bytes();
    (2..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// A job's key: `^[a-z][a-z0-9_]{0,31}$`.
fn job_name_ok(s: &str) -> bool {
    s.len() <= 32 && snake_ok(s)
}

/// The first field of message `name` that a payload read from `{}` cannot fill (`RemindUnpaid.since`): a single
/// `timestamp` or `date` that is not optional, directly or in a message field that is not optional. Every other
/// absent field reads as its default.
fn required_without_default(d: &Descriptor, name: &str) -> Option<String> {
    fn walk<'a>(d: &'a Descriptor, name: &'a str, seen: &mut Vec<&'a str>) -> Option<String> {
        if seen.contains(&name) {
            return None;
        }
        seen.push(name);
        let fields = d
            .messages
            .iter()
            .find(|m| m.name == name)
            .map(|m| &m.fields)
            .or_else(|| {
                d.entities
                    .iter()
                    .find(|e| e.message == name)
                    .map(|e| &e.fields)
            })?;
        for f in fields.iter().filter(|f| !f.optional && !f.repeated) {
            match FieldType::parse(&f.ty) {
                Some(FieldType::Scalar("timestamp" | "date")) => {
                    return Some(format!("{name}.{}", f.name));
                }
                Some(FieldType::Message(m)) => {
                    let Some(m) = d.messages.iter().find(|x| x.name == m) else {
                        continue;
                    };
                    if let Some(found) = walk(d, &m.name, seen) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }
    walk(d, name, &mut vec![])
}

/// snake_case (for the other modules' names).
pub(crate) fn snake(s: &str) -> bool {
    snake_ok(s)
}

/// snake_case: `^[a-z][a-z0-9_]{0,62}$`.
fn snake_ok(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=63).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// PascalCase: `^[A-Z][A-Za-z0-9]{0,63}$`.
fn pascal_ok(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=64).contains(&b.len())
        && b[0].is_ascii_uppercase()
        && b.iter().all(|c| c.is_ascii_alphanumeric())
}

/// The names declared by the descriptor that fields and endpoints refer to.
struct Types {
    /// messages and entity messages
    messages: HashSet<String>,
    enums: HashSet<String>,
}

struct V {
    errors: Vec<Error>,
}

impl V {
    fn push(&mut self, e: Error) {
        self.errors.push(e);
    }

    fn identity(&mut self, d: &Descriptor) {
        let mut seen = HashSet::new();
        for (i, r) in d.roles.iter().enumerate() {
            let p = format!("roles[{}]", at(&r.name, i));
            if !key_ok(&r.name) {
                self.push(
                    Error::new("E_ROLE_NAME", format!("{p}.name"), format!("{:?} is not a role name", r.name))
                        .help("2 to 32 characters: lowercase letters, digits and _, starting with a letter"),
                );
            }
            if !seen.insert(r.name.as_str()) {
                self.push(Error::new(
                    "E_ROLE_DUP",
                    p,
                    format!("role {:?} is declared twice", r.name),
                ));
            }
        }
        let mut seen = HashSet::new();
        for (i, s) in d.scopes.iter().enumerate() {
            let p = format!("scopes[{}]", at(&s.name, i));
            if s.name == "use" {
                self.push(
                    Error::new(
                        "E_SCOPE_NAME",
                        format!("{p}.name"),
                        "scope \"use\" is implicit",
                    )
                    .help("every plugin has <id>:use; drop this scope"),
                );
            } else if !key_ok(&s.name) {
                self.push(
                    Error::new("E_SCOPE_NAME", format!("{p}.name"), format!("{:?} is not a scope name", s.name))
                        .help("2 to 32 characters: lowercase letters, digits and _, starting with a letter"),
                );
            }
            if !seen.insert(s.name.as_str()) {
                self.push(Error::new(
                    "E_SCOPE_DUP",
                    p,
                    format!("scope {:?} is declared twice", s.name),
                ));
            }
        }
        for (i, c) in d.capabilities.iter().enumerate() {
            let p = format!("capabilities[{}]", at(&c.name, i));
            if !CAPABILITIES.contains(&c.name.as_str()) {
                self.push(
                    Error::new(
                        "E_CAPABILITY_UNKNOWN",
                        format!("{p}.name"),
                        format!("there is no capability {:?}", c.name),
                    )
                    .help(format!("one of {}", CAPABILITIES.join(", "))),
                );
            }
            if c.reason.trim().is_empty() {
                self.push(
                    Error::new(
                        "E_CAPABILITY_REASON",
                        format!("{p}.reason"),
                        "a capability needs a reason",
                    )
                    .help("say why the plugin needs it; the install screen shows it to the admin"),
                );
            }
        }
        for (i, a) in d.agents.iter().enumerate() {
            let p = format!("agents[{}]", at(&a.name, i));
            if !snake_ok(&a.name) {
                self.push(
                    Error::new(
                        "E_AGENT_NAME",
                        format!("{p}.name"),
                        format!("{:?} is not an agent name", a.name),
                    )
                    .help("snake_case: lowercase letters, digits and _, starting with a letter"),
                );
            }
            for (j, r) in a.roles.iter().enumerate() {
                if !d.roles.iter().any(|x| &x.name == r) {
                    self.push(
                        Error::new(
                            "E_AGENT_ROLE_UNKNOWN",
                            format!("{p}.roles[{j}]"),
                            format!("role {r:?} is not declared by the plugin"),
                        )
                        .help("an agent gets only the plugin's own roles; declare it or drop it"),
                    );
                }
            }
            for (j, w) in a.workspace.iter().enumerate() {
                if !celobus_manifest::workspace_permission_ok(w) || a.workspace[..j].contains(w) {
                    self.push(
                        Error::new(
                            "E_AGENT_PERMISSION",
                            format!("{p}.workspace[{j}]"),
                            format!("{w:?} is not a workspace permission, or is listed twice"),
                        )
                        .help("<area>.<verb>, each once: for example \"plugins.install\" or \"members.write\""),
                    );
                }
            }
            for (j, x) in a.delegates.iter().enumerate() {
                let why = match x.split_once('.') {
                    _ if a.delegates[..j].contains(x) => Some("is listed twice"),
                    None if *x == a.name => Some("is the agent itself"),
                    None if !d.agents.iter().any(|b| &b.name == x) => {
                        Some("is not an agent of this plugin")
                    }
                    Some((plugin, name))
                        if !snake_ok(name) || !celobus_manifest::plugin_name_shape_ok(plugin) =>
                    {
                        Some("is not <dependency>.<agent>")
                    }
                    Some((plugin, _)) if !d.dependencies.iter().any(|dep| dep.plugin == plugin) => {
                        Some("names a plugin that is not a dependency")
                    }
                    _ => None,
                };
                if let Some(why) = why {
                    self.push(
                        Error::new(
                            "E_AGENT_DELEGATE",
                            format!("{p}.delegates[{j}]"),
                            format!("{x:?} {why}"),
                        )
                        .help("an agent of this plugin by its name, or <dependency>.<agent> of a plugin in dependencies, each once"),
                    );
                }
            }
            if !a.connection.is_empty() {
                let why = match a.connection.split_once('.') {
                    None => match d.connections.iter().find(|c| c.name == a.connection) {
                        None => Some("is not a connection of this plugin"),
                        Some(c) if c.kind != "llm" => Some("is not an llm connection"),
                        Some(_) => None,
                    },
                    Some((plugin, name))
                        if !celobus_manifest::connection_name_ok(name)
                            || !celobus_manifest::plugin_name_shape_ok(plugin) =>
                    {
                        Some("is not <dependency>.<connection>")
                    }
                    Some((plugin, _)) if !d.dependencies.iter().any(|dep| dep.plugin == plugin) => {
                        Some("names a plugin that is not a dependency")
                    }
                    _ => None,
                };
                if let Some(why) = why {
                    self.push(
                        Error::new(
                            "E_AGENT_CONNECTION",
                            format!("{p}.connection"),
                            format!("{:?} {why}", a.connection),
                        )
                        .help("an llm connection of this plugin by its name, or <dependency>.<connection> of a plugin in dependencies"),
                    );
                }
            }
            if !a.icon.is_empty() && !crate::agent_files::skill_name_ok(&a.icon) {
                self.push(
                    Error::new(
                        "E_AGENT_ICON",
                        format!("{p}.icon"),
                        format!("{:?} is not an icon name", a.icon),
                    )
                    .help("a lucide icon's name, at most 64 characters of lowercase letters, digits and -: git-pull-request"),
                );
            }
            if !a.engine.is_empty() && !AGENT_ENGINES.contains(&a.engine.as_str()) {
                self.push(
                    Error::new(
                        "E_AGENT_ENGINE",
                        format!("{p}.engine"),
                        format!("there is no engine {:?}", a.engine),
                    )
                    .help(format!("one of {}", AGENT_ENGINES.join(", "))),
                );
            }
            if a.model.len() > 128 || a.model.chars().any(|c| c.is_whitespace() || c.is_control()) {
                self.push(
                    Error::new(
                        "E_AGENT_MODEL",
                        format!("{p}.model"),
                        format!("{:?} is not a model id", a.model),
                    )
                    .help("the id the engine takes, at most 128 characters without spaces: claude-opus-5-5"),
                );
            }
            if a.timeout_s.is_some_and(|t| !AGENT_TIMEOUT_S.contains(&t)) {
                self.push(
                    Error::new(
                        "E_AGENT_TIMEOUT_RANGE",
                        format!("{p}.timeout_s"),
                        "a run takes between a minute and six hours",
                    )
                    .help("60 to 21600 seconds"),
                );
            }
            for (j, s) in a.skills.iter().enumerate() {
                if !crate::agent_files::skill_name_ok(s) || a.skills[..j].contains(s) {
                    self.push(
                        Error::new(
                            "E_AGENT_SKILL",
                            format!("{p}.skills[{j}]"),
                            format!("{s:?} is not a skill name, or is listed twice"),
                        )
                        .help("the name of a folder of skills/, each once: lowercase letters, digits and -"),
                    );
                }
            }
            if !a.topic.is_empty() && !d.topics.iter().any(|t| t.name == a.topic) {
                self.push(
                    Error::new(
                        "E_AGENT_TOPIC_UNKNOWN",
                        format!("{p}.topic"),
                        format!("topic {:?} is not declared by the plugin", a.topic),
                    )
                    .help("an agent's runs invalidate a topic of its own plugin: declare it in topics, or drop it"),
                );
            }
        }
        for (i, c) in d.connections.iter().enumerate() {
            let p = format!("connections[{}]", at(&c.name, i));
            if !celobus_manifest::connection_name_ok(&c.name) {
                self.push(
                    Error::new(
                        "E_CONNECTION_NAME",
                        format!("{p}.name"),
                        format!("{:?} is not a connection name", c.name),
                    )
                    .help("snake_case, at most 32 characters: lowercase letters, digits and _, starting with a letter"),
                );
            } else if d.connections[..i].iter().any(|x| x.name == c.name) {
                self.push(Error::new(
                    "E_CONNECTION_DUP",
                    p.clone(),
                    format!("connection {:?} is declared twice", c.name),
                ));
            }
            if !celobus_manifest::CONNECTION_KINDS.contains(&c.kind.as_str()) {
                self.push(
                    Error::new(
                        "E_CONNECTION_KIND",
                        format!("{p}.kind"),
                        format!("there is no connection kind {:?}", c.kind),
                    )
                    .help("llm (a language model provider for agents) or stt (speech recognition for dictation)"),
                );
            }
            if c.title.chars().count() > 80 || c.description.chars().count() > 500 {
                self.push(
                    Error::new(
                        "E_CONNECTION_TEXT",
                        p,
                        "a connection's title is at most 80 characters, its description 500",
                    )
                    .help("shorten the title or the description"),
                );
            }
        }
        let mut seen = HashSet::new();
        for (i, dep) in d.dependencies.iter().enumerate() {
            let p = format!("dependencies[{}]", at(&dep.plugin, i));
            self.errors
                .extend(plugin_id(&dep.plugin, &format!("{p}.plugin")));
            if dep.plugin == d.id {
                self.push(Error::new(
                    "E_DEP_SELF",
                    format!("{p}.plugin"),
                    "a plugin cannot depend on itself",
                ));
            }
            if !seen.insert(dep.plugin.as_str()) {
                self.push(Error::new(
                    "E_DEP_DUP",
                    p.clone(),
                    format!("dependency {:?} is declared twice", dep.plugin),
                ));
            }
            for (j, c) in dep.calls.iter().enumerate() {
                if !pascal_ok(c) {
                    self.push(Error::new(
                        "E_ENDPOINT_NAME",
                        format!("{p}.calls[{j}]"),
                        format!("{c:?} is not an endpoint name"),
                    ));
                }
            }
            let mut read = HashSet::new();
            for (j, r) in dep.reads.iter().enumerate() {
                let rp = format!("{p}.reads[{}]", at(&r.entity, j));
                if !snake_ok(&r.entity) {
                    self.push(Error::new(
                        "E_DEP_READ",
                        format!("{rp}.entity"),
                        format!("{:?} is not an entity name", r.entity),
                    ));
                }
                if !read.insert(r.entity.as_str()) {
                    self.push(Error::new(
                        "E_DEP_READ",
                        rp.clone(),
                        format!("{}.{} is read twice", dep.plugin, r.entity),
                    ));
                }
                if !r.fields.iter().any(|f| f.name == "id") {
                    self.push(
                        Error::new(
                            "E_DEP_READ",
                            format!("{rp}.fields"),
                            format!("the read of {}.{} has no id field", dep.plugin, r.entity),
                        )
                        .help("celobus plugin add generates the read; do not write it by hand"),
                    );
                }
                let mut names = HashSet::new();
                for f in &r.fields {
                    let base =
                        f.ty.strip_prefix("repeated ")
                            .or_else(|| f.ty.strip_prefix("optional "))
                            .unwrap_or(&f.ty);
                    if !snake_ok(&f.name)
                        || !names.insert(f.name.as_str())
                        || FieldType::parse(base).is_none()
                    {
                        self.push(Error::new(
                            "E_DEP_READ",
                            format!("{rp}.fields[{}]", at(&f.name, 0)),
                            format!("{:?} ({:?}) is not a field of a read", f.name, f.ty),
                        ));
                    }
                }
            }
        }
    }

    /// The notification kinds: `E_NOTIFY_CAPABILITY`, `E_NOTIFY_KINDS`, `E_NOTIFY_KIND_NAME`,
    /// `E_NOTIFY_KIND_RESERVED`, `E_NOTIFY_KIND_TEXT`.
    fn notifications(&mut self, d: &Descriptor) {
        if d.notifications.is_empty() {
            return;
        }
        if !d.capabilities.iter().any(|c| c.name == "push:send") {
            self.push(
                Error::new(
                    "E_NOTIFY_CAPABILITY",
                    "notifications",
                    "a plugin that declares notification kinds needs the capability push:send",
                )
                .help("declare push:send with a reason; the install screen shows it to the admin"),
            );
        }
        if d.notifications.len() > MAX_NOTIFICATION_KINDS {
            self.push(Error::new(
                "E_NOTIFY_KINDS",
                "notifications",
                format!(
                    "{} notification kinds; a plugin declares at most {MAX_NOTIFICATION_KINDS}",
                    d.notifications.len()
                ),
            ));
        }
        let mut seen = HashSet::new();
        for (i, k) in d.notifications.iter().enumerate() {
            let p = format!("notifications[{}]", at(&k.name, i));
            let name_ok = k.name.len() <= 32 && snake_ok(&k.name);
            if !name_ok {
                self.push(
                    Error::new(
                        "E_NOTIFY_KIND_NAME",
                        format!("{p}.name"),
                        format!("{:?} is not a notification kind name", k.name),
                    )
                    .help("snake_case, at most 32 characters (mention, thread_reply)"),
                );
            } else if !seen.insert(k.name.as_str()) {
                self.push(Error::new(
                    "E_NOTIFY_KIND_NAME",
                    p.clone(),
                    format!("notification kind {:?} is declared twice", k.name),
                ));
            }
            if RESERVED_NOTIFICATION_KINDS.contains(&k.name.as_str())
                || k.name.starts_with(RESERVED_NOTIFICATION_PREFIX)
            {
                self.push(
                    Error::new(
                        "E_NOTIFY_KIND_RESERVED",
                        format!("{p}.name"),
                        format!("{:?} is a platform kind", k.name),
                    )
                    .help("approval and decision are asks (send with an ask); run_* are the agent runs' kinds"),
                );
            }
            let title = k.title.trim().chars().count();
            if !(1..=40).contains(&title) || k.title.chars().count() > 40 {
                self.push(Error::new(
                    "E_NOTIFY_KIND_TEXT",
                    format!("{p}.title"),
                    "a notification kind's title is 1 to 40 characters",
                ));
            }
            if k.description.chars().count() > 120 {
                self.push(Error::new(
                    "E_NOTIFY_KIND_TEXT",
                    format!("{p}.description"),
                    "a notification kind's description is at most 120 characters",
                ));
            }
        }
    }

    /// The background jobs: `E_JOB_NAME`, `E_JOB_DUP`, `E_JOB_PAYLOAD`, `E_JOB_PAYLOAD_DUP`, `E_JOB_CRON`,
    /// `E_JOB_SCHEDULE_PAYLOAD`, `E_JOB_RETRY`, `E_JOB_TIMEOUT`, `E_JOB_CONCURRENCY`.
    fn jobs(&mut self, d: &Descriptor, t: &Types) {
        let mut names = HashSet::new();
        let mut payloads = HashSet::new();
        for (i, j) in d.jobs.iter().enumerate() {
            let p = format!("jobs[{}]", at(&j.name, i));
            if !job_name_ok(&j.name) {
                self.push(
                    Error::new(
                        "E_JOB_NAME",
                        format!("{p}.name"),
                        format!("{:?} is not a job name", j.name),
                    )
                    .help("snake_case, at most 32 characters: remind_unpaid"),
                );
            }
            if !names.insert(j.name.as_str()) {
                self.push(Error::new(
                    "E_JOB_DUP",
                    p.clone(),
                    format!("job {:?} is declared twice", j.name),
                ));
            }
            if !t.messages.contains(&j.payload) {
                self.push(
                    Error::new(
                        "E_JOB_PAYLOAD",
                        format!("{p}.payload"),
                        format!("{:?} is not a declared message", j.payload),
                    )
                    .help("declare the payload message (a job message)"),
                );
            } else if !payloads.insert(j.payload.as_str()) {
                self.push(
                    Error::new(
                        "E_JOB_PAYLOAD_DUP",
                        format!("{p}.payload"),
                        format!("message {:?} is the payload of another job", j.payload),
                    )
                    .help(
                        "a job is picked by its payload's type: give each job a message of its own",
                    ),
                );
            }
            if let Some(Err(e)) = j.schedule() {
                self.push(
                    Error::new("E_JOB_CRON", format!("{p}.cron"), format!("{:?}: {e}", j.cron))
                        .help("five fields in UTC, minute hour day-of-month month day-of-week: 0 9 * * 1-5"),
                );
            }
            if !j.cron.is_empty()
                && let Some(field) = required_without_default(d, &j.payload)
            {
                self.push(
                    Error::new(
                        "E_JOB_SCHEDULE_PAYLOAD",
                        format!("{p}.payload"),
                        format!(
                            "a scheduled job runs with its payload's defaults, and {field} has none"
                        ),
                    )
                    .help("make the field optional, or start the job by enqueueing it instead of a schedule"),
                );
            }
            if let Some(r) = &j.retry {
                if r.retries > crate::JOB_MAX_RETRIES {
                    self.push(Error::new(
                        "E_JOB_RETRY",
                        format!("{p}.retry.retries"),
                        format!(
                            "{} retries; a job has at most {}",
                            r.retries,
                            crate::JOB_MAX_RETRIES
                        ),
                    ));
                }
                if r.delay_ms > crate::JOB_MAX_DELAY_MS {
                    self.push(Error::new(
                        "E_JOB_RETRY",
                        format!("{p}.retry.delay_ms"),
                        format!(
                            "a retry delay of {} ms; at most one hour ({} ms)",
                            r.delay_ms,
                            crate::JOB_MAX_DELAY_MS
                        ),
                    ));
                }
            }
            if let Some(ms) = j.timeout_ms
                && !crate::JOB_TIMEOUT_MS.contains(&ms)
            {
                self.push(Error::new(
                    "E_JOB_TIMEOUT",
                    format!("{p}.timeout_ms"),
                    format!("a timeout of {ms} ms; a run's timeout is 1 s to 1 h"),
                ));
            }
            if let Some(n) = j.concurrency
                && !crate::JOB_CONCURRENCY.contains(&n)
            {
                self.push(Error::new(
                    "E_JOB_CONCURRENCY",
                    format!("{p}.concurrency"),
                    format!("a concurrency of {n}; a job runs 1 to 64 runs at once"),
                ));
            }
        }
    }

    fn types(&mut self, d: &Descriptor) -> Types {
        let mut t = Types {
            messages: HashSet::new(),
            enums: HashSet::new(),
        };
        for (i, e) in d.enums.iter().enumerate() {
            let p = format!("enums[{}]", at(&e.name, i));
            if !pascal_ok(&e.name) {
                self.push(
                    Error::new(
                        "E_MSG_NAME",
                        format!("{p}.name"),
                        format!("{:?} is not an enum name", e.name),
                    )
                    .help("PascalCase: OrderStatus"),
                );
            }
            if !t.enums.insert(e.name.clone()) {
                self.push(Error::new(
                    "E_ENUM_DUP",
                    p.clone(),
                    format!("enum {:?} is declared twice", e.name),
                ));
            }
            let mut values = HashSet::new();
            for (j, x) in e.values.iter().enumerate() {
                if !values.insert(x.as_str()) {
                    self.push(Error::new(
                        "E_ENUM_DUP",
                        format!("{p}.values[{j}]"),
                        format!("value {x:?} appears twice"),
                    ));
                }
            }
        }
        for (i, m) in d.messages.iter().enumerate() {
            let p = format!("messages[{}]", at(&m.name, i));
            if !pascal_ok(&m.name) {
                self.push(
                    Error::new(
                        "E_MSG_NAME",
                        format!("{p}.name"),
                        format!("{:?} is not a message name", m.name),
                    )
                    .help("PascalCase: PlaceOrderRequest"),
                );
            }
            if t.enums.contains(&m.name) || !t.messages.insert(m.name.clone()) {
                self.push(Error::new(
                    "E_MSG_DUP",
                    p,
                    format!("{:?} is declared twice", m.name),
                ));
            }
        }
        let mut entities = HashSet::new();
        for (i, e) in d.entities.iter().enumerate() {
            let p = format!("entities[{}]", at(&e.name, i));
            if !snake_ok(&e.name) {
                self.push(
                    Error::new(
                        "E_ENT_NAME",
                        format!("{p}.name"),
                        format!("{:?} is not an entity name", e.name),
                    )
                    .help("snake_case: order_line"),
                );
            } else if d.id.len() + 1 + e.name.len() + "_with_deleted".len() > 63 {
                self.push(
                    Error::new(
                        "E_ENT_NAME_LENGTH",
                        format!("{p}.name"),
                        format!(
                            "{:?} is too long: its table {}_{} must stay within 50 characters",
                            e.name, d.id, e.name
                        ),
                    )
                    .help("shorten the entity's name"),
                );
            } else if e.name.ends_with("_with_deleted") {
                self.push(
                    Error::new(
                        "E_ENT_NAME_RESERVED",
                        format!("{p}.name"),
                        format!(
                            "{:?} ends in _with_deleted, which the platform reserves",
                            e.name
                        ),
                    )
                    .help("rename the entity"),
                );
            }
            if !entities.insert(e.name.as_str()) {
                self.push(Error::new(
                    "E_ENT_DUP",
                    p.clone(),
                    format!("entity {:?} is declared twice", e.name),
                ));
            }
            if !matches!(e.key.as_str(), "uuid" | "i64" | "string") {
                self.push(
                    Error::new(
                        "E_ENT_KEY",
                        format!("{p}.key"),
                        format!("{:?} is not a key type", e.key),
                    )
                    .help("uuid, i64 or string"),
                );
            }
            if !pascal_ok(&e.message) {
                self.push(
                    Error::new(
                        "E_MSG_NAME",
                        format!("{p}.message"),
                        format!("{:?} is not a message name", e.message),
                    )
                    .help("PascalCase: Order"),
                );
            }
            // the SDKs list an entity's message in `messages` too: the same fields are the same message
            let listed = d
                .messages
                .iter()
                .find(|m| m.name == e.message)
                .is_some_and(|m| same_fields(&m.fields, &e.fields));
            if t.enums.contains(&e.message) || (!t.messages.insert(e.message.clone()) && !listed) {
                self.push(Error::new(
                    "E_MSG_DUP",
                    format!("{p}.message"),
                    format!("{:?} is declared twice", e.message),
                ));
            }
        }
        for (i, m) in d.messages.iter().enumerate() {
            self.fields(d, &t, &format!("messages[{}]", at(&m.name, i)), &m.fields);
        }
        for (i, e) in d.entities.iter().enumerate() {
            self.fields(d, &t, &format!("entities[{}]", at(&e.name, i)), &e.fields);
        }
        t
    }

    fn fields(&mut self, d: &Descriptor, t: &Types, p: &str, fields: &[Field]) {
        let mut names = HashSet::new();
        let mut numbers = HashSet::new();
        for (i, f) in fields.iter().enumerate() {
            let fp = format!("{p}.fields[{}]", at(&f.name, i));
            if !snake_ok(&f.name) {
                self.push(
                    Error::new(
                        "E_FIELD_NAME",
                        format!("{fp}.name"),
                        format!("{:?} is not a field name", f.name),
                    )
                    .help("snake_case: customer_id (the SDK converts camelCase and PascalCase)"),
                );
            }
            if !names.insert(f.name.as_str()) {
                self.push(Error::new(
                    "E_FIELD_DUP",
                    fp.clone(),
                    format!("field {:?} appears twice", f.name),
                ));
            }
            if f.number == 0 {
                self.push(Error::new(
                    "E_SCHEMA",
                    format!("{fp}.number"),
                    "field numbers start at 1",
                ));
            } else if !numbers.insert(f.number) {
                self.push(Error::new(
                    "E_FIELD_DUP",
                    format!("{fp}.number"),
                    format!("field number {} is used twice", f.number),
                ));
            }
            if let Some(e) = field_type(d, t, &f.ty, &format!("{fp}.type")) {
                self.push(e);
            }
        }
    }

    /// What the store needs of an entity: its `id`, the label, the storage options that exclude each other, column
    /// names, column modifiers and indexes.
    fn storage(&mut self, d: &Descriptor) {
        for (i, m) in d.messages.iter().enumerate() {
            for (j, f) in m.fields.iter().enumerate() {
                if f.unique
                    || f.index
                    || f.encrypted
                    || !f.searchable.is_empty()
                    || !f.nav.is_empty()
                    || !f.inverse.is_empty()
                    || !f.on_delete.is_empty()
                    || f.history
                {
                    self.push(
                        Error::new(
                            "E_COL_MODIFIER",
                            format!("messages[{}].fields[{}]", at(&m.name, i), at(&f.name, j)),
                            "unique, index, encrypted, searchable, nav, inverse, on_delete and history are column modifiers of an entity",
                        )
                        .help("declare them on the entity's field"),
                    );
                }
            }
        }
        for (i, e) in d.entities.iter().enumerate() {
            let p = format!("entities[{}]", at(&e.name, i));
            if !e.label.is_empty() && e.label != "id" && !e.fields.iter().any(|f| f.name == e.label)
            {
                self.push(
                    Error::new(
                        "E_ENT_LABEL",
                        format!("{p}.label"),
                        format!("{:?} is not a field of {}", e.label, e.message),
                    )
                    .help("the label names a row: one of the entity's fields"),
                );
            }
            if e.append_only && e.version {
                self.push(
                    Error::new(
                        "E_ENT_APPEND_ONLY_VERSION",
                        p.clone(),
                        "an append-only entity has no version",
                    )
                    .help("its rows never change: drop version or append_only"),
                );
            }
            if e.append_only && e.soft_delete {
                self.push(
                    Error::new(
                        "E_ENT_SOFT_DELETE_APPEND_ONLY",
                        p.clone(),
                        "an append-only entity cannot be soft-deleted",
                    )
                    .help("drop soft_delete or append_only"),
                );
            }
            match e.fields.iter().position(|f| f.name == "id") {
                None => self.push(
                    Error::new(
                        "E_PK_MISSING",
                        p.clone(),
                        format!("{} has no id field", e.message),
                    )
                    .help("an entity has a field id: Id<Self>"),
                ),
                Some(j) => {
                    let f = &e.fields[j];
                    let own = format!("ref:{}.{}", d.id, e.name);
                    let typed = f.ty == own || key_field_type(&e.key) == Some(f.ty.as_str());
                    if !typed || f.repeated || f.optional {
                        self.push(
                            Error::new(
                                "E_PK_TYPE",
                                format!("{p}.fields[id].type"),
                                format!("the id of a {} key is {own}, required and single", e.key),
                            )
                            .help("declare id as Id<Self>"),
                        );
                    }
                }
            }
            let mut columns: Vec<&str> = e.fields.iter().map(|f| f.name.as_str()).collect();
            for (j, f) in e.fields.iter().enumerate() {
                let fp = format!("{p}.fields[{}]", at(&f.name, j));
                if SYSTEM_COLUMNS.contains(&f.name.as_str()) {
                    self.push(
                        Error::new(
                            "E_COL_RESERVED",
                            format!("{fp}.name"),
                            format!("{:?} is a system column of every entity", f.name),
                        )
                        .help("the store keeps created_at, updated_at, deleted_at and version: rename the field"),
                    );
                }
                let ty = FieldType::parse(&f.ty);
                let plain = !f.repeated
                    && !matches!(
                        ty,
                        Some(FieldType::Message(_)) | Some(FieldType::Scalar("json"))
                    );
                if (f.unique || f.index) && !plain {
                    self.push(
                        Error::new(
                            "E_UNIQUE_TYPE",
                            fp,
                            "a list, a message or json column cannot be unique or indexed",
                        )
                        .help("index a scalar column"),
                    );
                }
            }
            self.search(&p, e);
            columns.extend(e.system_columns());
            for (k, ix) in e.indexes.iter().enumerate() {
                let ip = format!("{p}.indexes[{k}]");
                if ix.columns.is_empty() {
                    self.push(Error::new(
                        "E_IDX_COLUMN",
                        ip.clone(),
                        "an index has columns",
                    ));
                }
                for (c, col) in ix.columns.iter().enumerate() {
                    let known = columns.contains(&col.name.as_str());
                    let twice = ix.columns[..c].iter().any(|x| x.name == col.name);
                    let indexable = e
                        .fields
                        .iter()
                        .find(|f| f.name == col.name)
                        .is_none_or(|f| {
                            !f.repeated
                                && !matches!(
                                    FieldType::parse(&f.ty),
                                    Some(FieldType::Message(_)) | Some(FieldType::Scalar("json"))
                                )
                        });
                    if !known || twice || !indexable {
                        self.push(
                            Error::new(
                                "E_IDX_COLUMN",
                                format!("{ip}.columns[{c}]"),
                                format!(
                                    "{:?} is not an indexable column of {}, or is listed twice",
                                    col.name, e.name
                                ),
                            )
                            .help(format!("one of {}", columns.join(", "))),
                        );
                    }
                }
            }
        }
    }

    /// What an encrypted field may not be: only its ciphertext is stored, so it can
    /// neither be compared, indexed nor joined on, and the access rules cannot read it.
    fn encrypted(&mut self, d: &Descriptor) {
        for (i, e) in d.entities.iter().enumerate() {
            let p = format!("entities[{}]", at(&e.name, i));
            let encrypted: HashSet<&str> = e
                .fields
                .iter()
                .filter(|f| f.encrypted)
                .map(|f| f.name.as_str())
                .collect();
            if encrypted.is_empty() {
                continue;
            }
            for (j, f) in e.fields.iter().enumerate().filter(|(_, f)| f.encrypted) {
                let fp = format!("{p}.fields[{}]", at(&f.name, j));
                if f.name == "id" {
                    self.push(
                        Error::new("E_ENCRYPTED_KEY", fp.clone(), "the id is never encrypted")
                            .help("encrypt another field: the key finds the row"),
                    );
                }
                if f.name == e.label {
                    self.push(
                        Error::new(
                            "E_ENCRYPTED_LABEL",
                            fp.clone(),
                            format!(
                                "{} is the label of {}: refs and headers show it",
                                f.name, e.name
                            ),
                        )
                        .help("name the rows by a field that is not encrypted"),
                    );
                }
                if matches!(FieldType::parse(&f.ty), Some(FieldType::Ref { .. })) {
                    self.push(
                        Error::new(
                            "E_ENCRYPTED_REF",
                            fp.clone(),
                            format!("{} is a ref: its references and the rules read it", f.name),
                        )
                        .help("a ref is never encrypted"),
                    );
                }
                if f.unique || f.index {
                    self.push(
                        Error::new(
                            "E_ENCRYPTED_INDEX",
                            fp.clone(),
                            format!("{} is encrypted: it cannot be unique or indexed", f.name),
                        )
                        .help("drop unique and index, or encrypted"),
                    );
                }
                if !f.searchable.is_empty() {
                    self.push(
                        Error::new(
                            "E_ENCRYPTED_INDEX",
                            fp,
                            format!(
                                "{} is encrypted: it cannot be searchable (its search indexes would hold the text)",
                                f.name
                            ),
                        )
                        .help("drop searchable, or encrypted"),
                    );
                }
            }
            for (k, ix) in e.indexes.iter().enumerate() {
                for (c, col) in ix.columns.iter().enumerate() {
                    if encrypted.contains(col.name.as_str()) {
                        self.push(Error::new(
                            "E_ENCRYPTED_INDEX",
                            format!("{p}.indexes[{k}].columns[{c}]"),
                            format!("{} is encrypted: an index cannot hold it", col.name),
                        ));
                    }
                }
            }
            let Some(a) = &e.access else { continue };
            for (k, r) in a.relations.iter().enumerate() {
                if encrypted.contains(r.field.as_str()) {
                    self.push(Error::new(
                        "E_ENCRYPTED_REF",
                        format!("{p}.access.relations[{k}].field"),
                        format!("{} is encrypted: a relation reads its column", r.field),
                    ));
                }
            }
            for (k, g) in a.guards.iter().enumerate() {
                for (c, name) in g.fields.iter().enumerate() {
                    if encrypted.contains(name.as_str()) {
                        self.push(
                            Error::new(
                                "E_ENCRYPTED_GUARD",
                                format!("{p}.access.guards[{k}].fields[{c}]"),
                                format!("{name} is encrypted: a guard compares its values"),
                            )
                            .help("guard the field in the plugin's code, or do not encrypt it"),
                        );
                    }
                }
            }
            let mut grants: Vec<(String, &crate::access::Grant)> = vec![];
            for op in crate::access::OPERATIONS {
                if let Some(g) = a.grant(op) {
                    grants.push(((*op).to_string(), g));
                }
            }
            for pm in &a.permits {
                grants.push((format!("permits[{}]", pm.action), &pm.grant));
            }
            for (k, g) in a.guards.iter().enumerate() {
                for (r, rule) in g.rules.iter().enumerate() {
                    grants.push((format!("guards[{k}].rules[{r}].grant"), &rule.grant));
                }
            }
            for (at_, g) in grants {
                let mut all = vec![];
                g.walk(&mut all);
                for g in all {
                    if let crate::access::Grant::When(w) = g {
                        for c in w
                            .filter
                            .iter()
                            .filter(|c| encrypted.contains(c.field.as_str()))
                        {
                            self.push(
                                Error::new(
                                    "E_ENCRYPTED_WHEN",
                                    format!("{p}.access.{at_}"),
                                    format!(
                                        "{} is encrypted: a when filter tests its value",
                                        c.field
                                    ),
                                )
                                .help("filter on a field that is not encrypted"),
                            );
                        }
                    }
                }
            }
        }
    }

    /// `searchable` columns: single `string` columns, at most [`MAX_SEARCHABLE`], in a known language.
    fn search(&mut self, p: &str, e: &Entity) {
        let mut searchable = 0;
        for (j, f) in e.fields.iter().enumerate() {
            if f.searchable.is_empty() {
                continue;
            }
            let fp = format!("{p}.fields[{}]", at(&f.name, j));
            searchable += 1;
            if f.ty != "string" || f.repeated || f.name == "id" {
                self.push(
                    Error::new(
                        "E_SEARCH_TYPE",
                        fp.clone(),
                        format!(
                            "{} is not a text column: only a single string column is searchable",
                            f.name
                        ),
                    )
                    .help("drop searchable, or search a string column"),
                );
            }
            if !SEARCH_LANGUAGES.contains(&f.searchable.as_str()) {
                self.push(
                    Error::new(
                        "E_SEARCH_LANGUAGE",
                        format!("{fp}.searchable"),
                        format!("{:?} is not a text search language", f.searchable),
                    )
                    .help(format!("one of {}", SEARCH_LANGUAGES.join(", "))),
                );
            }
            if searchable == MAX_SEARCHABLE + 1 {
                self.push(
                    Error::new(
                        "E_SEARCH_LIMIT",
                        fp,
                        format!(
                            "{} has more than {MAX_SEARCHABLE} searchable columns",
                            e.name
                        ),
                    )
                    .help("each one keeps its own indexes: search the columns people search"),
                );
            }
        }
    }

    /// Navigations: names, duplicates, and inverse navs onto the plugin's own entities only.
    fn navigations(&mut self, d: &Descriptor) {
        for (i, e) in d.entities.iter().enumerate() {
            let p = format!("entities[{}]", at(&e.name, i));
            let mut forward: Vec<(String, &str)> = vec![];
            for (j, f) in e.fields.iter().enumerate() {
                let fp = format!("{p}.fields[{}]", at(&f.name, j));
                let single_ref = !f.repeated
                    && f.name != "id"
                    && matches!(FieldType::parse(&f.ty), Some(FieldType::Ref { .. }));
                for (what, name) in [("nav", &f.nav), ("inverse", &f.inverse)] {
                    if name.is_empty() {
                        continue;
                    }
                    if !single_ref {
                        self.push(
                            Error::new(
                                "E_REF_NAME",
                                format!("{fp}.{what}"),
                                format!(
                                    "{} is not a ref column: only a single ref has a {what}",
                                    f.name
                                ),
                            )
                            .help("declare the navigation on an Id<Target> column"),
                        );
                    } else if !snake_ok(name) {
                        self.push(
                            Error::new(
                                "E_REF_NAME",
                                format!("{fp}.{what}"),
                                format!("{name:?} is not a navigation name"),
                            )
                            .help("snake_case, at most 63 characters"),
                        );
                    }
                }
                if let Some(nav) = f.nav_name().filter(|_| single_ref) {
                    let clash = e.fields.iter().any(|g| g.name == nav && g.name != f.name);
                    if clash || forward.iter().any(|(n, _)| *n == nav) {
                        self.push(
                            Error::new(
                                "E_REF_NAV_DUP",
                                format!("{fp}.nav"),
                                format!("{} has two navigations or fields named {nav}", e.name),
                            )
                            .help("name the navigation: #[col(nav = ..)]"),
                        );
                    }
                    forward.push((nav, &f.name));
                }
                if f.inverse.is_empty() || !single_ref {
                    continue;
                }
                let Some(FieldType::Ref { plugin, entity }) = FieldType::parse(&f.ty) else {
                    continue;
                };
                let Some(target) = d
                    .entities
                    .iter()
                    .find(|t| plugin == d.id && t.name == entity)
                else {
                    self.push(
                        Error::new(
                            "E_REF_INVERSE_TARGET",
                            format!("{fp}.inverse"),
                            format!(
                                "{} refers to {plugin}.{entity}: an inverse navigation is declared onto an entity of this plugin only",
                                f.name
                            ),
                        )
                        .help("drop inverse: read the rows that point at another plugin's entity with a filter"),
                    );
                    continue;
                };
                let taken_by_field = target.fields.iter().any(|g| g.name == f.inverse);
                let taken_by_nav = target
                    .fields
                    .iter()
                    .filter(|g| g.name != "id")
                    .any(|g| g.nav_name().as_deref() == Some(f.inverse.as_str()));
                let taken_by_inverse = d
                    .entities
                    .iter()
                    .flat_map(|s| s.fields.iter().map(move |g| (s, g)))
                    .take_while(|(s, g)| !(s.name == e.name && g.name == f.name))
                    .any(|(_, g)| {
                        g.inverse == f.inverse && FieldType::parse(&g.ty) == FieldType::parse(&f.ty)
                    });
                if taken_by_field || taken_by_nav || taken_by_inverse {
                    self.push(
                        Error::new(
                            "E_REF_INVERSE_DUP",
                            format!("{fp}.inverse"),
                            format!(
                                "{} already has a field or a navigation named {}",
                                target.name, f.inverse
                            ),
                        )
                        .help("name the inverse navigation otherwise"),
                    );
                }
            }
        }
    }

    /// `on_delete` of the refs: a value its kind of ref has (`E_REF_ON_DELETE` for a ref to the plugin's own entity
    /// or a field that is not a single ref, `E_REF_CROSS_ON_DELETE` for one to another plugin's entity or the
    /// directory), `set_null` on an optional ref only, and no `set_null` in an append-only entity, whose rows never
    /// change (`E_ENT_APPEND_ONLY_CASCADE`: they go with their parent, by `cascade`, or hold it, by `restrict`).
    fn on_delete(&mut self, d: &Descriptor) {
        use crate::{ON_DELETE_CROSS, ON_DELETE_OWN};
        for (i, e) in d.entities.iter().enumerate() {
            let p = format!("entities[{}]", at(&e.name, i));
            for (j, f) in e.fields.iter().enumerate() {
                if f.on_delete.is_empty() {
                    continue;
                }
                let fp = format!("{p}.fields[{}].on_delete", at(&f.name, j));
                let target = match FieldType::parse(&f.ty) {
                    Some(FieldType::Ref { plugin, entity }) if !f.repeated && f.name != "id" => {
                        (plugin, entity)
                    }
                    _ => {
                        self.push(
                            Error::new(
                                "E_REF_ON_DELETE",
                                fp,
                                format!(
                                    "{} is not a ref column: only a single ref has an on_delete",
                                    f.name
                                ),
                            )
                            .help("declare on_delete on an Id<Target> or Option<Id<Target>> field"),
                        );
                        continue;
                    }
                };
                let own = target.0 == d.id;
                let v = f.on_delete.as_str();
                if own && !ON_DELETE_OWN.contains(&v) {
                    let help = if v == "keep" {
                        "keep is for a ref to another plugin or the directory: a ref within the plugin is a foreign key; one of restrict, cascade, set_null, no_action".to_string()
                    } else {
                        format!("one of {}", ON_DELETE_OWN.join(", "))
                    };
                    self.push(
                        Error::new(
                            "E_REF_ON_DELETE",
                            fp,
                            format!(
                                "{v:?} is not an on_delete of a ref to {}.{}",
                                target.0, target.1
                            ),
                        )
                        .help(help),
                    );
                    continue;
                }
                if !own {
                    let directory = target.0 == "platform";
                    if !ON_DELETE_CROSS.contains(&v) || (v == "set_null" && !directory) {
                        let help = if directory {
                            "a ref to the directory keeps the id (keep) or is cleared (set_null)"
                                .to_string()
                        } else {
                            format!(
                                "a ref to another plugin is not a foreign key: it keeps the id (keep); react to {}.{}.deleted instead",
                                target.0, target.1
                            )
                        };
                        self.push(
                            Error::new(
                                "E_REF_CROSS_ON_DELETE",
                                fp,
                                format!(
                                    "{v:?} is not an on_delete of a ref to {}.{}",
                                    target.0, target.1
                                ),
                            )
                            .help(help),
                        );
                        continue;
                    }
                }
                if v == "set_null" && !f.optional {
                    self.push(
                        Error::new(
                            "E_REF_ON_DELETE",
                            fp.clone(),
                            format!("{} is required: set_null clears an optional ref", f.name),
                        )
                        .help(
                            "make the field optional (Option<Id<..>>), or choose another on_delete",
                        ),
                    );
                }
                if v == "set_null" && e.append_only {
                    self.push(
                        Error::new(
                            "E_ENT_APPEND_ONLY_CASCADE",
                            fp,
                            format!("{} is append-only: set_null would change its rows", e.name),
                        )
                        .help("an append-only entity's rows go with their parent (cascade) or hold it (restrict)"),
                    );
                }
            }
        }
    }

    fn endpoints(&mut self, d: &Descriptor, t: &Types) {
        let mut seen = HashSet::new();
        let roles: HashSet<&str> = d.roles.iter().map(|r| r.name.as_str()).collect();
        let scopes: HashSet<&str> = d
            .scopes
            .iter()
            .map(|s| s.name.as_str())
            .chain(IMPLICIT_SCOPES.iter().copied())
            .collect();
        for (i, e) in d.endpoints.iter().enumerate() {
            let p = format!("endpoints[{}]", at(&e.name, i));
            if !pascal_ok(&e.name) {
                self.push(
                    Error::new(
                        "E_ENDPOINT_NAME",
                        format!("{p}.name"),
                        format!("{:?} is not an endpoint name", e.name),
                    )
                    .help("PascalCase: PlaceOrder"),
                );
            }
            if !seen.insert(e.name.as_str()) {
                self.push(Error::new(
                    "E_ENDPOINT_DUP",
                    p.clone(),
                    format!("endpoint {:?} is declared twice", e.name),
                ));
            }
            for (what, name) in [("input", &e.input), ("output", &e.output)] {
                if !t.messages.contains(name) {
                    self.push(
                        Error::new(
                            "E_ENDPOINT_MESSAGE",
                            format!("{p}.{what}"),
                            format!("{name:?} is not a declared message or entity message"),
                        )
                        .help("declare the message, or use an entity's message"),
                    );
                }
            }
            for (j, r) in e.roles.iter().enumerate() {
                let workspace = r.strip_prefix("workspace.").is_some_and(key_ok);
                if !workspace && !roles.contains(r.as_str()) {
                    self.push(
                        Error::new(
                            "E_ENDPOINT_ROLE_UNKNOWN",
                            format!("{p}.roles[{j}]"),
                            format!("role {r:?} is used but not declared by the plugin"),
                        )
                        .help("declare the role on the plugin, or use a workspace role (workspace.admin)"),
                    );
                }
            }
            for (j, s) in e.scopes.iter().enumerate() {
                if !scopes.contains(s.as_str()) {
                    self.push(
                        Error::new(
                            "E_ENDPOINT_SCOPE_UNKNOWN",
                            format!("{p}.scopes[{j}]"),
                            format!("scope {s:?} is used but not declared by the plugin"),
                        )
                        .help("declare the scope on the plugin, or use read or write"),
                    );
                }
            }
            if e.scopes.len() > 1 {
                self.push(
                    Error::new(
                        "E_ENDPOINT_SCOPE_MANY",
                        format!("{p}.scopes"),
                        format!("{} scopes; an endpoint takes one scope", e.scopes.len()),
                    )
                    .help("keep one scope (the default is write)"),
                );
            }
            if let Some(ms) = e.timeout_ms
                && !(1..=MAX_TIMEOUT_MS).contains(&ms)
            {
                self.push(
                    Error::new(
                        "E_ENDPOINT_TIMEOUT_RANGE",
                        format!("{p}.timeout_ms"),
                        format!("timeout {ms} ms is out of range"),
                    )
                    .help("1 ms to 600 s"),
                );
            }
        }
    }
}

/// Two field lists that describe the same message (names, numbers, types; not descriptions or column modifiers).
fn same_fields(a: &[Field], b: &[Field]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            (&x.name, x.number, &x.ty, x.repeated, x.optional)
                == (&y.name, y.number, &y.ty, y.repeated, y.optional)
        })
}

/// `E_FIELD_TYPE` for a type that is not one, or names a message, enum or entity that does not exist.
fn field_type(d: &Descriptor, t: &Types, ty: &str, path: &str) -> Option<Error> {
    let err = |m: String| Some(Error::new("E_FIELD_TYPE", path, m));
    match FieldType::parse(ty) {
        None => err(format!("{ty:?} is not a field type"))
            .map(|e| e.help("a scalar (string, int64, decimal, ...), message:<Name>, enum:<Name> or ref:<plugin>.<entity>")),
        Some(FieldType::Scalar(_)) => None,
        Some(FieldType::Message(m)) => (!t.messages.contains(&m))
            .then(|| Error::new("E_FIELD_TYPE", path, format!("message {m:?} is not declared"))),
        Some(FieldType::Enum(n)) => (!t.enums.contains(&n))
            .then(|| Error::new("E_FIELD_TYPE", path, format!("enum {n:?} is not declared"))),
        Some(FieldType::Ref { plugin, entity }) => {
            if plugin == d.id {
                (!d.entities.iter().any(|e| e.name == entity)).then(|| {
                    Error::new("E_FIELD_TYPE", path, format!("entity {entity:?} is not declared"))
                })
            } else if plugin == "platform" {
                (!matches!(entity.as_str(), "user" | "group")).then(|| {
                    Error::new(
                        "E_FIELD_TYPE",
                        path,
                        format!("the directory has no {entity:?}"),
                    )
                    .help("ref:platform.user or ref:platform.group")
                })
            } else if !d.dependencies.iter().any(|x| x.plugin == plugin) {
                Some(
                    Error::new(
                        "E_FIELD_TYPE",
                        path,
                        format!("{plugin:?} is neither this plugin nor a dependency"),
                    )
                    .help(format!("declare {plugin} as a dependency")),
                )
            } else {
                None
            }
        }
    }
}
