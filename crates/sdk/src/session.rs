//! The SDK's session with the platform and the dispatch of requests.

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use celobus_proto as pb;
use futures::FutureExt as _;
use pb::{celld_message, plugin_message};
use serde_json::Value;
use tokio::sync::{Semaphore, mpsc, watch};
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::ReceiverStream;
use tracing::Instrument as _;

use crate::ctx::{Caller, Ctx, Link, PluginInfo, RequestInfo, endpoint_path};
use crate::descriptor::DescribeErrors;
use crate::endpoint::Handler;
use crate::error::Error;
use crate::jobs::{JobHandler, JobInfo};
use crate::plugin::Plugin;
use crate::runlog::{RUN, RunLog};

const MAX_MESSAGE: usize = 64 << 20;
/// Requests handled at once; more wait in a queue of `MAX_QUEUED`, past it the answer is 503.
const MAX_ADMITTED: usize = 64;
const MAX_QUEUED: usize = 256;
/// How long in-flight requests may finish after SIGTERM.
const DRAIN: Duration = Duration::from_secs(10);
const BACKOFF_MIN: Duration = Duration::from_millis(100);
const BACKOFF_MAX: Duration = Duration::from_secs(5);

struct Route {
    name: &'static str,
    handler: Handler,
    timeout: Duration,
    /// a request of the platform itself (the run-change hook): refused when it carries a caller
    system: bool,
}

/// How long the run-change hook may take.
const RUN_CHANGE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the settings check hook may take.
const SETTINGS_CHECK_TIMEOUT: Duration =
    Duration::from_millis(celobus_descriptor::settings::SETTINGS_CHECK_TIMEOUT_MS);

/// A plugin ready to serve: its routes by path.
pub(crate) struct Service {
    info: Arc<PluginInfo>,
    depends: Vec<String>,
    routes: HashMap<String, Route>,
    permits: Arc<Semaphore>,
    inflight: AtomicUsize,
    /// the jobs' handlers by key
    jobs: HashMap<String, JobHandler>,
    /// the job pool's size, announced at registration (0: no jobs)
    job_slots: u32,
}

/// The runs in flight on a session, by run id: what stops its handler.
pub(crate) type Runs = Arc<std::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<i32>>>>;

impl Service {
    pub(crate) fn new(plugin: Plugin) -> Result<Self, DescribeErrors> {
        let descriptor = plugin.descriptor()?;
        let routes = plugin
            .endpoints
            .iter()
            .filter_map(|e| {
                let name = e.name()?;
                let route = Route {
                    name,
                    handler: e.handler.clone(),
                    timeout: e.timeout,
                    system: false,
                };
                Some((endpoint_path(&plugin.id, name), route))
            })
            .chain(plugin.on_run_change.clone().map(|handler| {
                let route = Route {
                    name: celobus_descriptor::RUN_CHANGE_METHOD,
                    handler,
                    timeout: RUN_CHANGE_TIMEOUT,
                    system: true,
                };
                (celobus_descriptor::run_change_path(&plugin.id), route)
            }))
            .chain(
                plugin
                    .settings
                    .as_ref()
                    .and_then(|s| s.on_change.clone())
                    .map(|handler| {
                        let route = Route {
                            name: celobus_descriptor::SETTINGS_CHECK_METHOD,
                            handler,
                            timeout: SETTINGS_CHECK_TIMEOUT,
                            system: true,
                        };
                        (celobus_descriptor::settings_check_path(&plugin.id), route)
                    }),
            )
            .collect();
        let jobs: HashMap<String, JobHandler> = plugin
            .jobs
            .iter()
            .map(|j| (j.name.clone(), j.handler.clone()))
            .collect();
        let job_slots = if jobs.is_empty() { 0 } else { plugin.job_slots };
        Ok(Self {
            info: Arc::new(
                PluginInfo::new(plugin.id.clone(), plugin.version.clone())
                    .with_jobs(plugin.jobs.iter().map(|j| (j.payload, j.name.clone())))
                    .with_settings_message(plugin.settings.as_ref().map(|s| s.message)),
            ),
            jobs,
            job_slots,
            depends: descriptor
                .dependencies
                .iter()
                .map(|d| d.plugin.clone())
                .collect(),
            routes,
            permits: Arc::new(Semaphore::new(MAX_ADMITTED)),
            inflight: AtomicUsize::new(0),
        })
    }

