//! `ctx.agents()`: the runs of the plugin's own agents, from its code.
//!
//! An endpoint starts a run of an agent the plugin declares and answers its id; the screen reads the run through
//! another endpoint of the plugin and re-reads when the agent's topic is invalidated (`Agent::topic`).
//!
//! ```no_run
//! # use celobus_sdk::prelude::*;
//! # async fn ask(ctx: Ctx, question: String) -> Result<String> {
//! let run = ctx.agents().run("assistant", question).await?;
//! // later, in the endpoint the screen reads:
//! let page = ctx.agents().events(&run.id, 0, 0).await?;
//! let answer = page.run.result.clone().unwrap_or_else(|| page.partial.text());
//! # Ok(answer)
//! # }
//! ```
//!
//! A plugin reaches the runs of its own agents, all of them, whoever started them, and the runs its code started:
//! its endpoints decide whom to show which run. A run is started, and a message sent, by the caller of the request
//! being handled (`AgentRun::started_by`); outside a request both are refused with 403 `forbidden`.
//!
//! The agents of the workspace's other plugins are started by `<plugin>.<name>`, for a caller who reaches that
//! plugin (`available()` lists them), never for a caller that is an agent's account; the plugin follows such a run
//! through a topic of its own (`RunOptions::topic`, or one key of it: `RunOptions::topic_key`), since the agent's
//! topic is its plugin's.
//!
//! ```no_run
//! # use celobus_sdk::prelude::*;
//! # #[derive(Message, Clone, Debug)] pub struct ThreadKey { pub thread: String }
//! # use celobus_sdk::ui::Topic;
//! # const THREAD: Topic<ThreadKey> = Topic::new("thread");
//! # async fn mention(ctx: Ctx) -> Result<()> {
//! let agents = ctx.agents().available().await?;
//! if let Some(creator) = agents.iter().find(|a| a.agent == "crm.assistant") {
//!     let key = ThreadKey { thread: "t-1".into() };
//!     let options = RunOptions::default().topic_key(THREAD, &key);
//!     ctx.agents().run_with(&creator.agent, "Make a plugin for leave requests.", options).await?;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Every run belongs to a group (`AgentRun::group`): a run started without one founds
//! it, and the plugin adds runs of its other agents to it with `RunOptions::group`. The agents of a group write
//! to each other by name and may bring in the agents their declaration lists (`Agent::delegates`).
//!
//! ```no_run
//! # use celobus_sdk::prelude::*;
//! # async fn both(ctx: Ctx) -> Result<()> {
//! let first = ctx.agents().run("fulfilment", "What is unpaid?").await?;
//! let options = RunOptions::default().group(&first.group);
//! let second = ctx.agents().run_with("release", "Draft the notes.", options).await?;
//! assert_eq!(second.group, first.group);
//! # Ok(())
//! # }
//! ```

use std::future::IntoFuture;
use std::time::Duration;

use celobus_proto as pb;
use futures::future::BoxFuture;
use pb::{syscall, syscall_result};
use serde_json::{Map, Value};

use crate::ctx::Ctx;
use crate::error::{Error, Result};
use crate::identity::AgentEngine;
use crate::platform::User;
use crate::types::{File, Id};

/// Where a run is: `Starting` (the run is being set up) → `Running` ⇄ `Idle` (a session between turns) → one of
/// the four ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AgentRunStatus {
    Starting,
    Running,
    Idle,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    /// A status this SDK does not know (a newer platform).
    Unknown,
}

impl AgentRunStatus {
    fn parse(s: &str) -> Self {
        match s {
            "starting" | "queued" => Self::Starting,
            "running" => Self::Running,
            "idle" => Self::Idle,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "timed_out" => Self::TimedOut,
            _ => Self::Unknown,
        }
    }

    /// The platform's name of the status (`timed_out`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Unknown => "unknown",
        }
    }

    /// The run is over: nothing more is written to it.
    pub const fn ended(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }
}

