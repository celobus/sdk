//! `ctx.files()`: the bytes of the plugin's files.
//!
//! A [`File`] names a file, never holds its bytes. A plugin reads the bytes of a file it holds (one it was given in a
//! request, a call's answer, a job payload or a row, or one it made) with `open`, a stream that never loads the file
//! into memory, and makes a new file with `create`, a writer that sends the bytes as they are written. The bytes move
//! directly between the plugin and the workspace's object store over short-lived signed URLs.
//!
//! ```no_run
//! # use celobus_sdk::prelude::*;
//! # #[derive(Message, Clone, Debug)] #[message(job)] pub struct ImportCsv { pub file: File }
//! # #[derive(Message, Clone, Debug)] pub struct ExportReply { pub file: File }
//! async fn import(ctx: Ctx, job: ImportCsv) -> Result<()> {
//!     let mut lines = ctx.files().open(&job.file).await?.lines();
//!     while let Some(line) = lines.next_line().await? {
//!         tracing::info!("{line}");
//!     }
//!     Ok(())
//! }
//!
//! async fn export(ctx: Ctx) -> Result<ExportReply> {
//!     let mut w = ctx.files().create("notes.csv", "text/csv").await?;
//!     w.write_all(b"id,text\n").await?;
//!     Ok(ExportReply { file: w.finish().await? })
//! }
//! ```
//!
//! A file made by `create` lives while a row, a pending job, a schedule or a live agent run of the plugin holds it,
//! and for 24 hours after it was made (time for a caller to download an export). There is no delete: a file nothing
//! holds is collected.
//!
//! Errors: 404 `not_found` (not a file of this plugin), 410 `file_gone` (it was collected), 413 `file_too_large`,
//! 507 `quota_exceeded` (the workspace's storage is full: the next write fails with it), 409 `file_size_mismatch` (the
//! bytes written are not the size declared with `.size(n)`), 409 `read_only` (`create` in a read-only endpoint), 400
//! `bad_request` (an empty or invalid name), 503 `unavailable` (the store cannot be reached).
//!
//! In a test context ([`Ctx::test`]) files live in memory: `create` and `open` work without the platform.

use std::collections::HashMap;
use std::future::IntoFuture;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, ready};
use std::time::Duration;

use bytes::{Buf, Bytes};
use celobus_proto as pb;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt};
use pb::{syscall, syscall_result};
use tokio::io::{AsyncBufRead, AsyncRead, AsyncWrite, ReadBuf};

use crate::ctx::Ctx;
use crate::error::{Error, Result};
use crate::platform::User;
use crate::types::{File, Id, Timestamp};

/// How often a read or a part is tried again without progress before it fails.
const TRIES: u32 = 3;
/// A signed URL this close to its expiry is asked for again rather than used.
const EXPIRY_MARGIN_MS: i64 = 30_000;

/// What is known of one of the plugin's files (`ctx.files().head(&file)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub file: File,
    /// The store's tag of the bytes: it changes only with the bytes (a new file).
    pub etag: String,
    pub created_at: Option<Timestamp>,
    /// Who uploaded or made it; `None`: the plugin itself.
    pub created_by: Option<Id<User>>,
}

/// The plugin's files: `open` one to read it, `create` a new one, `head` what is known of one.
pub struct Files {
    ctx: Ctx,
}

impl Files {
    pub(crate) fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// Opens a file of the plugin for reading: an `AsyncRead` and `AsyncBufRead` of its bytes (`.lines()` for text).
    /// The bytes stream as they are read; a dropped connection or an expired URL is resumed where reading stopped.
    /// 404 `not_found` when it is not a file of this plugin, 410 `file_gone` when it was collected.
    pub async fn open(&self, file: &File) -> Result<FileReader> {
        match self.ctx.files_backend() {
            Backend::Memory(mem) => {
                let (file, bytes) = mem.get(&file.id)?;
                Ok(FileReader::memory(file, bytes))
            }
            Backend::Http(transport) => {
                let opener = Opener {
                    ctx: self.ctx.clone(),
                    transport,
                    id: file.id.clone(),
                };
                let opened = opener.clone().open(0, None, None).await?;
                Ok(FileReader::remote(opener, opened))
            }
        }
    }