    /// Answers one request: route, method, content type, JSON decode, the handler (bounded by the
    /// endpoint's timeout, panics caught), JSON encode, errors as status + `{code, message, details}`.
    pub(crate) async fn dispatch(&self, link: Arc<Link>, req: pb::Request) -> pb::Response {
        let id = req.id;
        let mut resp = self.answer(link, req).await;
        resp.id = id;
        resp
    }

    async fn answer(&self, link: Arc<Link>, req: pb::Request) -> pb::Response {
        // the platform's own requests carry no caller: one with a caller came from outside
        let route = self
            .routes
            .get(&req.path)
            .filter(|r| !(r.system && req.principal.is_some()));
        let Some(route) = route else {
            return error_response(&Error::new(
                404,
                "unknown_endpoint",
                format!("no endpoint {}", req.path),
            ));
        };
        if !req.method.eq_ignore_ascii_case("POST") {
            let mut resp = error_response(&Error::new(
                405,
                "method_not_allowed",
                format!("{} takes POST, not {}", route.name, req.method),
            ));
            resp.headers.push(header("allow", "POST"));
            return resp;
        }
        let content_type = header_value(&req.headers, "content-type").unwrap_or("");
        let media = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !(media.is_empty() || media == "application/json") {
            return error_response(&Error::new(
                415,
                "unsupported_media_type",
                format!("{media} is not supported: send application/json"),
            ));
        }
        let body: Value = if req.body.iter().all(u8::is_ascii_whitespace) {
            Value::Object(Default::default())
        } else {
            match serde_json::from_slice(&req.body) {
                Ok(v) => v,
                Err(e) => {
                    return error_response(&Error::invalid(format!(
                        "the body is not valid JSON: {e}"
                    )));
                }
            }
        };
        let idempotency_key = header_value(&req.headers, "idempotency-key").map(str::to_string);
        let ctx = Ctx::new(
            link,
            self.info.clone(),
            RequestInfo::new(req.id, route.timeout, idempotency_key),
            Caller::from_principal(req.principal, self.info.id()),
        );
        let span = tracing::info_span!("request", request_id = req.id, endpoint = route.name);
        let started = Instant::now();
        let run = AssertUnwindSafe((route.handler)(ctx, body)).catch_unwind();
        let outcome = tokio::time::timeout(route.timeout, run)
            .instrument(span.clone())
            .await;
        let _enter = span.enter();
        let result = match outcome {
            Err(_) => Err(Error::deadline_exceeded(format!(
                "{} did not answer within {} ms",
                route.name,
                route.timeout.as_millis()
            ))),
            Ok(Err(panic)) => Err(Error::internal(format!(
                "the handler panicked: {}",
                panic_text(&panic)
            ))),
            Ok(Ok(result)) => result,
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(value) => {
                tracing::debug!(elapsed_ms, status = 200, "answered");
                json_response(200, &value)
            }
            Err(e) => {
                if e.status() >= 500 {
                    tracing::error!(
                        elapsed_ms,
                        status = e.status(),
                        code = e.code(),
                        error = e.message(),
                        "failed"
                    );
                } else {
                    tracing::info!(
                        elapsed_ms,
                        status = e.status(),
                        code = e.code(),
                        error = e.message(),
                        "refused"
                    );
                }
                error_response(&e)
            }
        }
    }
}

fn header(name: &str, value: &str) -> pb::Header {
    pb::Header {
        name: name.into(),
        value: value.into(),
    }
}