impl std::fmt::Display for AgentRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who started a run, or sent it a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentActor {
    pub user_id: Id<User>,
    pub display_name: String,
}

/// Why a run failed or timed out (`start_timeout`, `lost`, `agent_failed`, `agent_run_timed_out`, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRunError {
    pub code: String,
    pub message: String,
}

/// A run of one of the plugin's agents, or one its code started.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRun {
    pub id: String,
    /// The agent's name in its plugin (`plugin`).
    pub agent: String,
    pub plugin: String,
    pub engine: String,
    /// `None`: the engine's own model.
    pub model: Option<String>,
    pub status: AgentRunStatus,
    /// The run waits for follow-up messages after every turn.
    pub session: bool,
    /// The task it was started with.
    pub input: String,
    /// The result of the last finished turn.
    pub result: Option<String>,
    pub error: Option<AgentRunError>,
    /// The caller of the request the run was started in (or of the HTTP call that started it).
    pub started_by: AgentActor,
    /// The plugin whose code started it; `None`: started over HTTP (the CLI, a client).
    pub started_by_plugin: Option<String>,
    /// Unix milliseconds, like the two below.
    pub created_at: i64,
    pub updated_at: i64,
    pub deadline_at: i64,
    pub turn: u32,
    /// The sequence number of the run's last event.
    pub last_seq: u64,
    /// Where the run executes, as the platform last reported it.
    pub pod: String,
    /// The group the run belongs to: the id of the group's first run (its own id for a run that founded one).
    pub group: String,
    /// The agent run that started this one; `None`: a person or a plugin's code did.
    pub parent: Option<String>,
    /// The agent of that run, as `<plugin>.<name>`.
    pub parent_agent: Option<String>,
    /// The person at the root: who started the run, or the one the run that started it works for.
    pub on_behalf_of: AgentActor,
}

impl AgentRun {
    fn from_pb(r: pb::AgentRun) -> Self {
        let text = |s: String| Some(s).filter(|s| !s.is_empty());
        let by = r.started_by.unwrap_or_default();
        let started_by = AgentActor {
            user_id: Id::new(by.user_id),
            display_name: by.display_name,
        };
        Self {
            agent: r.agent,
            plugin: r.plugin,
            engine: r.engine,
            model: text(r.model),
            status: AgentRunStatus::parse(&r.status),
            session: r.session,
            input: r.input,
            result: text(r.result),
            error: r.error.map(|e| AgentRunError {
                code: e.code,
                message: e.message,
            }),
            // a platform of an earlier release names no group: the run is its own
            group: if r.group.is_empty() {
                r.id.clone()
            } else {
                r.group
            },
            id: r.id,
            parent: text(r.parent),
            parent_agent: text(r.parent_agent),
            on_behalf_of: match r.on_behalf_of {
                Some(p) => AgentActor {
                    user_id: Id::new(p.user_id),
                    display_name: p.display_name,
                },
                None => started_by.clone(),
            },
            started_by,
            started_by_plugin: text(r.started_by_plugin),
            created_at: r.created_at,
            updated_at: r.updated_at,
            deadline_at: r.deadline_at,
            turn: r.turn,
            last_seq: r.last_seq,
            pod: r.pod,
        }
    }

    /// The run is over (`status.ended()`).
    pub fn ended(&self) -> bool {
        self.status.ended()
    }
}

/// One event of a run: `kind` is its type (`message`, `tool`, `turn`, `status`, `progress`, `error`, `result`,
/// `handoff`), `data` the whole event as the platform keeps it, with the fields of its type.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentEvent {
    pub seq: u64,
    /// Unix milliseconds.
    pub at: i64,
    pub kind: String,
    pub data: Map<String, Value>,
}

impl AgentEvent {
    /// A string field of the event (`text`, `role`, `name`, `phase`, `summary`, `status`, `code`, `message`).
    pub fn str(&self, field: &str) -> Option<&str> {
        self.data.get(field).and_then(Value::as_str)
    }

