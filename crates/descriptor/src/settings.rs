//! Plugin settings: what an owner or an admin of the workspace sets for an installed plugin (a webhook, a limit, an
//! API key), declared by the plugin as one message of kind `settings` and a form over its fields.
//!
//! This module is the one implementation of their rules, shared by `celobus plugin check`, the platform when it saves
//! values and the CLI before it sends them:
//!
//! - [`validate`]: the definition-time checks of the descriptor's `settings` and of the settings fields' options
//!   (`E_SETTINGS_*`, `E_FIELD_VALIDATOR_SCOPE`), and [`warnings`] (`W_SETTINGS_REQUIRED_NO_DEFAULT`);
//! - [`validate_settings_value`]: the values an owner or an admin saves, checked in order type → required →
//!   validators, with the messages the settings screen shows ([`FieldError`]);
//! - [`effective`], [`missing`], [`to_plugin_json`]: the values a plugin reads (the saved ones, else the defaults);
//! - [`convert`]: saved values brought to another version's settings (a removed field's value dropped, a value that
//!   no longer fits reset to its default);
//! - [`changes`]: the publish check's view of a settings change (breaking or not), what an upgrade's plan says;
//! - [`derive_form`], [`complete_form`]: the form an SDK writes when the plugin lays none out, and the inputs of a
//!   form it lays out filled from their fields.
//!
//! Values are JSON, keyed by the descriptor's field names (`thread_warm_hours`): strings and secrets as strings,
//! `bool`, numbers as numbers (`int64` too), decimals as strings (`"12.50"`), dates as `YYYY-MM-DD`, an enum's value by
//! its name, a member by their user id, a list as an array. A plugin reads them in its own JSON mapping
//! ([`to_plugin_json`]).

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::lock::Change;
use crate::ui::{Control, FormSection, Input, Opt, Text};
use crate::{Descriptor, Enum, Error, Field, FieldType, Message, MessageKind};

/// The most fields the settings message has.
pub const MAX_SETTINGS_FIELDS: usize = 64;
/// The most secrets among them.
pub const MAX_SECRETS: usize = 16;
/// The largest value of one field, as JSON: 64 KiB.
pub const MAX_VALUE_BYTES: usize = 64 << 10;
/// How long the platform waits for the settings check hook's answer.
pub const SETTINGS_CHECK_TIMEOUT_MS: u64 = 10_000;

/// The descriptor's `settings`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// The message of kind `settings` whose fields the settings are.
    pub message: String,
    /// The plugin checks a change before it is saved: the platform sends it the settings check hook
    /// ([`crate::SETTINGS_CHECK_METHOD`]).
    #[serde(default)]
    pub on_change: bool,
    /// The form owners and admins fill: every field of the message once.
    #[serde(default)]
    pub form: SettingsForm,
}

/// The settings form: sections of the plugin UI's form inputs (catalog 3), each input naming a field.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct SettingsForm {
    pub sections: Vec<FormSection>,
}

/// A settings value refused: the field's name (descriptor spelling) and the message shown under it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FieldError {
    pub name: String,
    pub message: String,
}

impl FieldError {
    fn new(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            message: message.into(),
        }
    }
}

/// What a settings field holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    String,
    Bool,
    Int32,
    Int64,
    Double,
    Decimal,
    Date,
    Secret,
    /// An enum's value, by name.
    Enum(Vec<String>),
    /// A member of the workspace (`ref:platform.user`), by user id.
    User,
}

impl Kind {
    fn numeric(&self) -> bool {
        matches!(
            self,
            Kind::Int32 | Kind::Int64 | Kind::Double | Kind::Decimal | Kind::Date
        )
    }
}

/// The kind of a settings field: `None` for a type settings do not take (a message, `json`, `file`, `bytes`, `uuid`,
/// `timestamp`, a ref to anything but `platform.user`, a list of anything but strings, enums and members, an optional
/// list).
pub fn kind(d: &Descriptor, f: &Field) -> Option<Kind> {
    let k = match FieldType::parse(&f.ty)? {
        FieldType::Scalar(s) => match s {
            "string" => Kind::String,
            "bool" => Kind::Bool,
            "int32" => Kind::Int32,
            "int64" => Kind::Int64,
            "double" => Kind::Double,
            "decimal" => Kind::Decimal,
            "date" => Kind::Date,
            "secret" => Kind::Secret,
            _ => return None,
        },
        FieldType::Enum(e) => Kind::Enum(d.enums.iter().find(|x| x.name == e)?.values.clone()),
        FieldType::Ref { plugin, entity } if plugin == "platform" && entity == "user" => Kind::User,
        _ => return None,
    };
    if f.repeated && (f.optional || !matches!(k, Kind::String | Kind::Enum(_) | Kind::User)) {
        return None;
    }
    Some(k)
}

/// Whether a field needs a value before the plugin works: neither optional nor a list, and without a default.
pub fn is_required(f: &Field) -> bool {
    !f.optional && !f.repeated && f.default.is_none()
}

/// The plugin's settings message, when it declares settings.
pub fn settings_message(d: &Descriptor) -> Option<&Message> {
    let s = d.settings.as_ref()?;
    d.messages.iter().find(|m| m.name == s.message)
}

/// The settings field named `name`, its descriptor name (`thread_warm_hours`) or its JSON name (`threadWarmHours`).
pub fn field<'a>(d: &'a Descriptor, name: &str) -> Option<&'a Field> {
    settings_message(d)?
        .fields
        .iter()
        .find(|f| f.name == name)
        .or_else(|| {
            settings_message(d)?
                .fields
                .iter()
                .find(|f| json_name(&f.name) == name)
        })
}