    /// Starts a new file of the plugin named `name` (`report.csv`) of the MIME type `content_type` (`text/csv`;
    /// empty: `application/octet-stream`). `.size(n)` declares its size when known (the space is reserved at
    /// once, and writing more or fewer bytes fails); `.await` gives the [`FileWriter`]. Inside a request the file is
    /// made for its caller, who may download it while no record holds it (an export). 409 `read_only` in a read-only
    /// endpoint, 400 `bad_request` for an empty name.
    pub fn create(&self, name: impl Into<String>, content_type: impl Into<String>) -> Create {
        Create {
            ctx: self.ctx.clone(),
            name: name.into(),
            content_type: content_type.into(),
            size: None,
        }
    }

    /// What is known of a file of the plugin: its name, type and size as stored, its tag, when and by whom it was
    /// made. 404 `not_found`, 410 `file_gone` as `open`.
    pub async fn head(&self, file: &File) -> Result<FileInfo> {
        if let Backend::Memory(mem) = self.ctx.files_backend() {
            let (file, bytes) = mem.get(&file.id)?;
            return Ok(FileInfo {
                etag: format!("\"{:x}\"", bytes.len()),
                file,
                created_at: Some(chrono::Utc::now()),
                created_by: None,
            });
        }
        let call = syscall::Call::FilesHead(pb::FilesHead {
            id: file.id.clone(),
            request_id: self.ctx.request().id(),
        });
        let Some(syscall_result::Result::FileInfo(info)) = self.ctx.syscall(call).await? else {
            return Err(unexpected("files_head"));
        };
        Ok(FileInfo {
            file: info.file.map(file_of).unwrap_or_else(|| file.clone()),
            etag: info.etag,
            created_at: chrono::DateTime::from_timestamp_millis(info.created_at_ms)
                .filter(|_| info.created_at_ms > 0),
            created_by: (!info.created_by.is_empty()).then(|| Id::new(info.created_by)),
        })
    }
}

fn unexpected(what: &str) -> Error {
    Error::internal(format!("the platform answered {what} with something else"))
}

fn file_of(f: pb::File) -> File {
    File {
        id: f.id,
        name: f.name,
        content_type: f.content_type,
        size: f.size,
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// An SDK error carried through `std::io` (`AsyncRead`, `AsyncWrite`): `?` on it gives the error back.
pub(crate) fn to_io(e: Error) -> io::Error {
    io::Error::other(e)
}

// --- the store's HTTP ------------------------------------------------------------------------------

/// A GET's answer: its status and, on success, its body.
pub(crate) struct Fetched {
    pub status: u16,
    pub body: BoxStream<'static, io::Result<Bytes>>,
}

/// A part PUT's answer: its status and the part's ETag.
pub(crate) struct Put {
    pub status: u16,
    pub etag: Option<String>,
}

/// How the SDK speaks to the store's signed URLs (an HTTP client; a fake in tests).
pub(crate) trait Transport: Send + Sync {
    /// GET `url` from byte `from` (a `Range` when `from > 0`).
    fn get(&self, url: String, from: u64) -> BoxFuture<'static, io::Result<Fetched>>;
    /// PUT `body` to `url` with its exact `Content-Length`.
    fn put(&self, url: String, body: Bytes) -> BoxFuture<'static, io::Result<Put>>;
}

pub(crate) struct Http(pub(crate) reqwest::Client);

impl Transport for Http {
    fn get(&self, url: String, from: u64) -> BoxFuture<'static, io::Result<Fetched>> {
        let mut req = self.0.get(url);
        if from > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={from}-"));
        }
        async move {
            let resp = req.send().await.map_err(io::Error::other)?;
            let status = resp.status().as_u16();
            let body = futures::stream::unfold(Some(resp), |resp| async move {
                let mut resp = resp?;
                match resp.chunk().await {
                    Ok(Some(chunk)) => Some((Ok(chunk), Some(resp))),
                    Ok(None) => None,
                    Err(e) => Some((Err(io::Error::other(e)), None)),
                }
            })
            .boxed();
            Ok(Fetched { status, body })
        }
        .boxed()
    }

    fn put(&self, url: String, body: Bytes) -> BoxFuture<'static, io::Result<Put>> {
        let req = self
            .0
            .put(url)
            .header(reqwest::header::CONTENT_LENGTH, body.len())
            .body(body);
        async move {
            let resp = req.send().await.map_err(io::Error::other)?;
            let etag = resp
                .headers()
                .get(reqwest::header::ETAG)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            Ok(Put {
                status: resp.status().as_u16(),
                etag,
            })
        }
        .boxed()
    }
}