    /// The text of a `message`, `status` or `progress` event.
    pub fn text(&self) -> Option<&str> {
        self.str("text")
    }
}

/// The text of one message while the agent writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPartialMessage {
    pub message_id: String,
    pub text: String,
}

/// The messages being written right now: text, not events. The finished message arrives as an event and takes
/// their place. `rev` counts the changes (0: nothing was written yet).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentPartial {
    pub rev: u64,
    pub messages: Vec<AgentPartialMessage>,
}

impl AgentPartial {
    /// The unfinished text, the messages joined by blank lines.
    pub fn text(&self) -> String {
        self.messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// A run with its events after a sequence number and its unfinished text.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentEvents {
    pub run: AgentRun,
    /// Oldest first.
    pub events: Vec<AgentEvent>,
    pub partial: AgentPartial,
}

/// A change of a run the plugin sees, as the run-change hook tells it ([`crate::Plugin::on_run_change`]): a run of
/// one of the plugin's agents, or one its code started. It names the run and where it stands; the handler reads the
/// rest with `ctx.agents()`. Delivered at least once and coalesced (the latest state of a burst): handle it
/// idempotently, and read the run rather than trust the order of deliveries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunChange {
    pub run_id: String,
    /// The agent's name in its plugin.
    pub agent: String,
    /// The agent's plugin.
    pub plugin: String,
    pub status: AgentRunStatus,
    pub turn: u32,
    /// The sequence number of the run's last event.
    pub last_seq: u64,
    /// The topic the start named (`RunOptions::topic`; empty: none), and its canonical key (empty: every key).
    pub topic: String,
    pub topic_key: String,
    /// The run is over: nothing more is written to it.
    pub ended: bool,
}

impl RunChange {
    /// Reads the hook's body (`{run_id, agent, plugin, status, turn, last_seq, topic, topic_key, ended}`).
    pub(crate) fn from_json(v: &Value) -> Result<Self> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        let run_id = s("run_id");
        if run_id.is_empty() {
            return Err(Error::invalid("run_id is required"));
        }
        let status = AgentRunStatus::parse(&s("status"));
        Ok(Self {
            run_id,
            agent: s("agent"),
            plugin: s("plugin"),
            status,
            turn: n("turn") as u32,
            last_seq: n("last_seq"),
            topic: s("topic"),
            topic_key: s("topic_key"),
            ended: v
                .get("ended")
                .and_then(Value::as_bool)
                .unwrap_or(status.ended()),
        })
    }
}

/// How a run is started, when it differs from what the agent declares:
/// `RunOptions::default().session(true).timeout(Duration::from_secs(600))`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOptions {
    engine: Option<AgentEngine>,
    model: Option<String>,
    session: Option<bool>,
    timeout: Option<Duration>,
    group: Option<String>,
    notify: bool,
    notify_link: Option<crate::notify::Link>,
    topic: Option<String>,
    topic_key: String,
    files: Vec<File>,
}

impl RunOptions {
    /// Files the run is given: the plugin's own files (409 `file_unknown` otherwise), kept while the run lives. The
    /// task lists them, a line each with the name, type, size and id; the agent downloads the ones it needs.
    pub fn files(mut self, files: &[File]) -> Self {
        self.files = files.to_vec();
        self
    }

