//! The SDK conformance plugin `prefs`: a plugin's settings, the same in Rust, TypeScript and C#. Its settings hold
//! every type a setting takes (text, a number of each kind, a decimal, a date, a bool, an enum, lists of enums, texts
//! and members, a member, a secret) and every validator, in sections. `--describe` prints the form derived from them
//! and must equal `../descriptor.json`; with `--form` the plugin lays the form out by hand and checks each change
//! (`on_change`), and `--describe` must equal `../descriptor-form.json`, as the TypeScript and C# versions' must
//! (`scripts/sdk-conformance.sh`).

use celobus_sdk::prelude::*;

#[derive(Message, Clone, Debug, PartialEq)]
pub enum Tone {
    Calm,
    LoudAndClear,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub enum Channel {
    Email,
    Chat,
    Push,
}

/// What the owners and admins of a workspace set for the plugin.
#[derive(Message, Clone, Debug)]
#[message(settings)]
pub struct PrefsSettings {
    /// After this many hours without messages a thread no longer sticks to the agent that answered in it.
    #[field(
        label = "Thread stays with its agent, hours",
        default = 2,
        min = 1,
        max = 48,
        section = "Agents"
    )]
    pub thread_warm_hours: i32,
    /// The longest message a member may send.
    #[field(
        default = 20000,
        min = 100,
        max = 20000,
        message = "From 100 to 20,000 characters",
        section = "Agents"
    )]
    pub message_max_chars: i64,
    #[field(default = 0.5, min = 0, max = 1)]
    pub sample_rate: f64,
    #[field(default = "12.50", min = "0", max = "100.00")]
    pub fee: Decimal,
    #[field(min = "2020-01-01", max = "2030-12-31")]
    pub starts_on: Option<Date>,
    #[field(default = true)]
    pub enabled: bool,
    #[field(default = "calm")]
    pub tone: Tone,
    #[field(label = "Notify by", default = ["email"], min_items = 1, max_items = 2)]
    pub channels: Vec<Channel>,
    #[field(max_items = 5, placeholder = "One per line")]
    pub tags: Vec<String>,
    #[field(max_items = 3)]
    pub watchers: Vec<Id<platform::User>>,
    pub owner: Option<Id<platform::User>>,
    /// The first message of a thread.
    #[field(multiline, pattern = "^[A-Z]", min_len = 1, max_len = 500)]
    pub greeting: String,
    #[field(
        label = "Webhook URL",
        url,
        placeholder = "https://",
        section = "Integrations"
    )]
    pub webhook_url: Option<String>,
    #[field(email, section = "Integrations")]
    pub contact: Option<String>,
    #[field(min_len = 8, max_len = 200, section = "Integrations")]
    pub webhook_token: Option<Secret>,
}

#[derive(Message, Clone, Debug)]
pub struct HoursRequest {}

#[derive(Message, Clone, Debug)]
pub struct HoursReply {
    pub hours: i32,
    pub tone: Tone,
}

/// How long a thread stays with its agent, as the settings say.
async fn hours(ctx: Ctx, _: HoursRequest) -> Result<HoursReply> {
    let s = ctx.settings::<PrefsSettings>()?;
    Ok(HoursReply {
        hours: s.thread_warm_hours,
        tone: s.tone.clone(),
    })
}

/// A token needs the URL it is for.
async fn check(_ctx: Ctx, change: SettingsChange<PrefsSettings>) -> Result<()> {
    if change.current.webhook_token.is_some() && change.current.webhook_url.is_none() {
        return Err(Error::invalid("a token needs a URL")
            .field("webhook_url", "Set the URL the token is for"));
    }
    Ok(())
}

/// The plugin; `form`: the form laid out by hand, and each change checked.
pub fn plugin(form: bool) -> Plugin {
    let settings = Settings::<PrefsSettings>::new();
    let settings = if form {
        settings.on_change(check).form(|f| {
            f.section("Agents");
            f.number(PrefsSettings::thread_warm_hours, "Hours")
                .help("How long a thread stays with its agent");
            f.number(PrefsSettings::message_max_chars, "Longest message");
            f.section("Behaviour");
            f.number(PrefsSettings::sample_rate, "Sample rate");
            f.number(PrefsSettings::fee, "Fee");
            f.date(PrefsSettings::starts_on, "Starts on");
            f.checkbox(PrefsSettings::enabled, "Enabled");
            f.radio(PrefsSettings::tone, "Tone");
            f.multi_select(PrefsSettings::channels, "Channels");
            f.textarea(PrefsSettings::tags, "Tags");
            f.member(PrefsSettings::watchers, "Watchers");
            f.member(PrefsSettings::owner, "Owner");
            f.textarea(PrefsSettings::greeting, "Greeting")
                .placeholder("Hello!");
            f.section("Integrations");
            f.text(PrefsSettings::webhook_url, "URL").required();
            f.text(PrefsSettings::contact, "Contact");
            f.secret(PrefsSettings::webhook_token, "Token");
        })
    } else {
        settings
    };
    Plugin::new("prefs", env!("CARGO_PKG_VERSION"))
        .endpoint(Endpoint::new(hours).describe("How long a thread stays with its agent."))
        .settings(settings)
}

fn main() {
    let form = std::env::args().any(|a| a == "--form");
    plugin(form).serve()
}