fn header_value<'a>(headers: &'a [pb::Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

fn json_response(status: u16, value: &Value) -> pb::Response {
    pb::Response {
        id: 0,
        status: status as u32,
        headers: vec![header("content-type", "application/json")],
        body: Bytes::from(serde_json::to_vec(value).unwrap_or_default()),
    }
}

pub(crate) fn error_response(e: &Error) -> pb::Response {
    json_response(e.status(), &e.body())
}

/// JSON lines on stderr, and a job run's lines with the run too (`crate::runlog`); the level from `CELOBUS_LOG` or
/// `RUST_LOG` (default `info`).
pub(crate) fn init_logging() {
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;
    let filter = tracing_subscriber::EnvFilter::try_from_env("CELOBUS_LOG")
        .or_else(|_| tracing_subscriber::EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_current_span(true)
                .with_span_list(false)
                .with_writer(std::io::stderr),
        )
        .with(crate::runlog::run_log_layer())
        .try_init();
}

impl Service {
    /// The job pool's size (0: the plugin has no jobs).
    #[cfg(test)]
    pub(crate) fn job_slots(&self) -> u32 {
        self.job_slots
    }

    /// The plugin, as its contexts see it.
    #[cfg(test)]
    pub(crate) fn info(&self) -> Arc<PluginInfo> {
        self.info.clone()
    }

    /// Starts a run's attempt on the job pool: its handler, stopped by a `JobAbort` (or the session's end), then its
    /// `JobDone`.
    pub(crate) fn start_run(
        &self,
        run: pb::JobRun,
        link: Arc<Link>,
        tx: mpsc::Sender<pb::PluginMessage>,
        runs: &Runs,
        slots: &Arc<Semaphore>,
        draining: bool,
    ) {
        let (run_id, attempt) = (run.run_id.clone(), run.attempt);
        let refuse = |code: &'static str, message: String| {
            let done =
                crate::jobs::job_done(run_id.clone(), attempt, Err(Error::new(503, code, message)));
            let tx = tx.clone();
            tokio::spawn(async move {
                let _ = tx.send(job_done(done)).await;
            });
        };
        if draining {
            return refuse("shutdown", "the plugin is stopping".into());
        }
        let Some(handler) = self.jobs.get(&run.job).cloned() else {
            return refuse(
                "unavailable",
                format!("this version has no job {}", run.job),
            );
        };
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            return refuse(
                "unavailable",
                format!("all {} job slots are taken", self.job_slots),
            );
        };
        let (abort_tx, abort_rx) = tokio::sync::oneshot::channel::<i32>();
        runs.lock().expect("runs").insert(run_id.clone(), abort_tx);
        let ctx = Ctx::for_run(
            link,
            self.info.clone(),
            RequestInfo::new(
                run.request_id,
                Duration::from_millis(run.timeout_ms as u64),
                None,
            ),
            Caller::from_principal(run.principal, self.info.id()),
            JobInfo {
                job: run.job.clone(),
                run_id: run_id.clone(),
                attempt,
                max_attempts: run.max_attempts,
                scheduled_at: chrono::DateTime::from_timestamp_millis(run.scheduled_at_unix_ms)
                    .unwrap_or_default(),
            },
        );
        let runs = runs.clone();
        let span = tracing::info_span!("job", job = %run.job, run_id = %run_id, attempt);
        let log = RunLog {
            request_id: run.request_id,
            tx: tx.clone(),
        };
        tokio::spawn(
            async move {
                let started = Instant::now();
                let result = match serde_json::from_str::<Value>(&run.payload) {
                    Err(e) => Err(Error::new(
                        400,
                        "payload_invalid",
                        format!("the payload is not JSON: {e}"),
                    )),
                    Ok(payload) => {
                        let work = AssertUnwindSafe(RUN.scope(log, handler(ctx, payload)))
                            .catch_unwind();
                        tokio::select! {
                            r = work => match r {
                                Ok(r) => r,
                                Err(panic) => Err(Error::internal(format!(
                                    "the handler panicked: {}",
                                    panic_text(&panic)
                                ))),
                            },
                            reason = abort_rx => {
                                // the handler's future is dropped here: it stops at its next await
                                let code = reason.map(crate::jobs::abort_code).unwrap_or("cancelled");
                                Err(Error::new(503, code, format!("stopped: {code}")))
                            }
                        }
                    }
                };
                let elapsed_ms = started.elapsed().as_millis() as u64;
                match &result {
                    Ok(()) => tracing::info!(elapsed_ms, "run done"),
                    Err(e) => tracing::warn!(
                        elapsed_ms,
                        code = e.code(),
                        error = e.message(),
                        permanent = e.is_permanent(),
                        "run failed"
                    ),
                }
                runs.lock().expect("runs").remove(&run_id);
                drop(permit);
                let _ = tx
                    .send(job_done(crate::jobs::job_done(run_id, attempt, result)))
                    .await;
            }
            .instrument(span),
        );
    }
}