    /// The run joins this group instead of founding one: the `group` of a run this plugin's code started
    /// without one. 404 `unknown_group` when the platform no longer remembers the group's first run or another
    /// plugin's code founded it, 409 `agent_in_group` when the agent has a live run there already, 409
    /// `group_full` when the group has its eight live runs.
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    pub fn engine(mut self, engine: AgentEngine) -> Self {
        self.engine = Some(engine);
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// The run is a session (it waits for follow-up messages after every turn), or is not.
    pub fn session(mut self, session: bool) -> Self {
        self.session = Some(session);
        self
    }

    /// The longest the run takes, from a minute to six hours.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Tell the person the run works for, in their inbox, when it finishes (a run that fails or runs out of time
    /// always tells them).
    pub fn notify(mut self) -> Self {
        self.notify = true;
        self
    }

    /// Where the notification of the run's end opens: a screen or overlay of this plugin's UI (400
    /// `bad_request` when the UI has no such target or params).
    pub fn notify_link(mut self, link: crate::notify::Link) -> Self {
        self.notify_link = Some(link);
        self
    }

    /// A topic of this plugin (declared with `Plugin::topic`) the platform invalidates, for every key, whenever the run
    /// changes, as it does the topic the agent declares: how a plugin follows the runs it starts of another plugin's
    /// agent. 400 `bad_request` when the plugin declares no such topic.
    pub fn topic<K: crate::message::Message>(mut self, topic: crate::ui::Topic<K>) -> Self {
        self.topic = Some(topic.name().to_string());
        self.topic_key.clear();
        self
    }

    /// Like `topic`, for one key of it only: the screens that follow the topic for `key` re-read when the run
    /// changes, and no other (a conversation's screens, not every screen of the plugin).
    pub fn topic_key<K: crate::message::Message>(
        mut self,
        topic: crate::ui::Topic<K>,
        key: &K,
    ) -> Self {
        self.topic = Some(topic.name().to_string());
        self.topic_key = crate::ctx::canonical_key(&key.to_json());
        self
    }
}

/// An agent of the workspace the caller of the request may start a run of (`ctx.agents().available()`), as its
/// installed package describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableAgent {
    /// `<plugin>.<name>`: what `run` takes.
    pub agent: String,
    pub plugin: String,
    pub name: String,
    /// Its display name: the declared title, else its name.
    pub title: String,
    /// What it is for, in a sentence (empty: not said).
    pub description: String,
    /// Its glyph, a lucide icon name.
    pub icon: Option<String>,
    /// The engine its runs get in this workspace when a start names none: an owner's or admin's choice, else the one
    /// it declares, else its connection's; `None`: none can be resolved (it has no connection).
    pub engine: Option<String>,
    /// Its runs are sessions by its declaration.
    pub session: bool,
    /// Its plugin's title, else the plugin's id.
    pub plugin_title: String,
}

impl AvailableAgent {
    fn from_pb(a: pb::AvailableAgent) -> Self {
        let text = |s: String| Some(s).filter(|s| !s.is_empty());
        Self {
            agent: a.agent,
            plugin: a.plugin,
            name: a.name,
            title: a.title,
            description: a.description,
            icon: text(a.icon),
            engine: text(a.engine),
            session: a.session,
            plugin_title: a.plugin_title,
        }
    }
}

/// The runs of the plugin's agents (`ctx.agents()`).
pub struct Agents<'a> {
    ctx: &'a Ctx,
}

fn unexpected(what: &str) -> Error {
    Error::internal(format!("the platform answered {what} with something else"))
}

impl<'a> Agents<'a> {
    pub(crate) fn new(ctx: &'a Ctx) -> Self {
        Self { ctx }
    }

    async fn run_of(&self, what: &str, call: syscall::Call) -> Result<AgentRun> {
        match self.ctx.syscall(call).await? {
            Some(syscall_result::Result::AgentRun(r)) => Ok(AgentRun::from_pb(r)),
            _ => Err(unexpected(what)),
        }
    }