fn http() -> Arc<dyn Transport> {
    static HTTP: OnceLock<Arc<dyn Transport>> = OnceLock::new();
    HTTP.get_or_init(|| {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Arc::new(Http(client))
    })
    .clone()
}

/// Where a context's files go: the store over HTTP, or memory in a test context.
#[derive(Clone, Default)]
pub(crate) enum Store {
    #[default]
    Http,
    /// another client of the store (the SDK's tests)
    #[cfg_attr(not(test), allow(dead_code))]
    Transport(Arc<dyn Transport>),
    Memory(Arc<Memory>),
}

pub(crate) enum Backend {
    Http(Arc<dyn Transport>),
    Memory(Arc<Memory>),
}

impl Store {
    pub(crate) fn backend(&self) -> Backend {
        match self {
            Store::Http => Backend::Http(http()),
            Store::Transport(t) => Backend::Http(t.clone()),
            Store::Memory(m) => Backend::Memory(m.clone()),
        }
    }
}

/// The files of a test context.
#[derive(Default)]
pub(crate) struct Memory {
    files: Mutex<HashMap<String, (File, Bytes)>>,
    next: AtomicU64,
}

impl Memory {
    fn get(&self, id: &str) -> Result<(File, Bytes)> {
        self.files
            .lock()
            .expect("test files")
            .get(id)
            .cloned()
            .ok_or_else(|| Error::not_found(format!("no file {id} of this plugin")))
    }

    fn put(&self, name: String, content_type: String, bytes: Bytes) -> File {
        let n = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let file = File {
            id: format!("test-file-{n}"),
            name,
            content_type: if content_type.is_empty() {
                "application/octet-stream".into()
            } else {
                content_type
            },
            size: bytes.len() as u64,
        };
        self.files
            .lock()
            .expect("test files")
            .insert(file.id.clone(), (file.clone(), bytes));
        file
    }
}

// --- reading ---------------------------------------------------------------------------------------

/// Opens (and re-opens) one file for reading from a position.
#[derive(Clone)]
struct Opener {
    ctx: Ctx,
    transport: Arc<dyn Transport>,
    id: String,
}

/// A GET under way: its body, the bytes to drop from its start (a store that answered a ranged GET whole), the URL
/// and the file as the platform knows it.
struct Opened {
    file: File,
    body: BoxStream<'static, io::Result<Bytes>>,
    skip: u64,
    url: Option<(String, i64)>,
}

