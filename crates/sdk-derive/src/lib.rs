//! Derive macros of the celobus plugin SDK (`celobus-sdk`); use them through `celobus_sdk::prelude`.
//!
//! - `#[derive(Message)]` on a struct with named fields: a protobuf message of the plugin (its
//!   descriptor entry and its JSON codec); on an enum without fields: a protobuf enum.
//! - `#[derive(View)]` on a struct with named fields: the row of an analytic view (`#[view(name = "sales")]`).
//! - `#[derive(LakeTable)]` on a struct with named fields: the row of a lake table (`#[lake(layer = RAW,
//!   name = "stock", partition(day), key(day, sku))]`).
//! - `#[derive(Entity)]` on a struct with named fields and an `id: Id<Self>` field: an entity
//!   (`#[entity(key = uuid | i64 | string, soft_delete, version, append_only, label = field,
//!   index(a, b desc), unique_index(a, b), history(layer = AUDIT, all, unmasked))]`, `#[col(unique, index,
//!   encrypted, searchable, searchable(lang = "english"), nav = name, inverse = name, on_delete = cascade,
//!   history)]` on its fields).
//! - `#[field(max_size = "1 GiB", accept = ["text/csv", "image/*"])]` on a `File`, `Option<File>` or `Vec<File>` field
//!   of a message or an entity: the largest file it takes and the MIME types (or patterns) it accepts.
//! - `#[field(label = "..", default = 2, min = 1, max = 48, section = "..", url, ..)]` on a field of a
//!   `#[message(settings)]` struct: its form and validators (see `Message`).
//!
//! The generated code refers to `::celobus_sdk::__private`, so the plugin depends on `celobus-sdk`
//! (which re-exports these macros) and never on this crate directly.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Fields, Ident, LitStr, parse_macro_input, spanned::Spanned,
};

/// A message (struct with named fields) or an enum (fieldless variants) of the plugin.
///
/// Struct options: `#[message(event | job | settings)]` (the kind; default `message`),
/// `#[message(response = Type)]` (implements `celobus_sdk::Call`, so `ctx.call(plugin, &req)` is
/// typed by its response) and `#[message(omit_defaults)]` (fields at their protobuf default — `""`, `0`,
/// `false`, an empty list, a message whose JSON is `{}` — are left out of the JSON; an `Option` that is `Some` and
/// an enum are always written). The first paragraph of a doc comment is the description.
///
/// Field options: `#[field(max_size = "1 GiB", accept = ["text/csv", "image/*"])]` on a `File` field (or
/// `Option<File>`, `Vec<File>`): the largest file it takes, in bytes or with a unit (`KiB`, `MiB`, `GiB`, `TiB`
/// binary; `KB`, `MB`, `GB`, `TB` decimal), and the MIME types or patterns it accepts.
///
/// On a field of a `#[message(settings)]` struct: `#[field(label = "Thread stays with its agent, hours", default = 2,
/// min = 1, max = 48, section = "Agents")]` — the form's `label`, `section` and `placeholder`, `multiline` (a text
/// area), a `default` (a number, a string — decimals, dates, an enum's value name —, `true`/`false`, or a list), and
/// validators: `min`, `max` (numbers, decimals and dates as strings), `min_len`, `max_len` (characters), `pattern`
/// (a regular expression), `url`, `email`, `min_items`, `max_items`, and `message` (its own text when one of them
/// refuses a value). The doc comment is the field's help.
#[proc_macro_derive(Message, attributes(message, field))]
pub fn derive_message(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let out = match &input.data {
        Data::Struct(_) => message_struct(&input, None),
        Data::Enum(_) => message_enum(&input),
        Data::Union(_) => Err(syn::Error::new(
            input.ident.span(),
            "#[derive(Message)] supports structs with named fields and fieldless enums",
        )),
    };
    out.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// An entity of the plugin: a struct with named fields, one of them `id: Id<Self>`.
///
/// `#[entity(key = uuid | i64 | string, soft_delete, version, append_only, label = field)]`; the key
/// defaults to `uuid`, the label to the first of `name`, `title`, `number` the struct has, else `id`.
/// `#[entity(plugin = "customers")]` declares another plugin's entity (for `Id<E>` refs to it): it is
/// not part of this plugin's descriptor. `#[entity(index(created_by, created_at desc))]` and
/// `#[entity(unique_index(owner_id, title))]` declare an index over several columns (fields or system columns);
/// `#[col(unique)]` and `#[col(index)]` on a field, one over that column; `#[col(encrypted)]` keeps the field's
/// values encrypted at rest with the workspace's key (never unique, indexed, searchable, filtered, sorted, a ref, the
/// id, the label or read by declared SQL); `#[col(searchable)]`
/// (`searchable(lang = "english")`) a string column searched by full text; `#[col(nav = customer, inverse = orders)]`
/// on a ref names its navigations (the forward one defaults to the field's name without `_id`; an inverse one is a
/// handle on the target, `Customer::orders`, so the target is an entity of the same crate);
/// `#[col(on_delete = cascade)]` what the ref does when its target is deleted for good (`restrict` by default,
/// `cascade`, `set_null`, `no_action`; a ref to another plugin or the directory `keep` by default, or `set_null`
/// onto the directory). `#[entity(history(layer = AUDIT))]` keeps the entity's history in the lake's layer `AUDIT` (a
/// `Layer` constant): the changes of the columns marked `#[col(history)]`, or with `all` of every column but the
/// encrypted ones and, unless `unmasked` is said too, the masked ones.
#[proc_macro_derive(Entity, attributes(entity, col, field))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let out = match &input.data {
        Data::Struct(_) => {
            entity_options(&input).and_then(|opts| message_struct(&input, Some(opts)))
        }
        _ => Err(syn::Error::new(
            input.ident.span(),
            "#[derive(Entity)] supports structs with named fields",
        )),
    };
    out.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// An analytic view's row (a struct with named fields): a message of the plugin, and a source of queries