fn panic_text(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic".into())
}

fn job_done(done: pb::JobDone) -> pb::PluginMessage {
    pb::PluginMessage {
        kind: Some(plugin_message::Kind::JobDone(done)),
    }
}

/// Resolves on SIGTERM or Ctrl-C.
pub(crate) async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Keeps a session with the platform at `addr`, reconnecting with backoff, until `shutdown` resolves;
/// then lets in-flight requests finish (at most `DRAIN`) and returns.
pub(crate) async fn serve_until(
    service: Arc<Service>,
    addr: &str,
    shutdown: impl Future<Output = ()>,
) {
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut shutdown = std::pin::pin!(shutdown);
    let mut backoff = BACKOFF_MIN;
    let mut sessions = std::pin::pin!(async {
        loop {
            let started = Instant::now();
            match session(&service, addr, stop_rx.clone()).await {
                Ok(Ended::Stopped) => return,
                Ok(Ended::Closed) => {
                    tracing::warn!("the platform closed the session; reconnecting")
                }
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), "the session with the platform failed; reconnecting")
                }
            }
            if *stop_rx.borrow() {
                return;
            }
            if started.elapsed() > Duration::from_secs(10) {
                backoff = BACKOFF_MIN;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    });
    tokio::select! {
        _ = &mut sessions => return,
        _ = &mut shutdown => {}
    }
    tracing::info!("stopping: finishing the requests in flight");
    let _ = stop_tx.send(true);
    if tokio::time::timeout(DRAIN + Duration::from_secs(1), &mut sessions)
        .await
        .is_err()
    {
        tracing::warn!("stopped with requests still in flight");
    }
}

enum Ended {
    /// The platform ended the stream.
    Closed,
    /// Asked to stop, and the requests in flight are done.
    Stopped,
}