impl Opener {
    /// A GET of the file from `pos`, with the URL in hand while it is valid, else a new one.
    async fn open(
        self,
        pos: u64,
        url: Option<(String, i64)>,
        known: Option<File>,
    ) -> Result<Opened> {
        let mut url = url.filter(|(_, exp)| *exp - EXPIRY_MARGIN_MS > now_ms());
        let mut file = known;
        let mut last = None;
        for _ in 0..TRIES {
            let (u, expires) = match url.take() {
                Some(u) => u,
                None => {
                    let call = syscall::Call::FilesOpen(pb::FilesOpen {
                        id: self.id.clone(),
                        offset: (pos > 0).then_some(pos),
                        request_id: self.ctx.request().id(),
                    });
                    let Some(syscall_result::Result::FilesRead(r)) = self.ctx.syscall(call).await?
                    else {
                        return Err(unexpected("files_open"));
                    };
                    file = r.file.map(file_of);
                    (r.url, r.expires_at_ms)
                }
            };
            let file = file.clone().unwrap_or_else(|| File {
                id: self.id.clone(),
                ..Default::default()
            });
            if file.size > 0 && pos >= file.size {
                return Ok(Opened {
                    file,
                    body: futures::stream::empty().boxed(),
                    skip: 0,
                    url: Some((u, expires)),
                });
            }
            match self.transport.get(u.clone(), pos).await {
                Ok(f) if f.status == 206 || f.status == 200 => {
                    return Ok(Opened {
                        file,
                        body: f.body,
                        skip: if f.status == 200 { pos } else { 0 },
                        url: Some((u, expires)),
                    });
                }
                // past the end: nothing more to read
                Ok(f) if f.status == 416 => {
                    return Ok(Opened {
                        file,
                        body: futures::stream::empty().boxed(),
                        skip: 0,
                        url: Some((u, expires)),
                    });
                }
                Ok(f) if f.status == 404 => {
                    return Err(Error::new(
                        410,
                        "file_gone",
                        format!("the file {} was collected", self.id),
                    ));
                }
                // the URL expired: ask for a new one
                Ok(f) => last = Some(format!("the store answered {}", f.status)),
                Err(e) => last = Some(e.to_string()),
            }
        }
        Err(Error::unavailable(format!(
            "reading the file {}: {}",
            self.id,
            last.unwrap_or_default()
        )))
    }
}

enum ReadState {
    Body(BoxStream<'static, io::Result<Bytes>>),
    Opening(BoxFuture<'static, Result<Opened>>),
    Eof,
    /// reading failed: every later read fails the same way (never a short file read as whole)
    Failed(Error),
}

/// A file being read (`ctx.files().open(&file)`): `AsyncRead` and `AsyncBufRead` of its bytes, from the first.
/// `.lines()` reads it as lines of text; `tokio::io::AsyncReadExt` (in the prelude) gives `read_to_end`, `read_exact`
/// and the rest. A dropped connection or an expired URL is resumed where reading stopped.
pub struct FileReader {
    file: File,
    opener: Option<Opener>,
    state: ReadState,
    chunk: Bytes,
    pos: u64,
    skip: u64,
    url: Option<(String, i64)>,
    /// re-opens since the last byte read
    stalls: u32,
}

impl FileReader {
    fn memory(file: File, bytes: Bytes) -> Self {
        Self {
            file,
            opener: None,
            state: ReadState::Eof,
            chunk: bytes,
            pos: 0,
            skip: 0,
            url: None,
            stalls: 0,
        }
    }

    fn remote(opener: Opener, opened: Opened) -> Self {
        Self {
            file: opened.file,
            opener: Some(opener),
            state: ReadState::Body(opened.body),
            chunk: Bytes::new(),
            pos: 0,
            skip: opened.skip,
            url: opened.url,
            stalls: 0,
        }
    }

    /// The file as stored: its name, type and size.
    pub fn file(&self) -> &File {
        &self.file
    }

    /// The bytes read so far.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// The file as lines of text (`while let Some(line) = lines.next_line().await? {..}`), without their `\n` or
    /// `\r\n`; a line that is not UTF-8 fails the read.
    pub fn lines(self) -> tokio::io::Lines<FileReader> {
        tokio::io::AsyncBufReadExt::lines(self)
    }

    /// Opens again from the current position, or fails when that has been tried too often without progress.
    fn reopen(&mut self, why: io::Error) -> io::Result<()> {
        let Some(opener) = self.opener.clone() else {
            return Err(why);
        };
        self.stalls += 1;
        if self.stalls > TRIES {
            let e = Error::unavailable(format!(
                "reading the file {} stopped at byte {}: {why}",
                self.file.id, self.pos
            ));
            self.state = ReadState::Failed(e.clone());
            return Err(to_io(e));
        }
        let url = self.url.take();
        let known = Some(self.file.clone());
        self.state = ReadState::Opening(opener.open(self.pos, url, known).boxed());
        Ok(())
    }
}

impl AsyncBufRead for FileReader {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        let this = self.get_mut();
        loop {
            if !this.chunk.is_empty() {
                return Poll::Ready(Ok(&this.chunk[..]));
            }
            match &mut this.state {
                ReadState::Eof => return Poll::Ready(Ok(&[])),
                ReadState::Failed(e) => return Poll::Ready(Err(to_io(e.clone()))),
                ReadState::Opening(f) => match ready!(f.poll_unpin(cx)) {
                    Ok(opened) => {
                        this.skip = opened.skip;
                        this.url = opened.url;
                        this.state = ReadState::Body(opened.body);
                    }
                    Err(e) => {
                        this.state = ReadState::Failed(e.clone());
                        return Poll::Ready(Err(to_io(e)));
                    }
                },
                ReadState::Body(body) => match ready!(body.poll_next_unpin(cx)) {
                    Some(Ok(mut b)) => {
                        let drop = this.skip.min(b.len() as u64);
                        b.advance(drop as usize);
                        this.skip -= drop;
                        this.chunk = b;
                    }
                    Some(Err(e)) => this.reopen(e)?,
                    None if this.opener.is_some() && this.pos < this.file.size => this.reopen(
                        io::Error::new(io::ErrorKind::UnexpectedEof, "the connection ended early"),
                    )?,
                    None => this.state = ReadState::Eof,
                },
            }
        }
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        let this = self.get_mut();
        let amt = amt.min(this.chunk.len());
        this.chunk.advance(amt);
        this.pos += amt as u64;
        if amt > 0 {
            this.stalls = 0;
        }
    }
}

impl AsyncRead for FileReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let have = ready!(self.as_mut().poll_fill_buf(cx))?;
        let n = have.len().min(buf.remaining());
        buf.put_slice(&have[..n]);
        self.consume(n);
        Poll::Ready(Ok(()))
    }
}