/// (`ctx.query::<V>()`, `ctx.analytics().query::<V>()`) once the plugin declares the view (`Plugin::view`).
///
/// `#[view(name = "sales")]`; the name defaults to the type's in snake_case. The first paragraph of a doc comment is
/// the view's description (an agent of the plugin reads it to pick the view).
#[proc_macro_derive(View, attributes(view))]
pub fn derive_view(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let out = match &input.data {
        Data::Struct(_) => view_name(&input).and_then(|name| {
            let ty = &input.ident;
            let message = message_struct(&input, None)?;
            Ok(quote! {
                #message
                const _: () = {
                    use ::celobus_sdk::__private as __p;
                    impl __p::Queryable for #ty {
                        const SOURCE: &'static str = #name;
                        const OWNER: ::std::option::Option<&'static str> = ::std::option::Option::None;
                    }
                    impl __p::QueryRow for #ty {
                        type Row = Self;
                        fn row(value: &__p::Value) -> __p::Result<Self> {
                            <Self as __p::Message>::from_json(value, "")
                        }
                    }
                };
            })
        }),
        _ => Err(syn::Error::new(
            input.ident.span(),
            "#[derive(View)] supports structs with named fields",
        )),
    };
    out.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// A lake table's row (a struct with named fields): a message of the plugin, a table of the workspace's Data Lake once
/// the plugin declares it (`Plugin::lake_table`), written with `ctx.lake().table::<T>()` and read with
/// `ctx.lake().query::<T>()`.
///
/// `#[lake(layer = RAW, name = "stock_file", partition(loaded_on), key(day, sku))]`: the layer (a `Layer` constant),
/// the table's name (default: the type's in snake_case), its partition columns (what `replace` swaps) and its key
/// columns (what `merge` writes over); each a field of the struct. `#[lake(plugin = "retail", name = "stock")]`
/// declares another plugin's exported table this plugin reads (`Plugin::lake_dependency`): not part of this plugin's
/// tables. The first paragraph of a doc comment is the table's description.
#[proc_macro_derive(LakeTable, attributes(lake))]
pub fn derive_lake_table(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let out = match &input.data {
        Data::Struct(data) => lake_options(&input).and_then(|o| {
            let ty = &input.ident;
            let Fields::Named(named) = &data.fields else {
                return Err(syn::Error::new(ty.span(), "a lake table has named fields"));
            };
            let fields: Vec<String> = named
                .named
                .iter()
                .filter_map(|f| f.ident.as_ref().map(unraw))
                .collect();
            for (what, list) in [("partition", &o.partition), ("key", &o.key)] {
                for c in list {
                    if !fields.contains(c) {
                        return Err(syn::Error::new(
                            ty.span(),
                            format!("{what} column `{c}` is not a field of {ty}"),
                        ));
                    }
                }
            }
            if o.plugin.is_none() && o.layer.is_none() {
                return Err(syn::Error::new(
                    ty.span(),
                    "a lake table names its layer: #[lake(layer = RAW)] (a `Layer` constant)",
                ));
            }
            let message = message_struct(&input, None)?;
            let name = o
                .name
                .clone()
                .unwrap_or_else(|| snake_case(&ty.to_string()));
            let layer = match &o.layer {
                Some(l) => quote! { #l },
                None => quote! { __p::Layer::new("") },
            };
            let owner = match &o.plugin {
                Some(p) => quote! { ::std::option::Option::Some(#p) },
                None => quote! { ::std::option::Option::None },
            };
            let (partition, key) = (&o.partition, &o.key);
            Ok(quote! {
                #message
                const _: () = {
                    use ::celobus_sdk::__private as __p;
                    impl __p::Queryable for #ty {
                        const SOURCE: &'static str = #name;
                        const OWNER: ::std::option::Option<&'static str> = #owner;
                    }
                    impl __p::QueryRow for #ty {
                        type Row = Self;
                        fn row(value: &__p::Value) -> __p::Result<Self> {
                            <Self as __p::Message>::from_json(value, "")
                        }
                    }
                    impl __p::LakeTable for #ty {
                        const LAYER: __p::Layer = #layer;
                        const TABLE: &'static str = #name;
                        const PARTITION: &'static [&'static str] = &[ #( #partition ),* ];
                        const KEY: &'static [&'static str] = &[ #( #key ),* ];
                    }
                };
            })
        }),
        _ => Err(syn::Error::new(
            input.ident.span(),
            "#[derive(LakeTable)] supports structs with named fields",
        )),
    };
    out.unwrap_or_else(syn::Error::into_compile_error).into()
}

struct LakeOptions {
    layer: Option<syn::Path>,
    name: Option<String>,
    plugin: Option<String>,
    partition: Vec<String>,
    key: Vec<String>,
}

/// `(a, b)`: field names.
fn field_list(input: syn::parse::ParseStream) -> syn::Result<Vec<String>> {
    let content;
    syn::parenthesized!(content in input);
    let mut out = Vec::new();
    while !content.is_empty() {
        let name: Ident = content.parse()?;
        out.push(unraw(&name));
        if !content.is_empty() {
            content.parse::<syn::Token![,]>()?;
        }
    }
    Ok(out)
}

fn lake_options(input: &DeriveInput) -> syn::Result<LakeOptions> {
    let mut o = LakeOptions {
        layer: None,
        name: None,
        plugin: None,
        partition: vec![],
        key: vec![],
    };
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("lake")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("layer") {
                o.layer = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("name") {
                let v: LitStr = meta.value()?.parse()?;
                o.name = Some(v.value());
            } else if meta.path.is_ident("plugin") {
                let v: LitStr = meta.value()?.parse()?;
                o.plugin = Some(v.value());
            } else if meta.path.is_ident("partition") {
                o.partition = field_list(meta.input)?;
            } else if meta.path.is_ident("key") {
                o.key = field_list(meta.input)?;
            } else {
                return Err(meta.error(
                    "unknown lake option (layer = .., name = \"..\", plugin = \"..\", partition(..), key(..))",
                ));
            }
            Ok(())
        })?;
    }
    Ok(o)
}

/// `#[view(name = "sales")]`, or the type's name in snake_case.
fn view_name(input: &DeriveInput) -> syn::Result<String> {
    let mut name = snake_case(&input.ident.to_string());
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("view")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                let v: LitStr = meta.value()?.parse()?;
                name = v.value();
                Ok(())
            } else {
                Err(meta.error("unknown view option (name = \"..\")"))
            }
        })?;
    }
    Ok(name)
}