async fn session(
    service: &Arc<Service>,
    addr: &str,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<Ended> {
    let mut client = pb::celld_client::CelldClient::new(unix_channel(addr).await?)
        .max_decoding_message_size(MAX_MESSAGE)
        .max_encoding_message_size(MAX_MESSAGE);
    let (tx, rx) = mpsc::channel(4096);
    tx.send(pb::PluginMessage {
        kind: Some(plugin_message::Kind::Register(pb::Register {
            plugin: service.info.id().to_string(),
            version: service.info.version().to_string(),
            runtime: "rust".into(),
            depends: service.depends.clone(),
            manifest_digest: String::new(),
            token: std::env::var("CELOBUS_PLUGIN_TOKEN").unwrap_or_default(),
            job_slots: service.job_slots,
        })),
    })
    .await?;
    let mut inbound = client.session(ReceiverStream::new(rx)).await?.into_inner();
    let link = Arc::new(Link::new(tx.clone()));
    let mut draining: Option<Instant> = None;
    // the runs in flight on this session: dropped with it, which stops their handlers
    let runs: Runs = Arc::default();
    let slots = Arc::new(Semaphore::new(service.job_slots as usize));

    loop {
        if let Some(since) = draining
            && ((service.inflight.load(Ordering::Acquire) == 0
                && runs.lock().expect("runs").is_empty())
                || since.elapsed() > DRAIN)
        {
            return Ok(Ended::Stopped);
        }
        let msg = tokio::select! {
            msg = inbound.next() => msg,
            changed = stop.changed(), if draining.is_none() => {
                if changed.is_err() || *stop.borrow() {
                    draining = Some(Instant::now());
                }
                continue;
            }
            _ = tokio::time::sleep(Duration::from_millis(50)), if draining.is_some() => continue,
        };
        let Some(msg) = msg else {
            return Ok(Ended::Closed);
        };
        match msg?.kind {
            Some(celld_message::Kind::Registered(r)) => {
                tracing::info!(plugin = service.info.id(), instance = %r.instance_id, "registered with the platform");
                service
                    .info
                    .settings_cache()
                    .update(r.settings_rev, &r.settings, true);
            }
            Some(celld_message::Kind::SettingsChanged(c)) => {
                if service.info.settings_cache().update(c.rev, &c.json, false) {
                    tracing::info!(rev = c.rev, "the settings changed");
                }
            }
            Some(celld_message::Kind::Request(req)) => {
                let busy =
                    service.inflight.fetch_add(1, Ordering::AcqRel) >= MAX_ADMITTED + MAX_QUEUED;
                let (service, link, tx) = (service.clone(), link.clone(), tx.clone());
                if busy || draining.is_some() {
                    service.inflight.fetch_sub(1, Ordering::AcqRel);
                    let why = if busy {
                        "the plugin is busy"
                    } else {
                        "the plugin is stopping"
                    };
                    let mut resp = error_response(&Error::unavailable(why));
                    resp.id = req.id;
                    resp.headers.push(header("retry-after", "1"));
                    tokio::spawn(async move {
                        let _ = tx.send(response(resp)).await;
                    });
                    continue;
                }
                tokio::spawn(async move {
                    let permit = service.permits.clone().acquire_owned().await;
                    let resp = service.dispatch(link, req).await;
                    drop(permit);
                    let _ = tx.send(response(resp)).await;
                    service.inflight.fetch_sub(1, Ordering::AcqRel);
                });
            }
            Some(celld_message::Kind::SyscallResult(r)) => link.complete(r),
            Some(celld_message::Kind::JobRun(run)) => service.start_run(
                run,
                link.clone(),
                tx.clone(),
                &runs,
                &slots,
                draining.is_some(),
            ),
            Some(celld_message::Kind::JobAbort(abort)) => {
                let stop = runs.lock().expect("runs").remove(&abort.run_id);
                if let Some(stop) = stop {
                    let _ = stop.send(abort.reason);
                }
            }
            None => {}
        }
    }
}

fn response(resp: pb::Response) -> pb::PluginMessage {
    pb::PluginMessage {
        kind: Some(plugin_message::Kind::Response(resp)),
    }
}

/// The channel to the platform at `CELOBUS_ADDR`: `unix:/path` or `unix:///path`.
async fn unix_channel(addr: &str) -> anyhow::Result<tonic::transport::Channel> {
    let path = addr
        .strip_prefix("unix://")
        .or_else(|| addr.strip_prefix("unix:"))
        .ok_or_else(|| anyhow::anyhow!("CELOBUS_ADDR must be unix:<path>, got {addr}"))?
        .to_string();
    // the URI is not used: the connector opens the socket
    let channel = tonic::transport::Endpoint::from_static("http://platform")
        .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
            let path = path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
            }
        }))
        .await?;
    Ok(channel)
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

/// Dispatching a request without a session, for the tests of other modules.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    async fn dispatch(service: &Service, path: &str, body: &Value, member: bool) -> (u32, Value) {
        let req = pb::Request {
            id: 1,
            method: "POST".into(),
            path: path.into(),
            headers: vec![header("content-type", "application/json")],
            body: Bytes::from(body.to_string()),
            principal: member.then(|| pb::Principal {
                user_id: "u-1".into(),
                kind: pb::principal::Kind::Person as i32,
                ..Default::default()
            }),
            ..Default::default()
        };
        let resp = service.dispatch(Arc::new(Link::detached()), req).await;
        (
            resp.status,
            serde_json::from_slice(&resp.body).unwrap_or(Value::Null),
        )
    }

    /// A request of the platform itself (no caller).
    pub(crate) async fn dispatch_system(
        service: &Service,
        path: &str,
        body: &Value,
    ) -> (u32, Value) {
        dispatch(service, path, body, false).await
    }

    /// A request of a member.
    pub(crate) async fn dispatch_member(
        service: &Service,
        path: &str,
        body: &Value,
    ) -> (u32, Value) {
        dispatch(service, path, body, true).await
    }
}