// --- writing ---------------------------------------------------------------------------------------

/// A new file being started (`ctx.files().create(name, type)`): `.size(n)` declares its size; `.await` starts it.
pub struct Create {
    ctx: Ctx,
    name: String,
    content_type: String,
    size: Option<u64>,
}

impl Create {
    /// The file's size in bytes, when known: the space is reserved at once (507 `quota_exceeded` here rather than
    /// while writing), and `finish` fails with 409 `file_size_mismatch` when another number of bytes was written.
    pub fn size(mut self, n: u64) -> Self {
        self.size = Some(n);
        self
    }
}

impl IntoFuture for Create {
    type Output = Result<FileWriter>;
    type IntoFuture = BoxFuture<'static, Result<FileWriter>>;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let transport = match self.ctx.files_backend() {
                Backend::Memory(mem) => {
                    return Ok(FileWriter::memory(
                        mem,
                        self.name,
                        self.content_type,
                        self.size,
                    ));
                }
                Backend::Http(t) => t,
            };
            let call = syscall::Call::FilesCreate(pb::FilesCreate {
                name: self.name,
                content_type: self.content_type,
                size: self.size,
                request_id: self.ctx.request().id(),
            });
            let Some(syscall_result::Result::FilesUpload(page)) = self.ctx.syscall(call).await?
            else {
                return Err(unexpected("files_create"));
            };
            if page.part_size == 0 {
                return Err(Error::internal(
                    "the platform answered files_create without a part size",
                ));
            }
            Ok(FileWriter::upload(self.ctx, transport, page, self.size))
        }
        .boxed()
    }
}

/// The part URLs in hand.
#[derive(Clone)]
struct Page(pb::FilesUpload);

impl Page {
    fn url(&self, part: u32) -> Option<String> {
        let p = &self.0;
        if part < p.first || p.expires_at_ms - EXPIRY_MARGIN_MS <= now_ms() {
            return None;
        }
        p.urls.get((part - p.first) as usize).cloned()
    }
}