struct EntityOptions {
    key: &'static str,
    soft_delete: bool,
    version: bool,
    append_only: bool,
    label: Option<String>,
    plugin: Option<String>,
    /// (columns with `desc`, unique)
    indexes: Vec<(Vec<(String, bool)>, bool)>,
    /// `history(layer = .., all, unmasked)`: the layer, all, unmasked
    history: Option<(syn::Path, bool, bool)>,
    /// `storage = olap`: its rows are kept in the analytic copy only
    olap: bool,
    /// `key(a, b)`: the fields that key a row of an entity stored in OLAP
    olap_key: Vec<String>,
}

/// `(a, b desc, c asc)`: the columns of an index.
fn index_columns(input: syn::parse::ParseStream) -> syn::Result<Vec<(String, bool)>> {
    let content;
    syn::parenthesized!(content in input);
    let mut out = Vec::new();
    while !content.is_empty() {
        let name: Ident = content.parse()?;
        let mut desc = false;
        if content.peek(Ident) {
            let dir: Ident = content.parse()?;
            desc = match dir.to_string().as_str() {
                "desc" => true,
                "asc" => false,
                _ => {
                    return Err(syn::Error::new(
                        dir.span(),
                        "a column is followed by asc or desc",
                    ));
                }
            };
        }
        out.push((unraw(&name), desc));
        if !content.is_empty() {
            content.parse::<syn::Token![,]>()?;
        }
    }
    if out.is_empty() {
        return Err(input.error("an index has columns"));
    }
    Ok(out)
}

/// The column options of an entity's field: `#[col(unique, index, encrypted, searchable,
/// searchable(lang = "english"), nav = name, inverse = name, on_delete = cascade)]`.
#[derive(Default)]
struct ColumnOptions {
    unique: bool,
    index: bool,
    encrypted: bool,
    /// the language ("simple" for a bare `searchable`); empty: not searchable
    searchable: String,
    nav: Option<Ident>,
    inverse: Option<Ident>,
    /// `restrict`, `cascade`, `set_null`, `no_action` (a ref within the plugin), `keep`, `set_null` (to another
    /// plugin or the directory); checked by `celobus plugin check`
    on_delete: Option<Ident>,
    /// the entity's history keeps the column's changes
    history: bool,
}

impl ColumnOptions {
    fn any(&self) -> bool {
        self.unique
            || self.index
            || self.encrypted
            || !self.searchable.is_empty()
            || self.nav.is_some()
            || self.inverse.is_some()
            || self.on_delete.is_some()
            || self.history
    }
}

fn column_options(attrs: &[Attribute]) -> syn::Result<ColumnOptions> {
    let mut o = ColumnOptions::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("col")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("unique") {
                o.unique = true;
            } else if meta.path.is_ident("index") {
                o.index = true;
            } else if meta.path.is_ident("encrypted") {
                o.encrypted = true;
            } else if meta.path.is_ident("searchable") {
                o.searchable = "simple".into();
                if meta.input.peek(syn::token::Paren) {
                    meta.parse_nested_meta(|inner| {
                        if inner.path.is_ident("lang") {
                            let lang: LitStr = inner.value()?.parse()?;
                            o.searchable = lang.value();
                            Ok(())
                        } else {
                            Err(inner.error("searchable takes lang = \"<language>\""))
                        }
                    })?;
                }
            } else if meta.path.is_ident("history") {
                o.history = true;
            } else if meta.path.is_ident("nav") {
                o.nav = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("inverse") {
                o.inverse = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("on_delete") {
                let v: Ident = meta.value()?.parse()?;
                const VALUES: &[&str] = &["restrict", "cascade", "set_null", "no_action", "keep"];
                if !VALUES.contains(&v.to_string().as_str()) {
                    return Err(syn::Error::new(
                        v.span(),
                        "on_delete is one of restrict, cascade, set_null, no_action (a ref within the plugin), keep, set_null (to another plugin or the directory)",
                    ));
                }
                o.on_delete = Some(v);
            } else {
                return Err(meta.error(
                    "unknown column option (unique, index, encrypted, searchable, searchable(lang = \"..\"), nav = name, inverse = name, on_delete = .., history)",
                ));
            }
            Ok(())
        })?;
    }
    Ok(o)
}

/// The options of a field: a file field's `#[field(max_size = "1 GiB", accept = ["text/csv"])]`, and a settings
/// field's `label`, `default`, `section`, `placeholder`, `multiline` and validators (each kept as its descriptor key and
/// its value as JSON text).
#[derive(Default)]
struct FileOptions {
    max_size: Option<u64>,
    accept: Vec<String>,
    settings: Vec<(String, String)>,
}

/// The settings options of `#[field(..)]`: what each takes.
const SETTINGS_OPTIONS: &str = "label = \"..\", default = <value>, section = \"..\", placeholder = \"..\", multiline, \
     min = <number or \"..\">, max = .., min_len = n, max_len = n, pattern = \"..\", url, email, min_items = n, \
     max_items = n, message = \"..\"";

/// A JSON string literal.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A literal as JSON: a string, an integer or a float (optionally negative), `true`/`false`, or a list of them.
fn json_literal(v: syn::parse::ParseStream) -> syn::Result<String> {
    if v.peek(syn::token::Bracket) {
        let content;
        syn::bracketed!(content in v);
        let mut items = vec![];
        while !content.is_empty() {
            items.push(json_literal(&content)?);
            if content.is_empty() {
                break;
            }
            content.parse::<syn::Token![,]>()?;
        }
        return Ok(format!("[{}]", items.join(",")));
    }
    let neg = if v.peek(syn::Token![-]) {
        v.parse::<syn::Token![-]>()?;
        "-"
    } else {
        ""
    };
    let lit: syn::Lit = v.parse()?;
    let text = match &lit {
        syn::Lit::Str(s) if neg.is_empty() => json_string(&s.value()),
        syn::Lit::Int(i) => {
            let digits = i.base10_digits();
            digits
                .parse::<i128>()
                .map_err(|_| syn::Error::new(i.span(), "not an integer"))?;
            format!("{neg}{digits}")
        }
        syn::Lit::Float(f) => {
            let digits = f.base10_digits();
            let digits = if digits.ends_with('.') {
                format!("{digits}0")
            } else {
                digits.to_string()
            };
            format!("{neg}{digits}")
        }
        syn::Lit::Bool(b) if neg.is_empty() => b.value.to_string(),
        _ => {
            return Err(syn::Error::new(
                lit.span(),
                "a value is a string, a number, true or false, or a list of them",
            ));
        }
    };
    Ok(text)
}

