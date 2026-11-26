//! `contract/<id>.proto`: the wire contract as a reviewer reads it, written by `celobus plugin build` from the
//! descriptor and the lock. A review view: never edited, never read by tools.

use std::fmt::Write as _;

use crate::lock::Lock;
use crate::{Descriptor, Field, FieldType, method_path, service_name};

fn upper_snake(s: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_ascii_uppercase() && prev_lower {
            out.push('_');
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        out.push(c.to_ascii_uppercase());
    }
    out
}

/// The proto type of a field and a note for what proto cannot say.
fn proto_type(f: &Field) -> (String, Option<String>) {
    match FieldType::parse(&f.ty) {
        Some(FieldType::Scalar(s)) => match s {
            "string" | "bool" | "int32" | "int64" | "double" | "bytes" => (s.to_string(), None),
            "decimal" => ("string".into(), Some("decimal".into())),
            "secret" => ("string".into(), Some("secret".into())),
            "uuid" => ("string".into(), Some("uuid".into())),
            "date" => ("string".into(), Some("date, YYYY-MM-DD".into())),
            "timestamp" => ("google.protobuf.Timestamp".into(), None),
            "json" => ("google.protobuf.Value".into(), None),
            "file" => (
                "celobus.v1.File".into(),
                Some("file: id, name, type, size".into()),
            ),
            _ => (s.to_string(), None),
        },
        Some(FieldType::Message(m)) => (m, None),
        Some(FieldType::Enum(e)) => (e, None),
        Some(FieldType::Ref { plugin, entity }) => {
            ("string".into(), Some(format!("ref {plugin}.{entity}")))
        }
        None => (f.ty.clone(), None),
    }
}

fn comment(out: &mut String, indent: &str, text: &str) {
    for l in text.lines() {
        let _ = writeln!(out, "{indent}// {l}");
    }
}

fn reserved(out: &mut String, numbers: &[u32]) {
    if !numbers.is_empty() {
        let list: Vec<String> = numbers.iter().map(u32::to_string).collect();
        let _ = writeln!(out, "  reserved {};", list.join(", "));
    }
}

/// The proto file of a descriptor; field numbers are the descriptor's (the locked ones after
/// [`crate::lock::apply_numbers`]), reserved numbers come from the lock.
pub fn to_proto(d: &Descriptor, lock: Option<&Lock>) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// contract/{}.proto: the wire contract of {} {}, written by `celobus plugin build`.",
        d.id, d.id, d.version
    );
    out.push_str(
        "// A review view: never edited and never read by tools. Field numbers come from celobus.lock.\n\
         // The wire is the protobuf JSON mapping of these messages (lowerCamelCase names, int64 and\n\
         // decimal as strings, enums by value name).\n\
         // Readers ignore unknown fields and accept unknown enum values.\n",
    );
    out.push_str("syntax = \"proto3\";\n\n");
    let _ = writeln!(out, "package {}.v1;\n", d.id);
    let all_fields = d.messages.iter().flat_map(|m| m.fields.iter());
    let (mut ts, mut json) = (false, false);
    for f in all_fields {
        ts |= f.ty == "timestamp";
        json |= f.ty == "json";
    }
    if ts {
        out.push_str("import \"google/protobuf/timestamp.proto\";\n");
    }
    if json {
        out.push_str("import \"google/protobuf/struct.proto\";\n");
    }
    if ts || json {
        out.push('\n');
    }

    if !d.endpoints.is_empty() {
        if !d.description.is_empty() {
            comment(&mut out, "", &d.description);
        }
        let _ = writeln!(out, "service {} {{", service_name(&d.id));
        for e in &d.endpoints {
            if !e.description.is_empty() {
                comment(&mut out, "  ", &e.description);
            }
            let scope = e
                .scopes
                .first()
                .map(String::as_str)
                .unwrap_or(crate::DEFAULT_SCOPE);
            let roles = if e.roles.is_empty() {
                "any member".to_string()
            } else {
                format!("role {}", e.roles.join(" or "))
            };
            let _ = writeln!(
                out,
                "  rpc {}({}) returns ({});   // {} · scope {scope} · {roles}",
                e.name,
                e.input,
                e.output,
                method_path(&d.id, &e.name)
            );
        }
        out.push_str("}\n");
    }

    for e in &d.enums {
        out.push('\n');
        let prefix = upper_snake(&e.name);
        let _ = writeln!(out, "enum {} {{", e.name);
        let _ = writeln!(out, "  {prefix}_UNSPECIFIED = 0;");
        let locked = lock.and_then(|l| l.enums.get(&e.name));
        for (i, v) in e.values.iter().enumerate() {
            let n = locked
                .and_then(|l| l.values.get(v).copied())
                .unwrap_or(i as u32 + 1);
            let _ = writeln!(
                out,
                "  {prefix}_{} = {n};   // \"{v}\"",
                v.to_ascii_uppercase()
            );
        }
        reserved(
            &mut out,
            locked.map(|l| l.reserved.as_slice()).unwrap_or(&[]),
        );
        out.push_str("}\n");
    }

    for m in &d.messages {
        out.push('\n');
        if !m.description.is_empty() {
            comment(&mut out, "", &m.description);
        }
        if let Some(e) = d.entities.iter().find(|e| e.message == m.name) {
            let _ = writeln!(out, "// entity {}.{} (key {})", d.id, e.name, e.key);
        }
        let _ = writeln!(out, "message {} {{", m.name);
        for f in &m.fields {
            let (ty, note) = proto_type(f);
            let label = if f.repeated {
                "repeated "
            } else if f.optional {
                "optional "
            } else {
                ""
            };
            let mut line = format!("  {label}{ty} {} = {};", f.name, f.number);
            let mut notes = vec![];
            notes.extend(note);
            if !f.description.is_empty() {
                notes.push(f.description.lines().next().unwrap_or_default().to_string());
            }
            if !notes.is_empty() {
                line.push_str(&format!("   // {}", notes.join(" · ")));
            }
            out.push_str(&line);
            out.push('\n');
        }
        reserved(
            &mut out,
            lock.and_then(|l| l.messages.get(&m.name))
                .map(|l| l.reserved.as_slice())
                .unwrap_or(&[]),
        );
        out.push_str("}\n");
    }
    out
}