/// Sends part `n`: its URL from the page in hand, else the next page; a refused (expired) URL is asked for again.
async fn send_part(
    ctx: Ctx,
    transport: Arc<dyn Transport>,
    mut page: Page,
    n: u32,
    body: Bytes,
) -> (Page, Result<Option<String>>) {
    let mut last = String::new();
    for _ in 0..TRIES {
        let url = match page.url(n) {
            Some(u) => u,
            None => {
                let call = syscall::Call::FilesParts(pb::FilesParts {
                    id: page.0.id.clone(),
                    from: n,
                    request_id: ctx.request().id(),
                });
                match ctx.syscall(call).await {
                    Ok(Some(syscall_result::Result::FilesUpload(next))) => {
                        page = Page(pb::FilesUpload {
                            part_size: page.0.part_size,
                            ..next
                        })
                    }
                    Ok(_) => return (page, Err(unexpected("files_parts"))),
                    Err(e) => return (page, Err(e)),
                }
                match page.url(n) {
                    Some(u) => u,
                    None => {
                        return (
                            page,
                            Err(Error::internal(format!(
                                "the platform gave no URL for part {n}"
                            ))),
                        );
                    }
                }
            }
        };
        match transport.put(url, body.clone()).await {
            Ok(p) if (200..300).contains(&p.status) => return (page, Ok(p.etag)),
            // an expired URL: a new page next time
            Ok(p) if p.status == 403 => {
                page.0.expires_at_ms = 0;
                last = "the store refused the URL (403)".into();
            }
            Ok(p) => {
                let e = Error::unavailable(format!(
                    "the store refused part {n} of the file {}: {}",
                    page.0.id, p.status
                ));
                return (page, Err(e));
            }
            Err(e) => last = e.to_string(),
        }
    }
    let id = page.0.id.clone();
    (
        page,
        Err(Error::unavailable(format!(
            "writing part {n} of the file {id}: {last}"
        ))),
    )
}

type PartFuture = BoxFuture<'static, (Page, Result<Option<String>>)>;

struct Upload {
    ctx: Ctx,
    transport: Arc<dyn Transport>,
    /// the part URLs in hand (none while a part is being sent)
    page: Option<Page>,
    part_size: usize,
    /// the next part's number (from 1)
    next: u32,
    etags: Vec<Option<String>>,
    sending: Option<PartFuture>,
}

impl Upload {
    fn start_part(&mut self, body: Bytes) {
        let page = self.page.take().expect("a page when no part is being sent");
        let n = self.next;
        self.next += 1;
        self.sending =
            Some(send_part(self.ctx.clone(), self.transport.clone(), page, n, body).boxed());
    }

    /// Waits for the part being sent.
    fn poll_sent(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let Some(f) = self.sending.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        let (page, res) = ready!(f.poll_unpin(cx));
        self.sending = None;
        self.page = Some(page);
        Poll::Ready(res.map(|etag| self.etags.push(etag)))
    }
}

enum Sink {
    Upload(Upload),
    Memory(Arc<Memory>),
}

/// A new file being written (`ctx.files().create(name, type).await?`): an `AsyncWrite` (`write_all` from
/// `tokio::io::AsyncWriteExt`, in the prelude). The bytes are sent in parts as they are written, one part held in
/// memory at a time; `finish()` ends the file and gives its [`File`]. An error (507 `quota_exceeded` when the
/// workspace's storage fills up) fails the next write and `finish`. A writer dropped without `finish` leaves no
/// file.
pub struct FileWriter {
    name: String,
    content_type: String,
    declared: Option<u64>,
    sink: Sink,
    buf: Vec<u8>,
    written: u64,
    failed: Option<Error>,
}

impl FileWriter {
    fn memory(mem: Arc<Memory>, name: String, content_type: String, size: Option<u64>) -> Self {
        Self {
            name,
            content_type,
            declared: size,
            sink: Sink::Memory(mem),
            buf: Vec::new(),
            written: 0,
            failed: None,
        }
    }