fn count_value(meta: &syn::meta::ParseNestedMeta) -> syn::Result<String> {
    let n: syn::LitInt = meta.value()?.parse()?;
    n.base10_parse::<u32>()
        .map(|n| n.to_string())
        .map_err(|_| syn::Error::new(n.span(), "a count from 0 to 4294967295"))
}

fn file_options(attrs: &[Attribute]) -> syn::Result<FileOptions> {
    let mut o = FileOptions::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("field")) {
        attr.parse_nested_meta(|meta| {
            let key = meta
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            let text = |meta: &syn::meta::ParseNestedMeta| -> syn::Result<String> {
                let s: LitStr = meta.value()?.parse()?;
                Ok(json_string(&s.value()))
            };
            let setting = match key.as_str() {
                "label" | "section" | "placeholder" | "pattern" | "message" => {
                    Some((key.clone(), text(&meta)?))
                }
                "default" | "min" | "max" => Some((key.clone(), json_literal(meta.value()?)?)),
                "min_len" | "max_len" | "min_items" | "max_items" => {
                    Some((key.clone(), count_value(&meta)?))
                }
                "multiline" => Some((key.clone(), "true".to_string())),
                "url" | "email" => Some(("format".to_string(), json_string(&key))),
                _ => None,
            };
            if let Some((k, v)) = setting {
                if o.settings.iter().any(|(x, _)| *x == k) {
                    return Err(meta.error(format!("{k} is given twice (url and email are one format)")));
                }
                o.settings.push((k, v));
                return Ok(());
            }
            if meta.path.is_ident("max_size") {
                let v = meta.value()?;
                let size = if v.peek(syn::LitInt) {
                    let n: syn::LitInt = v.parse()?;
                    n.base10_parse::<u64>().ok()
                } else {
                    let s: LitStr = v.parse()?;
                    parse_size(&s.value())
                };
                match size {
                    Some(n) if n > 0 => o.max_size = Some(n),
                    _ => {
                        return Err(meta.error(
                            "max_size is a size: bytes (1048576) or a number and a unit (\"1 GiB\", \"500 MB\"; KiB, MiB, GiB, TiB, KB, MB, GB, TB)",
                        ));
                    }
                }
            } else if meta.path.is_ident("accept") {
                let v = meta.value()?;
                if v.peek(LitStr) {
                    let s: LitStr = v.parse()?;
                    o.accept.push(s.value());
                } else {
                    let content;
                    syn::bracketed!(content in v);
                    let items =
                        syn::punctuated::Punctuated::<LitStr, syn::Token![,]>::parse_terminated(&content)?;
                    o.accept.extend(items.iter().map(LitStr::value));
                }
            } else {
                return Err(meta.error(format!(
                    "unknown field option (a file: max_size = \"1 GiB\", accept = [\"text/csv\"]; a setting: {SETTINGS_OPTIONS})"
                )));
            }
            Ok(())
        })?;
    }
    Ok(o)
}

/// A size: bytes, or a number and a unit (binary `KiB`..`TiB`, decimal `B`, `KB`..`TB`, case ignored); the same
/// grammar as `celobus_descriptor::parse_size`.
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = (&s[..split], s[split..].trim());
    if num.is_empty() || num.starts_with('.') || num.ends_with('.') || num.matches('.').count() > 1
    {
        return None;
    }
    let mult: u64 = match unit.to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "kb" => 1_000,
        "mb" => 1_000_000,
        "gb" => 1_000_000_000,
        "tb" => 1_000_000_000_000,
        "kib" => 1 << 10,
        "mib" => 1 << 20,
        "gib" => 1 << 30,
        "tib" => 1 << 40,
        _ => return None,
    };
    let (whole, frac) = num.split_once('.').unwrap_or((num, ""));
    let mut bytes = whole.parse::<u64>().ok()?.checked_mul(mult)?;
    if !frac.is_empty() {
        let digits = frac.len().min(18);
        let f: u64 = frac[..digits].parse().ok()?;
        let part = (u128::from(f) * u128::from(mult)) / 10u128.pow(digits as u32);
        bytes = bytes.checked_add(u64::try_from(part).ok()?)?;
    }
    Some(bytes)
}

/// The entity a ref field names: `T` of `Id<T>` or `Option<Id<T>>`.
fn ref_target(ty: &syn::Type) -> Option<syn::Type> {
    let syn::Type::Path(p) = ty else {
        return None;
    };
    let last = p.path.segments.last()?;
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    let Some(syn::GenericArgument::Type(inner)) = args.args.first() else {
        return None;
    };
    match last.ident.to_string().as_str() {
        "Id" => Some(inner.clone()),
        "Option" => match inner {
            syn::Type::Path(ip) if ip.path.segments.last().is_some_and(|s| s.ident == "Id") => {
                ref_target(inner)
            }
            _ => None,
        },
        _ => None,
    }
}