    /// Starts a run of the agent `agent` with the task `input`, as the agent declares its runs: the plugin's own
    /// agent by its name (`assistant`), or an agent of another plugin of the workspace as `<plugin>.<name>`. It is
    /// started by this request's caller and founds a group (`AgentRun::group`). 404 `unknown_agent` when there is
    /// no such agent, 403 `forbidden` when the caller does not reach another plugin's agent's plugin or is the
    /// account of an agent.
    /// `.files(&[..])` gives the run files (`RunOptions::files`).
    pub fn run(&self, agent: &str, input: impl Into<String>) -> StartRun<'a> {
        StartRun {
            ctx: self.ctx,
            agent: agent.into(),
            input: input.into(),
            options: RunOptions::default(),
        }
    }

    /// Starts a run with another engine, model, session or time than the agent declares, or in a group the
    /// plugin's code founded (`RunOptions::group`).
    pub async fn run_with(
        &self,
        agent: &str,
        input: impl Into<String>,
        options: RunOptions,
    ) -> Result<AgentRun> {
        let call = syscall::Call::AgentRun(pb::AgentRunStart {
            agent: agent.into(),
            input: input.into(),
            engine: options
                .engine
                .map(|e| e.name().to_string())
                .unwrap_or_default(),
            model: options.model.unwrap_or_default(),
            session: options.session,
            timeout_s: options
                .timeout
                .map(|t| t.as_secs().min(u32::MAX as u64) as u32)
                .unwrap_or(0),
            request_id: self.ctx.request().id(),
            group: options.group.unwrap_or_default(),
            notify: options.notify,
            notify_link: options.notify_link.as_ref().map(crate::notify::Link::wire),
            topic: options.topic.unwrap_or_default(),
            topic_key: options.topic_key,
            files: options.files.iter().map(file_to_pb).collect(),
        });
        self.run_of("agent_run", call).await
    }

    /// The agents of the workspace this request's caller may start a run of: those of the plugins they reach, this
    /// plugin's own among them, sorted by plugin and name. 403 `forbidden` outside a request of a signed-in caller
    /// and for the account of an agent.
    pub async fn available(&self) -> Result<Vec<AvailableAgent>> {
        let call = syscall::Call::AgentsAvailable(pb::AgentsAvailable {
            request_id: self.ctx.request().id(),
        });
        match self.ctx.syscall(call).await? {
            Some(syscall_result::Result::AgentsAvailable(page)) => Ok(page
                .agents
                .into_iter()
                .map(AvailableAgent::from_pb)
                .collect()),
            _ => Err(unexpected("agents_available")),
        }
    }

    /// The run; 404 `not_found` when it is neither a run of this plugin's agents nor one its code started.
    pub async fn get(&self, run_id: &str) -> Result<AgentRun> {
        let call = syscall::Call::AgentGet(pb::AgentRunGet {
            run_id: run_id.into(),
        });
        self.run_of("agent_get", call).await
    }

    /// The run, its events after the sequence number `after` (0: from the first; oldest first, at most `limit`,
    /// 0: all the run keeps) and the text being written. Answered at once: follow the agent's topic to know when
    /// to read again.
    pub async fn events(&self, run_id: &str, after: u64, limit: u32) -> Result<AgentEvents> {
        let call = syscall::Call::AgentEvents(pb::AgentRunEvents {
            run_id: run_id.into(),
            after,
            limit,
        });
        let Some(syscall_result::Result::AgentEvents(page)) = self.ctx.syscall(call).await? else {
            return Err(unexpected("agent_events"));
        };
        let events = page
            .events
            .into_iter()
            .map(|e| AgentEvent {
                seq: e.seq,
                at: e.at,
                kind: e.r#type,
                data: serde_json::from_slice(&e.json).unwrap_or_default(),
            })
            .collect();
        let partial = page.partial.unwrap_or_default();
        Ok(AgentEvents {
            run: AgentRun::from_pb(page.run.ok_or_else(|| unexpected("agent_events"))?),
            events,
            partial: AgentPartial {
                rev: partial.rev,
                messages: partial
                    .messages
                    .into_iter()
                    .map(|m| AgentPartialMessage {
                        message_id: m.message_id,
                        text: m.text,
                    })
                    .collect(),
            },
        })
    }

    /// The recent runs the plugin sees (of its agents, and the runs its code started), newest first: of one agent
    /// (`Some("assistant")`, or `Some("<plugin>.<name>")` of another plugin's) or of all, at most `limit` (0: 50;
    /// the platform keeps 200).
    pub async fn list(&self, agent: Option<&str>, limit: u32) -> Result<Vec<AgentRun>> {
        self.runs(pb::AgentRunList {
            agent: agent.unwrap_or_default().into(),
            limit,
            group: String::new(),
        })
        .await
    }

    /// The runs of the plugin's agents in one group (`AgentRun::group`), newest first: who works together on
    /// one conversation.
    pub async fn group(&self, group: &str) -> Result<Vec<AgentRun>> {
        self.runs(pb::AgentRunList {
            agent: String::new(),
            limit: 0,
            group: group.into(),
        })
        .await
    }

    async fn runs(&self, list: pb::AgentRunList) -> Result<Vec<AgentRun>> {
        match self.ctx.syscall(syscall::Call::AgentRuns(list)).await? {
            Some(syscall_result::Result::AgentRuns(page)) => {
                Ok(page.runs.into_iter().map(AgentRun::from_pb).collect())
            }
            _ => Err(unexpected("agent_runs")),
        }
    }

    /// Sends a follow-up message to a run that has not ended, as this request's caller; the sequence number of
    /// the input. `.files(&[..])` gives the run more files, as `RunOptions::files` does. 409 `run_ended` when the
    /// run is over.
    pub fn send(&self, run_id: &str, text: impl Into<String>) -> SendInput<'a> {
        SendInput {
            ctx: self.ctx,
            run_id: run_id.into(),
            text: text.into(),
            files: vec![],
        }
    }

    /// Stops the run now; a run that ended stays as it is.
    pub async fn cancel(&self, run_id: &str) -> Result<AgentRun> {
        let call = syscall::Call::AgentCancel(pb::AgentRunCancel {
            run_id: run_id.into(),
        });
        self.run_of("agent_cancel", call).await
    }

    /// Ends a session after the turn in progress.
    pub async fn close(&self, run_id: &str) -> Result<AgentRun> {
        let call = syscall::Call::AgentClose(pb::AgentRunClose {
            run_id: run_id.into(),
        });
        self.run_of("agent_close", call).await
    }
}