    fn upload(
        ctx: Ctx,
        transport: Arc<dyn Transport>,
        page: pb::FilesUpload,
        size: Option<u64>,
    ) -> Self {
        let part_size = usize::try_from(page.part_size).unwrap_or(usize::MAX);
        Self {
            name: String::new(),
            content_type: String::new(),
            declared: size,
            buf: Vec::with_capacity(part_size.min(16 << 20)),
            sink: Sink::Upload(Upload {
                ctx,
                transport,
                page: Some(Page(page)),
                part_size,
                next: 1,
                etags: Vec::new(),
                sending: None,
            }),
            written: 0,
            failed: None,
        }
    }

    /// The bytes written so far.
    pub fn written(&self) -> u64 {
        self.written
    }

    fn fail(&mut self, e: Error) -> io::Error {
        self.failed = Some(e.clone());
        to_io(e)
    }

    /// Ends the file: sends what is left, and answers the new [`File`] (put it in a row, a job, an answer). 409
    /// `file_size_mismatch` when a size was declared and another number of bytes was written.
    pub async fn finish(mut self) -> Result<File> {
        if let Some(e) = self.failed.take() {
            return Err(e);
        }
        if let Some(size) = self.declared
            && size != self.written
        {
            return Err(Error::new(
                409,
                "file_size_mismatch",
                format!(
                    "{} bytes were written to a file declared as {size} bytes",
                    self.written
                ),
            ));
        }
        let buf = std::mem::take(&mut self.buf);
        let up = match &mut self.sink {
            Sink::Memory(mem) => {
                return Ok(mem.put(
                    std::mem::take(&mut self.name),
                    std::mem::take(&mut self.content_type),
                    Bytes::from(buf),
                ));
            }
            Sink::Upload(up) => up,
        };
        std::future::poll_fn(|cx| up.poll_sent(cx)).await?;
        // the last part, or the only one (an empty file is one empty part)
        if !buf.is_empty() || up.next == 1 {
            up.start_part(Bytes::from(buf));
            std::future::poll_fn(|cx| up.poll_sent(cx)).await?;
        }
        let page = up.page.as_ref().expect("a page after the last part");
        let etags = if up.etags.iter().all(Option::is_some) {
            up.etags.iter().flatten().cloned().collect()
        } else {
            vec![]
        };
        let call = syscall::Call::FilesFinish(pb::FilesFinish {
            id: page.0.id.clone(),
            etags,
            request_id: up.ctx.request().id(),
        });
        match up.ctx.syscall(call).await? {
            Some(syscall_result::Result::File(f)) => Ok(file_of(f)),
            _ => Err(unexpected("files_finish")),
        }
    }
}

impl AsyncWrite for FileWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if let Some(e) = &this.failed {
            return Poll::Ready(Err(to_io(e.clone())));
        }
        if let Some(size) = this.declared
            && this.written + data.len() as u64 > size
        {
            let e = Error::new(
                409,
                "file_size_mismatch",
                format!("the file was declared as {size} bytes and more are written"),
            );
            return Poll::Ready(Err(this.fail(e)));
        }
        let room = match &mut this.sink {
            Sink::Memory(_) => data.len(),
            Sink::Upload(up) => loop {
                if let Err(e) = ready!(up.poll_sent(cx)) {
                    return Poll::Ready(Err(this.fail(e)));
                }
                if this.buf.len() < up.part_size {
                    break up.part_size - this.buf.len();
                }
                up.start_part(Bytes::from(std::mem::take(&mut this.buf)));
            },
        };
        let n = room.min(data.len());
        this.buf.extend_from_slice(&data[..n]);
        this.written += n as u64;
        Poll::Ready(Ok(n))
    }

    /// Waits for the part being sent; the bytes of a part not yet full stay until it is (or until `finish`).
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Sink::Upload(up) = &mut this.sink
            && let Err(e) = ready!(up.poll_sent(cx))
        {
            return Poll::Ready(Err(this.fail(e)));
        }
        Poll::Ready(Ok(()))
    }

    /// As `poll_flush`: the file ends with `finish()`.
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}