fn entity_options(input: &DeriveInput) -> syn::Result<EntityOptions> {
    let mut opts = EntityOptions {
        key: "uuid",
        soft_delete: false,
        version: false,
        append_only: false,
        label: None,
        plugin: None,
        indexes: Vec::new(),
        history: None,
        olap: false,
        olap_key: Vec::new(),
    };
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("entity")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") && meta.input.peek(syn::token::Paren) {
                // `key(day, store, metric)`: an entity stored in OLAP keyed by its fields
                opts.olap_key = field_list(meta.input)?;
                if opts.olap_key.is_empty() {
                    return Err(meta.error("key(..) names the fields that key a row"));
                }
            } else if meta.path.is_ident("storage") {
                let storage: Ident = meta.value()?.parse()?;
                if storage != "olap" {
                    return Err(syn::Error::new(
                        storage.span(),
                        "storage = olap keeps the entity's rows in the analytic copy only (the default keeps them in the workspace's database)",
                    ));
                }
                opts.olap = true;
            } else if meta.path.is_ident("key") {
                let key: Ident = meta.value()?.parse()?;
                opts.key = match key.to_string().as_str() {
                    "uuid" => "uuid",
                    "i64" => "i64",
                    "string" | "String" => "string",
                    _ => return Err(syn::Error::new(key.span(), "key is one of uuid, i64, string")),
                };
            } else if meta.path.is_ident("soft_delete") {
                opts.soft_delete = true;
            } else if meta.path.is_ident("version") {
                opts.version = true;
            } else if meta.path.is_ident("append_only") {
                opts.append_only = true;
            } else if meta.path.is_ident("label") {
                let label: Ident = meta.value()?.parse()?;
                opts.label = Some(unraw(&label));
            } else if meta.path.is_ident("plugin") {
                let plugin: LitStr = meta.value()?.parse()?;
                opts.plugin = Some(plugin.value());
            } else if meta.path.is_ident("index") {
                opts.indexes.push((index_columns(meta.input)?, false));
            } else if meta.path.is_ident("unique_index") {
                opts.indexes.push((index_columns(meta.input)?, true));
            } else if meta.path.is_ident("history") {
                let (mut layer, mut all, mut unmasked) = (None, false, false);
                meta.parse_nested_meta(|inner| {
                    if inner.path.is_ident("layer") {
                        layer = Some(inner.value()?.parse::<syn::Path>()?);
                    } else if inner.path.is_ident("all") {
                        all = true;
                    } else if inner.path.is_ident("unmasked") {
                        unmasked = true;
                    } else {
                        return Err(inner.error("history takes layer = <Layer>, all, unmasked"));
                    }
                    Ok(())
                })?;
                let layer = layer.ok_or_else(|| {
                    meta.error("history names its layer: history(layer = AUDIT) (a `Layer` constant)")
                })?;
                opts.history = Some((layer, all, unmasked));
            } else {
                return Err(meta.error(
                    "unknown entity option (known: key, soft_delete, version, append_only, label, plugin, index, unique_index, history, storage = olap, key(..))",
                ));
            }
            Ok(())
        })?;
    }
    Ok(opts)
}

struct MessageOptions {
    kind: &'static str,
    response: Option<syn::Type>,
    omit_defaults: bool,
}

fn message_options(attrs: &[Attribute]) -> syn::Result<MessageOptions> {
    let mut opts = MessageOptions {
        kind: "message",
        response: None,
        omit_defaults: false,
    };
    for attr in attrs.iter().filter(|a| a.path().is_ident("message")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("event") {
                opts.kind = "event";
            } else if meta.path.is_ident("job") {
                opts.kind = "job";
            } else if meta.path.is_ident("settings") {
                opts.kind = "settings";
            } else if meta.path.is_ident("response") {
                opts.response = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("omit_defaults") {
                opts.omit_defaults = true;
            } else {
                return Err(meta.error(
                    "unknown message option (event, job, settings, response = Type, omit_defaults)",
                ));
            }
            Ok(())
        })?;
    }
    Ok(opts)
}

fn no_generics(input: &DeriveInput) -> syn::Result<()> {
    if input.generics.params.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new(
            input.generics.span(),
            "a message or an entity cannot be generic",
        ))
    }
}