fn file_to_pb(f: &File) -> pb::File {
    pb::File {
        id: f.id.clone(),
        name: f.name.clone(),
        content_type: f.content_type.clone(),
        size: f.size,
    }
}

/// A run being started (`ctx.agents().run(agent, input)`); `.await` starts it.
pub struct StartRun<'a> {
    ctx: &'a Ctx,
    agent: String,
    input: String,
    options: RunOptions,
}

impl StartRun<'_> {
    /// Files the run is given (`RunOptions::files`).
    pub fn files(mut self, files: &[File]) -> Self {
        self.options = self.options.files(files);
        self
    }
}

impl<'a> IntoFuture for StartRun<'a> {
    type Output = Result<AgentRun>;
    type IntoFuture = BoxFuture<'a, Result<AgentRun>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            Agents::new(self.ctx)
                .run_with(&self.agent, self.input, self.options)
                .await
        })
    }
}

/// A follow-up message being sent to a run (`ctx.agents().send(run_id, text)`); `.await` sends it and answers the
/// sequence number of the input.
pub struct SendInput<'a> {
    ctx: &'a Ctx,
    run_id: String,
    text: String,
    files: Vec<File>,
}

impl SendInput<'_> {
    /// Files the run is given with the message: the plugin's own files (409 `file_unknown` otherwise), kept while
    /// the run lives.
    pub fn files(mut self, files: &[File]) -> Self {
        self.files = files.to_vec();
        self
    }
}

impl<'a> IntoFuture for SendInput<'a> {
    type Output = Result<u64>;
    type IntoFuture = BoxFuture<'a, Result<u64>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let call = syscall::Call::AgentSend(pb::AgentRunSend {
                run_id: self.run_id,
                text: self.text,
                request_id: self.ctx.request().id(),
                files: self.files.iter().map(file_to_pb).collect(),
            });
            match self.ctx.syscall(call).await? {
                Some(syscall_result::Result::AgentInput(i)) => Ok(i.seq),
                _ => Err(unexpected("agent_send")),
            }
        })
    }
}