/// The protobuf JSON name of a field: `owner_id` → `ownerId`.
pub fn json_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for c in name.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// A name as a label: `thread_warm_hours` → `Thread warm hours` (underscores to spaces, the first letter capital).
pub fn humanize(name: &str) -> String {
    let s = name.replace('_', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}

/// Whether the field carries any of the options of a settings field.
pub fn has_settings_options(f: &Field) -> bool {
    f.label.is_some()
        || f.default.is_some()
        || f.section.is_some()
        || f.placeholder.is_some()
        || f.multiline
        || f.min.is_some()
        || f.max.is_some()
        || f.min_len.is_some()
        || f.max_len.is_some()
        || f.pattern.is_some()
        || f.format.is_some()
        || f.min_items.is_some()
        || f.max_items.is_some()
        || f.message.is_some()
}

// ------------------------------------------------------------------------------------------------ values

/// Checks the values to save — every value the settings will hold, secrets included, by field name — in order type →
/// required → validators, and gives them back normalized (numbers as numbers, decimals and dates as strings). A
/// missing or `null` value is not set: a required field without one is refused (`Required`). Every refused value is
/// named, in the order of the fields (an unknown name last).
pub fn validate_settings_value(
    d: &Descriptor,
    values: &Map<String, Value>,
) -> Result<Map<String, Value>, Vec<FieldError>> {
    let Some(m) = settings_message(d) else {
        return Err(vec![FieldError::new("", "The plugin has no settings")]);
    };
    let mut out = Map::new();
    let mut errors = vec![];
    for f in &m.fields {
        match values.get(&f.name) {
            None | Some(Value::Null) => {
                if is_required(f) {
                    errors.push(FieldError::new(&f.name, "Required"));
                }
            }
            Some(v) => match check_value(d, f, v) {
                Ok(v) => {
                    out.insert(f.name.clone(), v);
                }
                Err(message) => errors.push(FieldError::new(&f.name, message)),
            },
        }
    }
    for k in values.keys() {
        if !m.fields.iter().any(|f| f.name == *k) {
            errors.push(FieldError::new(k, "Not a setting"));
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// One value of the settings field `f`: its type, then its validators. The normalized value, or the message.
pub fn check_value(d: &Descriptor, f: &Field, v: &Value) -> Result<Value, String> {
    let Some(k) = kind(d, f) else {
        return Err("Not a setting".into());
    };
    let v = if f.repeated {
        let Value::Array(items) = v else {
            return Err(type_message(&k));
        };
        Value::Array(
            items
                .iter()
                .map(|x| typed(&k, x))
                .collect::<Result<Vec<_>, _>>()?,
        )
    } else {
        typed(&k, v)?
    };
    if v.to_string().len() > MAX_VALUE_BYTES {
        return Err("Too large (at most 64 KiB)".into());
    }
    if let Some(m) = validators(&k, f, &v) {
        return Err(f.message.clone().unwrap_or(m));
    }
    Ok(v)
}

/// The message of a value that is not of its field's type: what the settings screen says to fill in. A field's own
/// `message` never replaces it (it replaces the validators' texts only).
fn type_message(k: &Kind) -> String {
    match k {
        Kind::String | Kind::Secret => "Enter text",
        Kind::Bool => "Choose on or off",
        Kind::Int32 | Kind::Int64 => "Enter a whole number",
        Kind::Double | Kind::Decimal => "Enter a number",
        Kind::Date => "Enter a date",
        Kind::Enum(_) => "Choose one of the options",
        Kind::User => "Choose people",
    }
    .into()
}

/// A value of kind `k`, normalized; the type's message when it is not one.
fn typed(k: &Kind, v: &Value) -> Result<Value, String> {
    let out = match k {
        Kind::String | Kind::Secret => v.as_str().map(|s| Value::String(s.into())),
        Kind::Bool => v.as_bool().map(Value::Bool),
        Kind::Int32 => integer(v)
            .and_then(|n| i32::try_from(n).ok())
            .map(Value::from),
        Kind::Int64 => integer(v)
            .and_then(|n| i64::try_from(n).ok())
            .map(Value::from),
        Kind::Double => v
            .as_f64()
            .filter(|x| x.is_finite())
            .and_then(|_| v.as_number().cloned().map(Value::Number)),
        Kind::Decimal => match v {
            Value::String(s) if decimal_parts(s.trim()).is_some() => {
                Some(Value::String(s.trim().into()))
            }
            Value::Number(n) if decimal_parts(&n.to_string()).is_some() => {
                Some(Value::String(n.to_string()))
            }
            _ => None,
        },
        Kind::Date => v
            .as_str()
            .filter(|s| date_ok(s))
            .map(|s| Value::String(s.into())),
        Kind::Enum(values) => v
            .as_str()
            .filter(|s| values.iter().any(|x| x == s))
            .map(|s| Value::String(s.into())),
        Kind::User => v
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 128 && !s.contains(char::is_whitespace))
            .map(|s| Value::String(s.into())),
    };
    out.ok_or_else(|| type_message(k))
}

/// Whether a value in the descriptor (a default, a bound) is written as its kind is: numbers as JSON numbers, `bool`
/// as a JSON boolean, everything else as a string.
fn spelled(k: &Kind, v: &Value) -> bool {
    match k {
        Kind::Int32 | Kind::Int64 | Kind::Double => v.is_number(),
        Kind::Bool => v.is_boolean(),
        _ => v.is_string(),
    }
}

fn integer(v: &Value) -> Option<i128> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
            .or_else(|| {
                n.as_f64()
                    .filter(|f| f.fract() == 0.0 && f.abs() < 1e19)
                    .map(|f| f as i128)
            }),
        Value::String(s) => {
            let s = s.trim();
            let digits = s.strip_prefix('-').unwrap_or(s);
            (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                .then(|| s.parse::<i128>().ok())
                .flatten()
        }
        _ => None,
    }
}

/// A decimal's sign, integer digits (no leading zeros) and fraction digits (no trailing zeros): `-?\d+(\.\d+)?`.
fn decimal_parts(s: &str) -> Option<(bool, String, String)> {
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (int, frac) = rest.split_once('.').unwrap_or((rest, ""));
    if int.is_empty()
        || !int.bytes().all(|b| b.is_ascii_digit())
        || (rest.contains('.') && frac.is_empty())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let int = int.trim_start_matches('0').to_string();
    let frac = frac.trim_end_matches('0').to_string();
    let zero = int.is_empty() && frac.is_empty();
    Some((neg && !zero, int, frac))
}

fn cmp_decimal(a: &str, b: &str) -> Option<Ordering> {
    let (an, ai, af) = decimal_parts(a)?;
    let (bn, bi, bf) = decimal_parts(b)?;
    let magnitude = ai
        .len()
        .cmp(&bi.len())
        .then_with(|| ai.cmp(&bi))
        .then_with(|| {
            let n = af.len().max(bf.len());
            format!("{af:0<n$}").cmp(&format!("{bf:0<n$}"))
        });
    Some(match (an, bn) {
        (false, false) => magnitude,
        (true, true) => magnitude.reverse(),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
    })
}

fn date_ok(s: &str) -> bool {
    s.len() == 10 && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// Orders two values of kind `k` (bounds and values): numbers, decimals, dates.
fn cmp_value(k: &Kind, a: &Value, b: &Value) -> Option<Ordering> {
    match k {
        Kind::Int32 | Kind::Int64 => Some(integer(a)?.cmp(&integer(b)?)),
        Kind::Double => a.as_f64()?.partial_cmp(&b.as_f64()?),
        Kind::Decimal => cmp_decimal(&decimal_text(a)?, &decimal_text(b)?),
        Kind::Date => {
            let (a, b) = (a.as_str()?, b.as_str()?);
            (date_ok(a) && date_ok(b)).then(|| a.cmp(b))
        }
        _ => None,
    }
}

fn decimal_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A bound as the messages write it: numbers with thousands separators (`20,000`, `-1,500.5`), dates as they are.
pub fn display_bound(v: &Value) -> String {
    let text = match v {
        Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    };
    if decimal_parts(&text).is_none() {
        return text;
    }
    let (sign, rest) = match text.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", text.as_str()),
    };
    let (int, frac) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    format!(
        "{sign}{}{}",
        thousands(int),
        frac.map(|f| format!(".{f}")).unwrap_or_default()
    )
}

fn thousands(digits: &str) -> String {
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn count(n: u32) -> String {
    thousands(&n.to_string())
}

/// The platform's message for the first validator of `f` the (typed) value fails, if any.
fn validators(k: &Kind, f: &Field, v: &Value) -> Option<String> {
    if f.repeated {
        let items = v.as_array()?;
        let n = items.len() as u64;
        if let Some(min) = f.min_items
            && n < u64::from(min)
        {
            return Some(format!("At least {} items", count(min)));
        }
        if let Some(max) = f.max_items
            && n > u64::from(max)
        {
            return Some(format!("At most {} items", count(max)));
        }
        return items.iter().find_map(|x| scalar_validators(k, f, x));
    }
    scalar_validators(k, f, v)
}

fn scalar_validators(k: &Kind, f: &Field, v: &Value) -> Option<String> {
    if k.numeric() {
        let below = f
            .min
            .as_ref()
            .is_some_and(|m| cmp_value(k, v, m) == Some(Ordering::Less));
        let above = f
            .max
            .as_ref()
            .is_some_and(|m| cmp_value(k, v, m) == Some(Ordering::Greater));
        if below || above {
            return Some(match (&f.min, &f.max) {
                (Some(a), Some(b)) => format!("From {} to {}", display_bound(a), display_bound(b)),
                (Some(a), None) => format!("At least {}", display_bound(a)),
                (None, Some(b)) => format!("At most {}", display_bound(b)),
                (None, None) => unreachable!("a bound was crossed"),
            });
        }
        return None;
    }
    let s = v.as_str()?;
    let chars = s.chars().count() as u64;
    if let Some(n) = f.min_len
        && chars < u64::from(n)
    {
        return Some(format!("At least {} characters", count(n)));
    }
    if let Some(n) = f.max_len
        && chars > u64::from(n)
    {
        return Some(format!("At most {} characters", count(n)));
    }
    if let Some(p) = &f.pattern
        && compile_pattern(p).is_ok_and(|re| !re.is_match(s))
    {
        return Some("Doesn't match the expected format".into());
    }
    match f.format.as_deref() {
        Some("url") if !url_ok(s) => Some("Not a URL".into()),
        Some("email") if !email_ok(s) => Some("Not an email address".into()),
        _ => None,
    }
}

/// An absolute `http` or `https` URL with a host, and no spaces.
pub fn url_ok(s: &str) -> bool {
    if s.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        match host_port.find(']') {
            Some(i) => &host_port[..=i],
            None => return false,
        }
    } else {
        let (host, port) = host_port.split_once(':').unwrap_or((host_port, ""));
        if !port.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        host
    };
    !host.is_empty() && !host.starts_with('.') && !host.ends_with('.')
}

/// One `@`, a local part, a domain with a dot (not at either end), no spaces.
pub fn email_ok(s: &str) -> bool {
    if s.chars().any(char::is_whitespace) || s.matches('@').count() != 1 {
        return false;
    }
    let (local, domain) = s.split_once('@').unwrap_or_default();
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

// ------------------------------------------------------------------------------------------------ patterns

/// The engine a `pattern` is checked with here: the `regex` crate's syntax restricted to what a browser's `RegExp`
/// with the flag `u` reads the same way, translated so that both match the same strings (`.` stops at any line
/// break, `\d` and `\w` are ASCII). `Err` names what is not allowed:
///
/// - literals; `\` before one of `^$\.*+?()[]{}|/`, `\n`, `\r`, `\t`, `\d`, `\D`, `\w`, `\W` (`\-` inside a class);
/// - `.`, `^`, `$`, `|`, groups `( )` and `(?: )`;
/// - `*`, `+`, `?`, `{n}`, `{n,}`, `{n,m}`, each optionally lazy (`?`);
/// - classes `[...]`, `[^...]` with literals, ranges, `\d`, `\w`; not empty, no `[`, `&&`, `--`, `~~` inside.
///
/// Not allowed: `\s`, `\b`, `\p{..}`, other escapes, named groups, lookaround, backreferences, inline flags.
pub fn compile_pattern(p: &str) -> Result<regex::Regex, String> {
    let rust = translate_pattern(p)?;
    regex::RegexBuilder::new(&rust)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| {
            let text = e.to_string();
            let last = text
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            format!(
                "does not compile: {}",
                last.trim().trim_start_matches("error: ")
            )
        })
}

fn translate_pattern(p: &str) -> Result<String, String> {
    const SYNTAX: &str = "^$\\.*+?()[]{}|/";
    let chars: Vec<char> = p.chars().collect();
    let mut out = String::with_capacity(p.len() + 8);
    let mut in_class = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\\' {
            let Some(e) = next else {
                return Err("ends with a lone \\".into());
            };
            match e {
                'd' => out.push_str(if in_class { "0-9" } else { "[0-9]" }),
                'w' => out.push_str(if in_class {
                    "0-9A-Za-z_"
                } else {
                    "[0-9A-Za-z_]"
                }),
                'D' | 'W' if in_class => {
                    return Err(format!("\\{e} inside a class is not allowed"));
                }
                'D' => out.push_str("[^0-9]"),
                'W' => out.push_str("[^0-9A-Za-z_]"),
                'n' | 'r' | 't' => {
                    out.push('\\');
                    out.push(e);
                }
                '/' => out.push('/'),
                '-' if in_class => out.push_str("\\-"),
                e if SYNTAX.contains(e) => {
                    out.push('\\');
                    out.push(e);
                }
                e => {
                    return Err(format!(
                        "\\{e} is not allowed (browsers and the platform read it differently)"
                    ));
                }
            }
            i += 2;
            continue;
        }
        if in_class {
            match c {
                ']' => {
                    in_class = false;
                    out.push(']');
                }
                '[' => return Err("[ inside a class is not allowed: escape it (\\[)".into()),
                '&' | '-' | '~' if next == Some(c) => {
                    return Err(format!("{c}{c} inside a class is not allowed"));
                }
                _ => out.push(c),
            }
            i += 1;
            continue;
        }
        match c {
            '[' => {
                in_class = true;
                out.push('[');
                if next == Some('^') {
                    out.push('^');
                    i += 1;
                }
                if chars.get(i + 1) == Some(&']') {
                    return Err("an empty class ([] or [^]) is not allowed".into());
                }
            }
            ']' | '}' => {
                return Err(format!("a lone {c} is not allowed: escape it (\\{c})"));
            }
            '{' => {
                let end = chars[i..]
                    .iter()
                    .position(|x| *x == '}')
                    .map(|n| i + n)
                    .ok_or("a lone { is not allowed: escape it (\\{)")?;
                let body: String = chars[i + 1..end].iter().collect();
                let (min, max) = body.split_once(',').unwrap_or((&body, "0"));
                if min.is_empty()
                    || !min.bytes().all(|b| b.is_ascii_digit())
                    || !max.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err(format!(
                        "{{{body}}} is not a repetition ({{n}}, {{n,}} or {{n,m}})"
                    ));
                }
                out.push('{');
                out.push_str(&body);
                out.push('}');
                i = end + 1;
                continue;
            }
            '(' if next == Some('?') => {
                if chars.get(i + 2) != Some(&':') {
                    return Err(
                        "only (...) and (?:...) groups are allowed (no names, lookaround or flags)"
                            .into(),
                    );
                }
                out.push_str("(?:");
                i += 3;
                continue;
            }
            '.' => out.push_str("[^\\n\\r\\x{2028}\\x{2029}]"),
            _ => out.push(c),
        }
        i += 1;
    }
    if in_class {
        return Err("a class is not closed (])".into());
    }
    Ok(out)
}