fn message_struct(input: &DeriveInput, entity: Option<EntityOptions>) -> syn::Result<TokenStream2> {
    no_generics(input)?;
    let ty = &input.ident;
    let name = ty.to_string();
    let Data::Struct(data) = &input.data else {
        unreachable!()
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new(
            ty.span(),
            "a message or an entity has named fields",
        ));
    };
    let description = doc_paragraph(&input.attrs);

    let mut idents = Vec::new();
    let mut types = Vec::new();
    let mut names = Vec::new();
    let mut json_names = Vec::new();
    let mut docs = Vec::new();
    let mut columns: Vec<ColumnOptions> = Vec::new();
    let mut files: Vec<FileOptions> = Vec::new();
    for field in &named.named {
        let ident = field.ident.clone().expect("named field");
        let fname = unraw(&ident);
        json_names.push(json_name(&fname));
        names.push(fname);
        docs.push(doc_paragraph(&field.attrs));
        types.push(field.ty.clone());
        idents.push(ident);
        let col = column_options(&field.attrs)?;
        if col.any() && entity.is_none() {
            return Err(syn::Error::new(
                field.span(),
                "#[col] declares a column of an entity: use #[derive(Entity)]",
            ));
        }
        columns.push(col);
        files.push(file_options(&field.attrs)?);
    }

    // the options of a setting belong to the fields of a settings message
    let settings_kind = entity.is_none() && message_options(&input.attrs)?.kind == "settings";
    if !settings_kind
        && let Some((field, _)) = named
            .named
            .iter()
            .zip(&files)
            .find(|(_, f)| !f.settings.is_empty())
    {
        return Err(syn::Error::new(
            field.span(),
            "label, default, section, placeholder, multiline, validators and message are options of a setting: \
             declare the struct with #[message(settings)]",
        ));
    }
    // `#[message(omit_defaults)]`: fields at their protobuf default are left out of the JSON
    let omit = entity.is_none() && message_options(&input.attrs)?.omit_defaults;
    let encode = if omit {
        quote! { __p::encode_field_trimmed }
    } else {
        quote! { __p::encode_field }
    };
    let to_json = quote! {
        fn to_json(&self) -> __p::Value {
            let mut map = __p::Map::new();
            #( #encode(&mut map, #json_names, &self.#idents); )*
            __p::Value::Object(map)
        }
        fn from_json(value: &__p::Value, path: &str) -> __p::Result<Self> {
            let obj = __p::object(value, path)?;
            ::std::result::Result::Ok(Self {
                #( #idents: __p::decode_field::<#types>(obj, #json_names, #names, path)?, )*
            })
        }
    };
    let uniques = columns.iter().map(|c| c.unique);
    let indexed = columns.iter().map(|c| c.index);
    let searchable = columns.iter().map(|c| c.searchable.clone());
    let navs = columns
        .iter()
        .map(|c| c.nav.as_ref().map(unraw).unwrap_or_default());
    let inverses = columns
        .iter()
        .map(|c| c.inverse.as_ref().map(unraw).unwrap_or_default());
    let on_deletes = columns
        .iter()
        .map(|c| c.on_delete.as_ref().map(unraw).unwrap_or_default());
    let encrypted = columns.iter().map(|c| c.encrypted);
    let kept = columns.iter().map(|c| c.history);
    let file_opts = files.iter().map(|f| {
        let max = match f.max_size {
            Some(n) => quote! { ::std::option::Option::Some(#n) },
            None => quote! { ::std::option::Option::None },
        };
        let accept = &f.accept;
        let (keys, values): (Vec<&String>, Vec<&String>) =
            f.settings.iter().map(|(k, v)| (k, v)).unzip();
        let settings =
            (!keys.is_empty()).then(|| quote! { .settings(&[ #( (#keys, #values) ),* ]) });
        quote! { .file(#max, &[ #( #accept ),* ]) #settings }
    });
    let fields = quote! {
        ::std::vec![ #( reg.field::<#types>(#names, #docs).column(#uniques, #indexed, #encrypted).searchable(#searchable).navigation(#navs, #inverses).on_delete(#on_deletes).history(#kept) #file_opts, )* ]
    };
    // navigation handles: a ref's forward nav (when its name is not the field's own) and its inverse nav, on the
    // target (an entity of the same crate)
    let mut nav_handles = Vec::new();
    let mut inverse_handles = Vec::new();
    if entity.as_ref().is_some_and(|o| o.plugin.is_none()) {
        for ((ident, fty), col) in idents.iter().zip(&types).zip(&columns) {
            if *ident == "id" {
                continue;
            }
            let Some(target) = ref_target(fty) else {
                if col.nav.is_some() || col.inverse.is_some() || col.on_delete.is_some() {
                    return Err(syn::Error::new(
                        ident.span(),
                        "nav, inverse and on_delete are options of a ref: an Id<Target> or Option<Id<Target>> field",
                    ));
                }
                continue;
            };
            let fname = unraw(ident);
            let nav = match &col.nav {
                Some(n) => unraw(n),
                None => fname
                    .strip_suffix("_id")
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&fname)
                    .to_string(),
            };
            if nav != fname && !names.contains(&nav) {
                let nav_ident = Ident::new(&nav, ident.span());
                nav_handles.push(quote! {
                    pub const #nav_ident: ::celobus_sdk::db::Nav<#ty, #target> = ::celobus_sdk::db::Nav::new(#nav);
                });
            }
            if let Some(inv) = &col.inverse {
                let inv_name = unraw(inv);
                inverse_handles.push(quote! {
                    /// An inverse navigation another entity declares onto this one (`include`).
                    #[allow(non_upper_case_globals, dead_code)]
                    impl #target {
                        pub const #inv: ::celobus_sdk::db::Inverse<#target, #ty> = ::celobus_sdk::db::Inverse::new(#inv_name);
                    }
                });
            }
        }
    }

    // a ref of an entity joins its entity to a query that has the ref's target (the reverse of a join along it; the
    // forward one is the SDK's, for every ref): not for a ref to the entity itself, which the forward one covers
    let mut join_refs = Vec::new();
    // one impl per ref type: two refs of one type (`created_by`, `approved_by`) share it
    let mut ref_types = std::collections::HashSet::new();
    if entity.is_some() {
        for (ident, fty) in idents.iter().zip(&types) {
            if *ident == "id" {
                continue;
            }
            let Some(target) = ref_target(fty) else {
                continue;
            };
            let to_self = matches!(&target, syn::Type::Path(p)
                if p.path.segments.last().is_some_and(|s| s.ident == *ty || s.ident == "Self"));
            if to_self || !ref_types.insert(quote!(#fty).to_string()) {
                continue;
            }
            join_refs.push(quote! {
                impl ::celobus_sdk::query::JoinRef<#ty> for ::celobus_sdk::ui::Field<#ty, #fty> {
                    fn join_ref(&self) -> (::std::string::String, &'static str, ::std::string::String) {
                        (
                            ::celobus_sdk::query::canonical::<#ty>(),
                            self.name(),
                            ::celobus_sdk::query::canonical::<#target>(),
                        )
                    }
                }
            });
        }
    }

    // an entity's system columns have handles too (filters and orders name them)
    let system = match &entity {
        Some(opts) => {
            let mut handles = vec![
                quote! { pub const created_at: ::celobus_sdk::ui::Field<#ty, ::celobus_sdk::Timestamp> = ::celobus_sdk::ui::Field::new("created_at"); },
                quote! { pub const updated_at: ::celobus_sdk::ui::Field<#ty, ::celobus_sdk::Timestamp> = ::celobus_sdk::ui::Field::new("updated_at"); },
            ];
            if opts.soft_delete {
                handles.push(quote! { pub const deleted_at: ::celobus_sdk::ui::Field<#ty, ::std::option::Option<::celobus_sdk::Timestamp>> = ::celobus_sdk::ui::Field::new("deleted_at"); });
            }
            if opts.version {
                handles.push(quote! { pub const version: ::celobus_sdk::ui::Field<#ty, i64> = ::celobus_sdk::ui::Field::new("version"); });
            }
            quote! { #( #handles )* }
        }
        _ => quote! {},
    };
    let (register, entity_impl) = match entity {
        None => {
            let opts = message_options(&input.attrs)?;
            let kind = opts.kind;
            let call = opts.response.map(|resp| {
                quote! { impl __p::Call for #ty { type Response = #resp; } }
            });
            let register = quote! {
                if reg.begin::<Self>(#name) {
                    let fields = #fields;
                    reg.message::<Self>(__p::MessageInfo {
                        name: #name,
                        kind: #kind,
                        description: #description,
                        fields,
                    });
                }
            };
            (register, call)
        }
        Some(opts) => {
            let id = names.iter().position(|n| n == "id").ok_or_else(|| {
                syn::Error::new(ty.span(), "an entity has an `id: Id<Self>` field")
            })?;
            let entity_name = snake_case(&name);
            let label = opts.label.clone().unwrap_or_else(|| {
                ["name", "title", "number"]
                    .iter()
                    .find(|l| names.iter().any(|n| n == *l))
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| "id".into())
            });
            if !names.contains(&label) {
                return Err(syn::Error::new(
                    ty.span(),
                    format!("label `{label}` is not a field of {name}"),
                ));
            }
            let key = match opts.key {
                "uuid" => quote! { __p::Uuid },
                "i64" => quote! { i64 },
                _ => quote! { ::std::string::String },
            };
            let key_name = opts.key;
            let id_ty = &types[id];
            let id_check = quote! {
                const _: fn() = || {
                    fn same<T>(_: T, _: T) {}
                    fn check(a: #id_ty, b: __p::Id<#ty>) { same(a, b) }
                };
            };
            let plugin = match &opts.plugin {
                Some(p) => quote! { ::std::option::Option::Some(#p) },
                None => quote! { ::std::option::Option::None },
            };
            let (soft_delete, version, append_only) =
                (opts.soft_delete, opts.version, opts.append_only);
            let indexes = opts.indexes.iter().map(|(cols, unique)| {
                let (cols, descs): (Vec<&String>, Vec<bool>) =
                    cols.iter().map(|(c, d)| (c, *d)).unzip();
                quote! { __p::IndexInfo { columns: ::std::vec![ #( (#cols, #descs) ),* ], unique: #unique } }
            });
            let history = match &opts.history {
                Some((layer, all, unmasked)) => quote! {
                    ::std::option::Option::Some(__p::HistoryInfo { layer: #layer, all: #all, unmasked: #unmasked })
                },
                None => quote! { ::std::option::Option::None },
            };
            let storage = if opts.olap { "olap" } else { "" };
            let olap = opts.olap;
            for k in &opts.olap_key {
                if !names.contains(k) {
                    return Err(syn::Error::new(
                        ty.span(),
                        format!("key({k}): `{k}` is not a field of {name}"),
                    ));
                }
            }
            if !opts.olap_key.is_empty() && !opts.olap {
                return Err(syn::Error::new(
                    ty.span(),
                    "key(..) keys an entity stored in OLAP: declare storage = olap",
                ));
            }
            let olap_key = opts.olap_key.clone();
            let register = if opts.plugin.is_some() {
                quote! { let _ = reg; }
            } else {
                quote! {
                    if reg.begin::<Self>(#name) {
                        let fields = #fields;
                        reg.entity::<Self>(__p::EntityInfo {
                            name: #entity_name,
                            message: #name,
                            description: #description,
                            key: #key_name,
                            label: #label,
                            soft_delete: #soft_delete,
                            version: #version,
                            append_only: #append_only,
                            fields,
                            indexes: ::std::vec![ #( #indexes ),* ],
                            history: #history,
                            storage: #storage,
                            olap_key: ::std::vec![ #( #olap_key ),* ],
                        });
                    }
                }
            };
            let entity_impl = quote! {
                #id_check
                impl __p::Entity for #ty {
                    type Key = #key;
                    const ENTITY: &'static str = #entity_name;
                    const PLUGIN: ::std::option::Option<&'static str> = #plugin;
                    const OLAP: bool = #olap;
                }
                impl __p::Queryable for #ty {
                    const SOURCE: &'static str = #entity_name;
                    const OWNER: ::std::option::Option<&'static str> = #plugin;
                    const OLAP: bool = #olap;
                }
                impl __p::QueryRow for #ty {
                    type Row = __p::Row<Self>;
                    fn row(value: &__p::Value) -> __p::Result<__p::Row<Self>> {
                        <__p::Row<Self> as __p::Message>::from_json(value, "")
                    }
                }
            };
            (register, Some(entity_impl))
        }
    };

    Ok(quote! {
        const _: () = {
            use ::celobus_sdk::__private as __p;

            impl __p::Message for #ty {
                const NAME: &'static str = #name;
                fn register(reg: &mut __p::Registry) { #register }
                #to_json
            }

            impl __p::Nullable for #ty {
                type Opt = ::std::option::Option<#ty>;
            }

            impl __p::FieldValue for #ty {
                fn field_type(reg: &mut __p::Registry) -> __p::FieldType {
                    <Self as __p::Message>::register(reg);
                    __p::FieldType::message(#name)
                }
                fn encode(&self) -> __p::Value { <Self as __p::Message>::to_json(self) }
                fn decode(value: &__p::Value, path: &str) -> __p::Result<Self> {
                    <Self as __p::Message>::from_json(value, path)
                }
                fn missing(path: &str) -> __p::Result<Self> {
                    <Self as __p::Message>::from_json(&__p::Value::Object(__p::Map::new()), path)
                }
                fn encode_trimmed(&self) -> ::std::option::Option<__p::Value> {
                    match <Self as __p::Message>::to_json(self) {
                        __p::Value::Object(m) if m.is_empty() => ::std::option::Option::None,
                        v => ::std::option::Option::Some(v),
                    }
                }
            }

            impl __p::serde::Serialize for #ty {
                fn serialize<S: __p::serde::Serializer>(&self, s: S) -> ::std::result::Result<S::Ok, S::Error> {
                    __p::serde::Serialize::serialize(&<Self as __p::Message>::to_json(self), s)
                }
            }

            impl<'de> __p::serde::Deserialize<'de> for #ty {
                fn deserialize<D: __p::serde::Deserializer<'de>>(d: D) -> ::std::result::Result<Self, D::Error> {
                    let value = <__p::Value as __p::serde::Deserialize>::deserialize(d)?;
                    <Self as __p::Message>::from_json(&value, "").map_err(<D::Error as __p::serde::de::Error>::custom)
                }
            }

            #entity_impl
        };

        /// Typed handles of the fields, for UI declarations and filters (`Order::status`).
        #[allow(non_upper_case_globals, dead_code)]
        impl #ty {
            #( pub const #idents: ::celobus_sdk::ui::Field<#ty, #types> = ::celobus_sdk::ui::Field::new(#names); )*
            #system
            #( #nav_handles )*
        }

        #( #inverse_handles )*
        #( #join_refs )*
    })
}

fn message_enum(input: &DeriveInput) -> syn::Result<TokenStream2> {
    no_generics(input)?;
    let ty = &input.ident;
    let name = ty.to_string();
    let Data::Enum(data) = &input.data else {
        unreachable!()
    };
    if data.variants.is_empty() {
        return Err(syn::Error::new(ty.span(), "an enum has at least one value"));
    }
    let mut variants = Vec::new();
    let mut values = Vec::new();
    for v in &data.variants {
        if !matches!(v.fields, Fields::Unit) {
            return Err(syn::Error::new(v.span(), "enum values have no fields"));
        }
        if v.discriminant.is_some() {
            return Err(syn::Error::new(
                v.span(),
                "enum values take their numbers from the declaration order",
            ));
        }
        values.push(snake_case(&unraw(&v.ident)));
        variants.push(v.ident.clone());
    }
    let first = &variants[0];
    let indexes = (0..variants.len()).map(|i| i as u64);

    Ok(quote! {
        const _: () = {
            use ::celobus_sdk::__private as __p;

            impl __p::Enum for #ty {
                const NAME: &'static str = #name;
                const VALUES: &'static [&'static str] = &[ #( #values ),* ];
                fn as_str(&self) -> &'static str {
                    match self { #( #ty::#variants => #values, )* }
                }
                fn from_name(value: &str) -> ::std::option::Option<Self> {
                    match value { #( #values => ::std::option::Option::Some(#ty::#variants), )* _ => ::std::option::Option::None }
                }
                fn from_index(index: u64) -> ::std::option::Option<Self> {
                    match index { #( #indexes => ::std::option::Option::Some(#ty::#variants), )* _ => ::std::option::Option::None }
                }
            }

            impl __p::Nullable for #ty {
                type Opt = ::std::option::Option<#ty>;
            }

            impl __p::FieldValue for #ty {
                fn field_type(reg: &mut __p::Registry) -> __p::FieldType {
                    reg.enumeration::<Self>();
                    __p::FieldType::enumeration(#name)
                }
                fn encode(&self) -> __p::Value { __p::Value::String(<Self as __p::Enum>::as_str(self).into()) }
                fn decode(value: &__p::Value, path: &str) -> __p::Result<Self> { __p::decode_enum::<Self>(value, path) }
                fn missing(_path: &str) -> __p::Result<Self> { ::std::result::Result::Ok(#ty::#first) }
            }

            impl __p::serde::Serialize for #ty {
                fn serialize<S: __p::serde::Serializer>(&self, s: S) -> ::std::result::Result<S::Ok, S::Error> {
                    s.serialize_str(<Self as __p::Enum>::as_str(self))
                }
            }

            impl<'de> __p::serde::Deserialize<'de> for #ty {
                fn deserialize<D: __p::serde::Deserializer<'de>>(d: D) -> ::std::result::Result<Self, D::Error> {
                    let value = <__p::Value as __p::serde::Deserialize>::deserialize(d)?;
                    __p::decode_enum::<Self>(&value, "").map_err(<D::Error as __p::serde::de::Error>::custom)
                }
            }
        };
    })
}

fn unraw(ident: &Ident) -> String {
    let s = ident.to_string();
    s.strip_prefix("r#").map(str::to_string).unwrap_or(s)
}

/// The protobuf JSON name: `owner_id` → `ownerId`.
fn json_name(name: &str) -> String {
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

/// `OrderLine` → `order_line`, `HTTPServer` → `http_server`.
fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev = i.checked_sub(1).map(|j| chars[j]);
            let next = chars.get(i + 1).copied();
            let boundary = match prev {
                None => false,
                Some('_') => false,
                Some(p) if p.is_lowercase() || p.is_ascii_digit() => true,
                Some(p) => p.is_uppercase() && next.is_some_and(|n| n.is_lowercase()),
            };
            if boundary {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The first paragraph of the doc comment, its lines joined by spaces.
fn doc_paragraph(attrs: &[Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("doc")) {
        if let syn::Meta::NameValue(nv) = &attr.meta
            && let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
        {
            let value = s.value();
            if value.trim().is_empty() {
                lines.push(String::new());
            } else {
                lines.extend(value.lines().map(|l| l.trim().to_string()));
            }
        }
    }
    let paragraph: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .skip_while(|l| l.is_empty())
        .take_while(|l| !l.is_empty())
        .collect();
    paragraph.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(json_name("owner_id"), "ownerId");
        assert_eq!(json_name("a_b_c"), "aBC");
        assert_eq!(json_name("title"), "title");
        assert_eq!(snake_case("OrderLine"), "order_line");
        assert_eq!(snake_case("Note"), "note");
        assert_eq!(snake_case("HTTPServer"), "http_server");
        assert_eq!(snake_case("CustomerDeleted"), "customer_deleted");
        assert_eq!(snake_case("V2Item"), "v2_item");
    }

    #[test]
    fn settings_values_as_json() {
        use syn::parse::Parser as _;
        let lit = |s: &str| json_literal.parse_str(s).unwrap();
        assert_eq!(lit("2"), "2");
        assert_eq!(lit("-15"), "-15");
        assert_eq!(lit("0.75"), "0.75");
        assert_eq!(lit("1."), "1.0");
        assert_eq!(lit("true"), "true");
        assert_eq!(lit(r#""12.50""#), r#""12.50""#);
        assert_eq!(lit(r#"["calm", "loud"]"#), r#"["calm","loud"]"#);
        assert_eq!(lit("[]"), "[]");
        assert!(json_literal.parse_str("-\"x\"").is_err());
        assert!(json_literal.parse_str("'c'").is_err());
        assert_eq!(json_string("a\"b\\c\nd\u{1}"), r#""a\"b\\c\nd\u0001""#);
    }
}