// ------------------------------------------------------------------------------------------------ reading

/// The values a plugin reads, by field name: the saved ones, else the defaults; a field with neither is absent.
pub fn effective(d: &Descriptor, saved: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(m) = settings_message(d) else {
        return out;
    };
    for f in &m.fields {
        match saved.get(&f.name).filter(|v| !v.is_null()) {
            Some(v) => {
                out.insert(f.name.clone(), v.clone());
            }
            None => {
                if let Some(v) = &f.default {
                    out.insert(f.name.clone(), v.clone());
                }
            }
        }
    }
    out
}

/// The required fields without a saved value: the plugin needs setup until they have one.
pub fn missing(d: &Descriptor, saved: &Map<String, Value>) -> Vec<String> {
    settings_message(d)
        .map(|m| {
            m.fields
                .iter()
                .filter(|f| is_required(f) && saved.get(&f.name).is_none_or(Value::is_null))
                .map(|f| f.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Values by field name (as [`effective`] gives them) in the plugin's JSON mapping: lowerCamelCase names, `int64`
/// as a string. What the plugin's process reads (`Registered.settings`, `SettingsChanged`, the check hook).
pub fn to_plugin_json(d: &Descriptor, values: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(m) = settings_message(d) else {
        return out;
    };
    for f in &m.fields {
        let Some(v) = values.get(&f.name) else {
            continue;
        };
        let v = match (f.ty.as_str(), v) {
            ("int64", Value::Number(n)) => Value::String(n.to_string()),
            _ => v.clone(),
        };
        out.insert(json_name(&f.name), v);
    }
    out
}

// ------------------------------------------------------------------------------------------------ versions

/// Saved values brought to another version's settings ([`convert`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Converted {
    /// What stays, normalized.
    pub values: Map<String, Value>,
    /// The fields the version no longer has: their values are gone.
    pub dropped: Vec<String>,
    /// The fields whose values no longer fit their type or validators: back to their defaults.
    pub reset: Vec<String>,
}

/// Brings saved values (by field name) to the settings of `new`: a value of a field it no longer has is dropped, a
/// value that does not pass the field's type and validators is reset (left out, so the default applies), the rest
/// stays. Nothing is said about required fields: [`missing`] tells which still need a value.
pub fn convert(new: &Descriptor, saved: &Map<String, Value>) -> Converted {
    let mut out = Converted::default();
    for (name, v) in saved {
        if v.is_null() {
            continue;
        }
        let Some(f) = settings_message(new).and_then(|m| m.fields.iter().find(|f| f.name == *name))
        else {
            out.dropped.push(name.clone());
            continue;
        };
        match check_value(new, f, v) {
            Ok(v) => {
                out.values.insert(name.clone(), v);
            }
            Err(_) => out.reset.push(name.clone()),
        }
    }
    out
}

/// The settings changes from `old` to `new`, for the publish check and an upgrade's plan. Breaking (a new major
/// version): settings removed (`E_SETTINGS_REMOVED`), a field removed (`E_SETTINGS_FIELD_REMOVED`, "Drops setting X"),
/// its type changed (`E_SETTINGS_FIELD_TYPE_CHANGED`) or a validator tightened (`E_SETTINGS_VALIDATOR_TIGHTENED`; both
/// "Resets X if it no longer fits"), a field required that was not, without a default (`E_SETTINGS_REQUIRED_ADDED`,
/// "Needs X before it works"). Not breaking but a new minor version: settings added (`W_SETTINGS_ADDED`), a field added
/// with a default or optional (`W_SETTINGS_FIELD_ADDED`), a validator loosened or a field no longer required
/// (`W_SETTINGS_VALIDATOR_LOOSENED`), a default changed (`W_SETTINGS_DEFAULT_CHANGED`). Labels, sections, texts, the
/// form and the hook change nothing.
pub fn changes(old: &Descriptor, new: &Descriptor) -> Vec<Change> {
    let mut out = vec![];
    let (om, nm) = (settings_message(old), settings_message(new));
    match (om, nm) {
        (None, None) => return out,
        (Some(_), None) => {
            out.push(Change::new(
                "E_SETTINGS_REMOVED",
                "settings",
                "the plugin no longer has settings: every saved value is dropped",
                true,
            ));
            return out;
        }
        (None, Some(_)) => out.push(Change::new(
            "W_SETTINGS_ADDED",
            "settings",
            "the plugin has settings now",
            false,
        )),
        (Some(_), Some(_)) => {}
    }
    let empty = vec![];
    let ofields = om.map(|m| &m.fields).unwrap_or(&empty);
    let nfields = nm.map(|m| &m.fields).unwrap_or(&empty);
    for o in ofields {
        if !nfields.iter().any(|n| n.name == o.name) {
            out.push(Change::new(
                "E_SETTINGS_FIELD_REMOVED",
                format!("settings.fields[{}]", o.name),
                format!("Drops setting {}: its saved value is gone", o.name),
                true,
            ));
        }
    }
    for n in nfields {
        let path = format!("settings.fields[{}]", n.name);
        let Some(o) = ofields.iter().find(|o| o.name == n.name) else {
            if is_required(n) {
                out.push(Change::new(
                    "E_SETTINGS_REQUIRED_ADDED",
                    path,
                    format!(
                        "Needs {} before it works: a new required setting without a default",
                        n.name
                    ),
                    true,
                ));
            } else {
                out.push(Change::new(
                    "W_SETTINGS_FIELD_ADDED",
                    path,
                    format!("adds setting {}", n.name),
                    false,
                ));
            }
            continue;
        };
        if o.ty != n.ty || o.repeated != n.repeated {
            out.push(Change::new(
                "E_SETTINGS_FIELD_TYPE_CHANGED",
                path,
                format!(
                    "Resets {} if it no longer fits: its type changed from {} to {}",
                    n.name,
                    crate::lock::wire_type(o),
                    crate::lock::wire_type(n)
                ),
                true,
            ));
            continue;
        }
        match (is_required(o), is_required(n)) {
            (false, true) => out.push(Change::new(
                "E_SETTINGS_REQUIRED_ADDED",
                path.clone(),
                format!(
                    "Needs {} before it works: it is required now, without a default",
                    n.name
                ),
                true,
            )),
            (true, false) => out.push(Change::new(
                "W_SETTINGS_VALIDATOR_LOOSENED",
                path.clone(),
                format!("{} is no longer required", n.name),
                false,
            )),
            _ => {}
        }
        if o.default.is_some() && n.default.is_some() && o.default != n.default {
            out.push(Change::new(
                "W_SETTINGS_DEFAULT_CHANGED",
                path.clone(),
                format!("the default of {} changed", n.name),
                false,
            ));
        }
        let k = kind(new, n);
        let (mut tight, mut loose) = (vec![], vec![]);
        // a lower bound tightens when it appears or rises, an upper one when it appears or falls
        let mut bound =
            |what: &str, k: Option<&Kind>, o: Option<&Value>, n: Option<&Value>, lower: bool| {
                let tighter = match (o, n) {
                    (None, None) => return,
                    (None, Some(_)) => true,
                    (Some(_), None) => false,
                    (Some(a), Some(b)) => match k.and_then(|k| cmp_value(k, b, a)) {
                        Some(Ordering::Equal) => return,
                        Some(ord) => (ord == Ordering::Greater) == lower,
                        None if a == b => return,
                        None => true,
                    },
                };
                if tighter { &mut tight } else { &mut loose }.push(what.to_string());
            };
        let num = |x: Option<u32>| x.map(Value::from);
        let count = Some(&Kind::Int64);
        bound("min", k.as_ref(), o.min.as_ref(), n.min.as_ref(), true);
        bound("max", k.as_ref(), o.max.as_ref(), n.max.as_ref(), false);
        for (what, a, b, lower) in [
            ("min_len", o.min_len, n.min_len, true),
            ("max_len", o.max_len, n.max_len, false),
            ("min_items", o.min_items, n.min_items, true),
            ("max_items", o.max_items, n.max_items, false),
        ] {
            bound(what, count, num(a).as_ref(), num(b).as_ref(), lower);
        }
        for (what, a, b) in [
            ("pattern", &o.pattern, &n.pattern),
            ("format", &o.format, &n.format),
        ] {
            match (a, b) {
                (None, Some(_)) => tight.push(what.to_string()),
                (Some(x), Some(y)) if x != y => tight.push(what.to_string()),
                (Some(_), None) => loose.push(what.to_string()),
                _ => {}
            }
        }
        if !tight.is_empty() {
            out.push(Change::new(
                "E_SETTINGS_VALIDATOR_TIGHTENED",
                path.clone(),
                format!(
                    "Resets {} if it no longer fits: {} tightened",
                    n.name,
                    tight.join(", ")
                ),
                true,
            ));
        }
        if !loose.is_empty() {
            out.push(Change::new(
                "W_SETTINGS_VALIDATOR_LOOSENED",
                path,
                format!("{} accepts more: {} loosened", n.name, loose.join(", ")),
                false,
            ));
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------ the form

/// The controls a settings field may be filled with; the first is the one a derived form uses (a `multiline`
/// string: `textarea`). A list of strings is a `textarea`, one item per line; a list of members a `member` input
/// that takes several.
pub fn controls(k: &Kind, repeated: bool) -> &'static [Control] {
    match (k, repeated) {
        (Kind::String, false) => &[Control::Text, Control::Textarea],
        (Kind::String, true) => &[Control::Textarea],
        (Kind::Secret, _) => &[Control::Secret],
        (Kind::Bool, _) => &[Control::Switch, Control::Checkbox],
        (Kind::Int32 | Kind::Int64 | Kind::Double | Kind::Decimal, _) => &[Control::Number],
        (Kind::Date, _) => &[Control::Date],
        (Kind::Enum(_), false) => &[Control::Select, Control::RadioCards],
        (Kind::Enum(_), true) => &[Control::MultiSelect],
        (Kind::User, _) => &[Control::Member],
    }
}

fn enum_options(f: &Field, enums: &[Enum]) -> Vec<Opt> {
    let Some(FieldType::Enum(name)) = FieldType::parse(&f.ty) else {
        return vec![];
    };
    enums
        .iter()
        .find(|e| e.name == name)
        .map(|e| {
            e.values
                .iter()
                .map(|v| Opt {
                    value: Value::String(v.clone()),
                    label: Text::Plain(humanize(v)),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn kind_in(f: &Field, enums: &[Enum]) -> Option<Kind> {
    let d = Descriptor {
        enums: enums.to_vec(),
        ..empty_descriptor()
    };
    kind(&d, f)
}

fn empty_descriptor() -> Descriptor {
    Descriptor {
        schema: crate::SCHEMA,
        id: String::new(),
        version: String::new(),
        title: String::new(),
        description: String::new(),
        roles: vec![],
        scopes: vec![],
        capabilities: vec![],
        permissions: vec![],
        agents: vec![],
        connections: vec![],
        dependencies: vec![],
        enums: vec![],
        messages: vec![],
        entities: vec![],
        endpoints: vec![],
        topics: vec![],
        ui: None,
        notifications: vec![],
        runs: None,
        jobs: vec![],
        views: vec![],
        layers: vec![],
        lake_tables: vec![],
        lake_models: vec![],
        loads: vec![],
        settings: None,
    }
}

fn blank_input(name: &str, control: Control, label: String) -> Input {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "control": control,
        "label": label,
    }))
    .expect("an input")
}

/// The form an SDK writes for a settings message the plugin lays out no form for: its fields in declaration order,
/// in sections by their `section` (in the order each first appears; the fields without one in an untitled section,
/// where the first of them appears). Each input: `name` the field's, `control` the first of [`controls`] (`textarea`
/// for a `multiline` string), `label` the field's `label` or its name humanized, `help` its description (when not
/// empty), `placeholder` its own, `required` [`is_required`], and an enum's `options` (each value with its name
/// humanized as the label). A field of a type settings do not take is left out (`check` names it).
pub fn derive_form(fields: &[Field], enums: &[Enum]) -> SettingsForm {
    let mut sections: Vec<FormSection> = vec![];
    let mut keys: Vec<Option<String>> = vec![];
    for f in fields {
        let Some(k) = kind_in(f, enums) else { continue };
        let control = if f.multiline && k == Kind::String && !f.repeated {
            Control::Textarea
        } else {
            controls(&k, f.repeated)[0]
        };
        let mut input = blank_input(
            &f.name,
            control,
            f.label.clone().unwrap_or_else(|| humanize(&f.name)),
        );
        fill_input(&mut input, f, enums);
        let at = match keys.iter().position(|k| *k == f.section) {
            Some(i) => i,
            None => {
                keys.push(f.section.clone());
                sections.push(FormSection {
                    title: f.section.clone().map(Text::Plain),
                    inputs: vec![],
                });
                sections.len() - 1
            }
        };
        sections[at].inputs.push(input);
    }
    SettingsForm { sections }
}

/// What every input of a settings form carries from its field, whether derived or laid out by the plugin:
/// `required` ([`is_required`]), and, when the input has none of its own, `help` (the description), `placeholder` and
/// an enum's `options`.
fn fill_input(input: &mut Input, f: &Field, enums: &[Enum]) {
    input.required = is_required(f);
    if input.help.is_none() && !f.description.is_empty() {
        input.help = Some(Text::Plain(f.description.clone()));
    }
    if input.placeholder.is_none() {
        input.placeholder = f.placeholder.clone().map(Text::Plain);
    }
    if input.options.is_empty()
        && matches!(
            input.control,
            Control::Select | Control::RadioCards | Control::MultiSelect
        )
    {
        input.options = enum_options(f, enums);
    }
}

/// A form the plugin laid out, its inputs filled from their fields as [`derive_form`] fills them (`required`, and
/// `help`, `placeholder`, an enum's `options` where the input has none). Inputs that name no field stay as they are.
pub fn complete_form(mut form: SettingsForm, fields: &[Field], enums: &[Enum]) -> SettingsForm {
    for s in &mut form.sections {
        for i in &mut s.inputs {
            if let Some(f) = fields.iter().find(|f| f.name == i.name) {
                fill_input(i, f, enums);
            }
        }
    }
    form
}

// ------------------------------------------------------------------------------------------------ check

/// The definition-time rules of settings (run by [`crate::validate`]).
pub fn validate(d: &Descriptor) -> Vec<Error> {
    let mut out = vec![];
    let settings_name = d.settings.as_ref().map(|s| s.message.as_str());
    // the options of settings fields, and the secret type, belong to the settings message only
    for m in &d.messages {
        if Some(m.name.as_str()) == settings_name {
            continue;
        }
        for f in &m.fields {
            scope_errors(
                &format!("messages[{}].fields[{}]", m.name, f.name),
                f,
                &mut out,
            );
        }
    }
    for e in &d.entities {
        for f in &e.fields {
            scope_errors(
                &format!("entities[{}].fields[{}]", e.name, f.name),
                f,
                &mut out,
            );
        }
    }
    let Some(s) = &d.settings else {
        return out;
    };
    let Some(m) = d.messages.iter().find(|m| m.name == s.message) else {
        out.push(
            Error::new(
                "E_SETTINGS_MESSAGE",
                "settings.message",
                format!("message {:?} is not declared", s.message),
            )
            .help("name the message of kind settings that holds the plugin's settings"),
        );
        return out;
    };
    if m.kind != MessageKind::Settings {
        out.push(
            Error::new(
                "E_SETTINGS_MESSAGE",
                "settings.message",
                format!("{} is not a message of kind settings", m.name),
            )
            .help("declare it with #[message(settings)] (kind: settings)"),
        );
    }
    let base = format!("messages[{}]", m.name);
    if m.fields.len() > MAX_SETTINGS_FIELDS {
        out.push(Error::new(
            "E_SETTINGS_LIMIT",
            &base,
            format!(
                "{} fields; settings have at most {MAX_SETTINGS_FIELDS}",
                m.fields.len()
            ),
        ));
    }
    let secrets = m
        .fields
        .iter()
        .filter(|f| f.ty == crate::SECRET_TYPE)
        .count();
    if secrets > MAX_SECRETS {
        out.push(Error::new(
            "E_SETTINGS_LIMIT",
            &base,
            format!("{secrets} secrets; settings have at most {MAX_SECRETS}"),
        ));
    }
    for f in &m.fields {
        field_errors(d, f, &format!("{base}.fields[{}]", f.name), &mut out);
    }
    form_errors(d, m, &s.form, &mut out);
    out
}

fn scope_errors(path: &str, f: &Field, out: &mut Vec<Error>) {
    if f.ty == crate::SECRET_TYPE {
        out.push(
            Error::new(
                "E_SETTINGS_FIELD_TYPE",
                format!("{path}.type"),
                "secret is the type of a settings field only",
            )
            .help("keep the value in the plugin's settings, or use string with #[col(encrypted)] on an entity"),
        );
    }
    if has_settings_options(f) {
        out.push(
            Error::new(
                "E_FIELD_VALIDATOR_SCOPE",
                path,
                "label, default, section, placeholder, multiline, validators and message belong to the fields of the settings message",
            )
            .help("check an endpoint's input in its handler"),
        );
    }
}

fn field_errors(d: &Descriptor, f: &Field, path: &str, out: &mut Vec<Error>) {
    let Some(k) = kind(d, f) else {
        out.push(
            Error::new(
                "E_SETTINGS_FIELD_TYPE",
                format!("{path}.type"),
                format!(
                    "{} is {}: settings take string, bool, int32, int64, double, decimal, date, secret, an enum or a member, optional or not, and lists of strings, enums or members",
                    f.name,
                    crate::lock::wire_type(f)
                ),
            ),
        );
        return;
    };
    if let Some(p) = &f.pattern
        && let Err(why) = compile_pattern(p)
    {
        out.push(
            Error::new("E_SETTINGS_PATTERN", format!("{path}.pattern"), format!("{p:?}: {why}"))
                .help("use literals, classes, \\d, \\w, groups, alternation and repetition; anchor with ^ and $"),
        );
    }
    let mut bad = |what: &str, why: String| {
        out.push(Error::new(
            "E_SETTINGS_VALIDATOR",
            format!("{path}.{what}"),
            why,
        ))
    };
    let list = f.repeated;
    let stringy = matches!(k, Kind::String | Kind::Secret);
    if (f.min.is_some() || f.max.is_some()) && !(k.numeric() && !list) {
        bad(
            "min",
            format!(
                "min and max bound a number, a decimal or a date; {} is not one",
                f.name
            ),
        );
    } else if k.numeric() {
        for (what, b) in [("min", &f.min), ("max", &f.max)] {
            if let Some(b) = b
                && (!spelled(&k, b) || typed(&k, b).is_err())
            {
                bad(
                    what,
                    format!(
                        "{what} {b} is not a value of {}: {}",
                        f.ty,
                        type_message(&k)
                    ),
                );
            }
        }
        if let (Some(a), Some(b)) = (&f.min, &f.max)
            && cmp_value(&k, a, b) == Some(Ordering::Greater)
        {
            bad("min", format!("min {a} is above max {b}"));
        }
    }
    if (f.min_len.is_some() || f.max_len.is_some()) && !stringy {
        bad(
            "min_len",
            format!(
                "min_len and max_len bound a string or a secret; {} is not one",
                f.name
            ),
        );
    }
    if let (Some(a), Some(b)) = (f.min_len, f.max_len)
        && a > b
    {
        bad("min_len", format!("min_len {a} is above max_len {b}"));
    }
    if f.pattern.is_some() && k != Kind::String {
        bad(
            "pattern",
            format!("a pattern checks a string; {} is not one", f.name),
        );
    }
    match f.format.as_deref() {
        None => {}
        Some("url" | "email") if k == Kind::String => {}
        Some("url" | "email") => bad(
            "format",
            format!("url and email check a string; {} is not one", f.name),
        ),
        Some(other) => bad("format", format!("format {other:?} is not url or email")),
    }
    if (f.min_items.is_some() || f.max_items.is_some()) && !list {
        bad(
            "min_items",
            format!(
                "min_items and max_items bound a list; {} is not one",
                f.name
            ),
        );
    }
    if let (Some(a), Some(b)) = (f.min_items, f.max_items)
        && a > b
    {
        bad("min_items", format!("min_items {a} is above max_items {b}"));
    }
    if f.multiline && !(k == Kind::String && !list) {
        bad(
            "multiline",
            format!("multiline belongs to a string; {} is not one", f.name),
        );
    }
    if let Some(v) = &f.default {
        if k == Kind::Secret {
            out.push(Error::new(
                "E_SETTINGS_DEFAULT",
                format!("{path}.default"),
                "a secret has no default: an owner or an admin sets it",
            ));
        } else if v.to_string().len() > MAX_VALUE_BYTES {
            out.push(Error::new(
                "E_SETTINGS_LIMIT",
                format!("{path}.default"),
                "the default is larger than 64 KiB",
            ));
        } else if !(if f.repeated {
            v.as_array()
                .is_some_and(|a| a.iter().all(|x| spelled(&k, x)))
        } else {
            spelled(&k, v)
        }) {
            out.push(Error::new(
                "E_SETTINGS_DEFAULT",
                format!("{path}.default"),
                format!(
                    "the default {v} is not written as a {} value is (numbers as numbers; decimals, dates, enum values and members as strings)",
                    f.ty
                ),
            ));
        } else if let Err(why) = check_value(d, f, v) {
            out.push(Error::new(
                "E_SETTINGS_DEFAULT",
                format!("{path}.default"),
                format!("the default {v} is refused: {why}"),
            ));
        }
    }
}

fn form_errors(d: &Descriptor, m: &Message, form: &SettingsForm, out: &mut Vec<Error>) {
    let mut seen: Vec<&str> = vec![];
    for (si, s) in form.sections.iter().enumerate() {
        for (ii, i) in s.inputs.iter().enumerate() {
            let p = format!("settings.form.sections[{si}].inputs[{ii}]");
            let Some(f) = m.fields.iter().find(|f| f.name == i.name) else {
                out.push(Error::new(
                    "E_SETTINGS_FORM_FIELD",
                    &p,
                    format!("{} has no field {}", m.name, i.name),
                ));
                continue;
            };
            if seen.contains(&i.name.as_str()) {
                out.push(Error::new(
                    "E_SETTINGS_FORM_FIELD",
                    &p,
                    format!("{} is in the form twice", i.name),
                ));
            }
            seen.push(&i.name);
            if i.options_from.is_some()
                || i.pick.is_some()
                || !i.fields.is_empty()
                || i.aside.is_some()
                || i.initial.is_some()
            {
                out.push(Error::new(
                    "E_SETTINGS_CONTROL",
                    &p,
                    "a settings input takes no options_from, pick, fields, aside or initial (a field's default is its initial value)",
                ));
            }
            let Some(k) = kind(d, f) else { continue };
            let allowed = controls(&k, f.repeated);
            if !allowed.contains(&i.control) {
                out.push(Error::new(
                    "E_SETTINGS_CONTROL",
                    format!("{p}.control"),
                    format!(
                        "{} ({}) is filled with {}, not {}",
                        f.name,
                        crate::lock::wire_type(f),
                        allowed
                            .iter()
                            .map(control_name)
                            .collect::<Vec<_>>()
                            .join(" or "),
                        control_name(&i.control)
                    ),
                ));
            }
            if let Kind::Enum(values) = &k {
                for o in &i.options {
                    if !o
                        .value
                        .as_str()
                        .is_some_and(|v| values.iter().any(|x| x == v))
                    {
                        out.push(Error::new(
                            "E_SETTINGS_CONTROL",
                            format!("{p}.options"),
                            format!("option {} is not a value of {}", o.value, f.ty),
                        ));
                    }
                }
            } else if !i.options.is_empty() {
                out.push(Error::new(
                    "E_SETTINGS_CONTROL",
                    format!("{p}.options"),
                    format!("{} is not an enum: its input takes no options", f.name),
                ));
            }
        }
    }
    for f in &m.fields {
        if !seen.contains(&f.name.as_str()) {
            out.push(
                Error::new(
                    "E_SETTINGS_FORM_FIELD",
                    "settings.form",
                    format!("{} is not in the form", f.name),
                )
                .help("a settings form has every field of the settings message once"),
            );
        }
    }
}

fn control_name(c: &Control) -> String {
    serde_json::to_value(c)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Warnings of settings: `W_SETTINGS_REQUIRED_NO_DEFAULT` for every required field without a default (a workspace
/// that installs the plugin must set it before the plugin works).
pub fn warnings(d: &Descriptor) -> Vec<Error> {
    let Some(m) = settings_message(d) else {
        return vec![];
    };
    m.fields
        .iter()
        .filter(|f| is_required(f))
        .map(|f| {
            Error::new(
                "W_SETTINGS_REQUIRED_NO_DEFAULT",
                format!("messages[{}].fields[{}]", m.name, f.name),
                format!(
                    "{} is required and has no default: the plugin needs setup before it works",
                    f.name
                ),
            )
            .help("give it a default, or make it optional")
        })
        .collect()
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
