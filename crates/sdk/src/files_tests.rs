//! `ctx.files()` against a fake platform and a fake store: the writer's parts, pages, finish and refusals; the
//! reader's resume after a dropped connection and an expired URL; the in-memory files of a test context.

use std::io;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use celobus_proto as pb;
use futures::FutureExt;
use futures::future::BoxFuture;
use futures::stream::{self, StreamExt};
use pb::{syscall, syscall_result};

use crate::db_tests::{Answer, ctx_with};
use crate::files::{Fetched, Put, Store, Transport};
use crate::prelude::*;

fn far() -> i64 {
    chrono::Utc::now().timestamp_millis() + 3_600_000
}

/// A page of part URLs `first..first+n` of upload `u-1`.
fn page(part_size: u64, parts: u32, first: u32, n: u32) -> syscall_result::Result {
    syscall_result::Result::FilesUpload(pb::FilesUpload {
        id: "u-1".into(),
        part_size,
        parts,
        first,
        urls: (first..first + n)
            .map(|i| format!("https://store/put/{i}"))
            .collect(),
        done: vec![],
        expires_at_ms: far(),
    })
}

fn pb_file(size: u64) -> pb::File {
    pb::File {
        id: "u-1".into(),
        name: "out.csv".into(),
        content_type: "text/csv".into(),
        size,
    }
}

/// The store: what was PUT where, and how GETs answer.
#[derive(Default)]
struct FakeStore {
    puts: Mutex<Vec<(String, Bytes)>>,
    /// urls whose PUT answers 403 once (expired)
    expired: Mutex<Vec<String>>,
    /// the object a GET reads
    object: Bytes,
    /// GETs seen: (url, from)
    gets: Mutex<Vec<(String, u64)>>,
    /// a GET of a url ending with this answers 403
    refuse_get: Mutex<Vec<String>>,
    /// a GET from 0 drops the connection after this many bytes
    drop_after: Option<usize>,
    /// a GET from 0 ends cleanly after this many bytes (short of the size)
    short: Option<usize>,
    /// a ranged GET is answered whole (200, from byte 0)
    whole: bool,
    /// every GET fails (the connection is refused)
    down: bool,
}

impl Transport for FakeStore {
    fn get(&self, url: String, from: u64) -> BoxFuture<'static, io::Result<Fetched>> {
        self.gets.lock().unwrap().push((url.clone(), from));
        let refused = {
            let mut r = self.refuse_get.lock().unwrap();
            let hit = r.iter().position(|u| *u == url);
            hit.map(|i| r.remove(i)).is_some()
        };
        let start = if self.whole { 0 } else { from as usize };
        let rest = self.object.slice(start..);
        let drop_after = self.drop_after.filter(|_| from == 0);
        let short = self.short.filter(|_| from == 0);
        let (whole, down) = (self.whole, self.down);
        async move {
            if down {
                return Err(io::Error::new(io::ErrorKind::ConnectionRefused, "refused"));
            }
            if refused {
                return Ok(Fetched {
                    status: 403,
                    body: stream::empty().boxed(),
                });
            }
            let body = match drop_after {
                Some(n) => stream::iter([
                    Ok(rest.slice(..n)),
                    Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset")),
                ])
                .boxed(),
                None if short.is_some() => stream::iter([Ok(rest.slice(..short.unwrap()))]).boxed(),
                None => stream::iter(
                    rest.chunks(3)
                        .map(|c| Ok(Bytes::copy_from_slice(c)))
                        .collect::<Vec<_>>(),
                )
                .boxed(),
            };
            Ok(Fetched {
                status: if from > 0 && !whole { 206 } else { 200 },
                body,
            })
        }
        .boxed()
    }

    fn put(&self, url: String, body: Bytes) -> BoxFuture<'static, io::Result<Put>> {
        let expired = {
            let mut e = self.expired.lock().unwrap();
            let hit = e.iter().position(|u| *u == url);
            hit.map(|i| e.remove(i)).is_some()
        };
        if !expired {
            self.puts.lock().unwrap().push((url.clone(), body));
        }
        async move {
            Ok(if expired {
                Put {
                    status: 403,
                    etag: None,
                }
            } else {
                Put {
                    status: 200,
                    etag: Some(format!("\"e-{}\"", url.rsplit('/').next().unwrap())),
                }
            })
        }
        .boxed()
    }
}

fn with_store(
    store: Arc<FakeStore>,
    answer: impl Fn(&syscall::Call) -> Answer + Send + 'static,
) -> (Ctx, Arc<Mutex<Vec<syscall::Call>>>) {
    let (ctx, seen) = ctx_with(answer);
    (ctx.with_files(Store::Transport(store)), seen)
}

#[tokio::test]
async fn a_writer_sends_full_parts_asks_for_the_next_page_and_finishes_with_the_etags() {
    let store = Arc::new(FakeStore::default());
    let (ctx, seen) = with_store(store.clone(), |call| match call {
        // a part size of 4 bytes, two URLs a page
        syscall::Call::FilesCreate(_) => Ok(page(4, 0, 1, 2)),
        syscall::Call::FilesParts(p) => Ok(page(4, 0, p.from, 2)),
        syscall::Call::FilesFinish(_) => Ok(syscall_result::Result::File(pb_file(12))),
        _ => Err(("unexpected".into(), None)),
    });
    let mut w = ctx.files().create("out.csv", "text/csv").await.unwrap();
    w.write_all(b"id,t").await.unwrap();
    w.write_all(b"ext\n1,").await.unwrap();
    w.write_all(b"a\n").await.unwrap();
    assert_eq!(w.written(), 12);
    let file = w.finish().await.unwrap();
    assert_eq!(
        (file.id.as_str(), file.name.as_str(), file.size),
        ("u-1", "out.csv", 12)
    );
    let puts = store.puts.lock().unwrap().clone();
    assert_eq!(
        puts,
        vec![
            (
                "https://store/put/1".to_string(),
                Bytes::from_static(b"id,t")
            ),
            (
                "https://store/put/2".to_string(),
                Bytes::from_static(b"ext\n")
            ),
            (
                "https://store/put/3".to_string(),
                Bytes::from_static(b"1,a\n")
            ),
        ]
    );
    let seen = seen.lock().unwrap();
    let syscall::Call::FilesCreate(c) = &seen[0] else {
        panic!("{seen:?}")
    };
    assert_eq!(
        (
            c.name.as_str(),
            c.content_type.as_str(),
            c.size,
            c.request_id
        ),
        ("out.csv", "text/csv", None, 77)
    );
    // part 3 was past the first page: one page request from it
    let pages: Vec<u32> = seen
        .iter()
        .filter_map(|c| match c {
            syscall::Call::FilesParts(p) => Some(p.from),
            _ => None,
        })
        .collect();
    assert_eq!(pages, vec![3]);
    let syscall::Call::FilesFinish(f) = seen.last().unwrap() else {
        panic!("{seen:?}")
    };
    assert_eq!(f.id, "u-1");
    assert_eq!(f.etags, vec!["\"e-1\"", "\"e-2\"", "\"e-3\""]);
}

#[tokio::test]
async fn a_declared_size_is_sent_and_checked() {
    let store = Arc::new(FakeStore::default());
    let (ctx, seen) = with_store(store.clone(), |call| match call {
        syscall::Call::FilesCreate(_) => Ok(page(4, 2, 1, 2)),
        syscall::Call::FilesFinish(_) => Ok(syscall_result::Result::File(pb_file(6))),
        _ => Err(("unexpected".into(), None)),
    });
    let mut w = ctx.files().create("a.bin", "").size(6).await.unwrap();
    w.write_all(b"abcdef").await.unwrap();
    // one byte more than declared fails the write
    let e: Error = w.write_all(b"g").await.unwrap_err().into();
    assert_eq!((e.status(), e.code()), (409, "file_size_mismatch"));
    let first = seen.lock().unwrap()[0].clone();
    let syscall::Call::FilesCreate(c) = &first else {
        panic!()
    };
    assert_eq!(c.size, Some(6));
    // fewer bytes than declared fail at finish
    let mut w = ctx.files().create("b.bin", "").size(6).await.unwrap();
    w.write_all(b"abc").await.unwrap();
    let e = w.finish().await.unwrap_err();
    assert_eq!((e.status(), e.code()), (409, "file_size_mismatch"));
}

#[tokio::test]
async fn an_expired_part_url_is_asked_for_again() {
    let store = Arc::new(FakeStore::default());
    store
        .expired
        .lock()
        .unwrap()
        .push("https://store/put/1".into());
    let (ctx, seen) = with_store(store.clone(), |call| match call {
        syscall::Call::FilesCreate(_) => Ok(page(4, 0, 1, 1)),
        syscall::Call::FilesParts(p) => Ok(page(4, 0, p.from, 1)),
        syscall::Call::FilesFinish(_) => Ok(syscall_result::Result::File(pb_file(2))),
        _ => Err(("unexpected".into(), None)),
    });
    let mut w = ctx.files().create("x", "").await.unwrap();
    w.write_all(b"hi").await.unwrap();
    w.finish().await.unwrap();
    assert_eq!(store.puts.lock().unwrap().len(), 1);
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, syscall::Call::FilesParts(p) if p.from == 1))
    );
}

#[tokio::test]
async fn a_full_quota_fails_the_next_write_and_finish() {
    let store = Arc::new(FakeStore::default());
    let (ctx, _) = with_store(store.clone(), |call| match call {
        syscall::Call::FilesCreate(_) => Ok(page(4, 0, 1, 1)),
        syscall::Call::FilesParts(_) => Err((
            "quota_exceeded: the workspace's storage is full".into(),
            None,
        )),
        _ => Err(("unexpected".into(), None)),
    });
    let mut w = ctx.files().create("big", "").await.unwrap();
    // part 1 goes; part 2 needs a page the platform refuses
    w.write_all(b"aaaabbbb").await.unwrap();
    let e: Error = w.write_all(b"c").await.unwrap_err().into();
    assert_eq!((e.status(), e.code()), (507, "quota_exceeded"));
    let e: Error = w.write_all(b"c").await.unwrap_err().into();
    assert_eq!(e.code(), "quota_exceeded");
    let e = w.finish().await.unwrap_err();
    assert_eq!((e.status(), e.code()), (507, "quota_exceeded"));
    assert_eq!(store.puts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn refusals_keep_their_status() {
    let (ctx, _) = with_store(Arc::default(), |call| match call {
        syscall::Call::FilesCreate(_) => Err(("read_only: a read-only endpoint".into(), None)),
        syscall::Call::FilesOpen(_) => Err(("file_gone: collected".into(), None)),
        syscall::Call::FilesHead(_) => Err(("not_found: not a file of this plugin".into(), None)),
        _ => Err(("unexpected".into(), None)),
    });
    let e = ctx.files().create("x", "").await.err().unwrap();
    assert_eq!((e.status(), e.code()), (409, "read_only"));
    let f = File {
        id: "f-1".into(),
        ..Default::default()
    };
    let e = ctx.files().open(&f).await.err().unwrap();
    assert_eq!((e.status(), e.code()), (410, "file_gone"));
    let e = ctx.files().head(&f).await.unwrap_err();
    assert_eq!((e.status(), e.code()), (404, "not_found"));
}

fn opened(url: &str, size: u64) -> syscall_result::Result {
    syscall_result::Result::FilesRead(pb::FilesRead {
        file: Some(pb::File {
            id: "f-1".into(),
            name: "ads.csv".into(),
            content_type: "text/csv".into(),
            size,
        }),
        url: url.into(),
        expires_at_ms: far(),
    })
}

#[tokio::test]
async fn a_reader_resumes_where_the_connection_dropped() {
    let text = "day,clicks\n2026-11-01,10\n2026-11-02,12\n";
    let store = Arc::new(FakeStore {
        object: Bytes::from_static(text.as_bytes()),
        drop_after: Some(15),
        ..Default::default()
    });
    let size = text.len() as u64;
    let (ctx, seen) = with_store(store.clone(), move |call| match call {
        syscall::Call::FilesOpen(o) => Ok(opened(
            &format!("https://store/get/{}", o.offset.unwrap_or(0)),
            size,
        )),
        _ => Err(("unexpected".into(), None)),
    });
    let f = File {
        id: "f-1".into(),
        ..Default::default()
    };
    let reader = ctx.files().open(&f).await.unwrap();
    assert_eq!(reader.file().size, size);
    let mut lines = reader.lines();
    let mut got = vec![];
    while let Some(line) = lines.next_line().await.unwrap() {
        got.push(line);
    }
    assert_eq!(got, vec!["day,clicks", "2026-11-01,10", "2026-11-02,12"]);
    // the URL in hand was still valid: the second GET used it from byte 15
    assert_eq!(
        *store.gets.lock().unwrap(),
        vec![
            ("https://store/get/0".to_string(), 0),
            ("https://store/get/0".to_string(), 15)
        ]
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_refused_url_is_opened_again_at_the_offset() {
    let text = "0123456789abcdef";
    let store = Arc::new(FakeStore {
        object: Bytes::from_static(text.as_bytes()),
        drop_after: Some(5),
        ..Default::default()
    });
    // the first URL expires while reading
    store
        .refuse_get
        .lock()
        .unwrap()
        .push("https://store/get/0".into());
    let (ctx, seen) = with_store(store.clone(), move |call| match call {
        syscall::Call::FilesOpen(o) => {
            let at = o.offset.unwrap_or(0);
            Ok(opened(&format!("https://store/get/{at}"), 16))
        }
        _ => Err(("unexpected".into(), None)),
    });
    // the first open's GET is refused (403) once, so the reader opens again before reading
    let f = File {
        id: "f-1".into(),
        ..Default::default()
    };
    let mut r = ctx.files().open(&f).await.unwrap();
    let mut out = String::new();
    r.read_to_string(&mut out).await.unwrap();
    assert_eq!(out, text);
    let offsets: Vec<Option<u64>> = seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            syscall::Call::FilesOpen(o) => Some(o.offset),
            _ => None,
        })
        .collect();
    assert_eq!(offsets, vec![None, None]);
    // now refuse the URL in hand: the drop at byte 5 re-opens at offset 5
    store
        .refuse_get
        .lock()
        .unwrap()
        .push("https://store/get/0".into());
    let mut r = ctx.files().open(&f).await.unwrap();
    store
        .refuse_get
        .lock()
        .unwrap()
        .push("https://store/get/0".into());
    let mut out = Vec::new();
    r.read_to_end(&mut out).await.unwrap();
    assert_eq!(out, text.as_bytes());
    let last = seen.lock().unwrap().last().cloned().unwrap();
    let syscall::Call::FilesOpen(o) = last else {
        panic!()
    };
    assert_eq!(o.offset, Some(5));
    assert!(
        store
            .gets
            .lock()
            .unwrap()
            .contains(&("https://store/get/5".to_string(), 5))
    );
}

#[tokio::test]
async fn a_test_context_keeps_files_in_memory() {
    let ctx = Ctx::test("notes", "1.0.0");
    let mut w = ctx.files().create("notes.csv", "text/csv").await.unwrap();
    w.write_all(b"id,text\n1,hello\n").await.unwrap();
    let file = w.finish().await.unwrap();
    assert_eq!((file.name.as_str(), file.size), ("notes.csv", 16));
    // a context made from it sees the same files
    let other = ctx.with_caller(Caller::test_user("u-1"));
    let mut lines = other.files().open(&file).await.unwrap().lines();
    assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("id,text"));
    assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("1,hello"));
    assert_eq!(lines.next_line().await.unwrap(), None);
    assert_eq!(ctx.files().head(&file).await.unwrap().file, file);
    let e = ctx
        .files()
        .open(&File {
            id: "nope".into(),
            ..Default::default()
        })
        .await
        .err()
        .unwrap();
    assert_eq!(e.status(), 404);
}

#[tokio::test]
async fn runs_and_follow_ups_carry_their_files() {
    let (ctx, seen) = ctx_with(|call| match call {
        syscall::Call::AgentRun(_) => Ok(syscall_result::Result::AgentRun(pb::AgentRun {
            id: "r-1".into(),
            ..Default::default()
        })),
        syscall::Call::AgentSend(_) => Ok(syscall_result::Result::AgentInput(pb::AgentRunInput {
            seq: 2,
        })),
        _ => Err(("unexpected".into(), None)),
    });
    let files = vec![File {
        id: "f-1".into(),
        name: "ads.csv".into(),
        content_type: "text/csv".into(),
        size: 2_100_000,
    }];
    ctx.agents()
        .run("analyst", "Compare the campaigns.")
        .files(&files)
        .await
        .unwrap();
    let seq = ctx
        .agents()
        .send("r-1", "And this one.")
        .files(&files)
        .await
        .unwrap();
    assert_eq!(seq, 2);
    ctx.agents()
        .run_with("analyst", "x", RunOptions::default().files(&files))
        .await
        .unwrap();
    let seen = seen.lock().unwrap();
    let want = vec![pb::File {
        id: "f-1".into(),
        name: "ads.csv".into(),
        content_type: "text/csv".into(),
        size: 2_100_000,
    }];
    let syscall::Call::AgentRun(r) = &seen[0] else {
        panic!()
    };
    assert_eq!(r.files, want);
    let syscall::Call::AgentSend(s) = &seen[1] else {
        panic!()
    };
    assert_eq!(s.files, want);
    let syscall::Call::AgentRun(r) = &seen[2] else {
        panic!()
    };
    assert_eq!(r.files, want);
}

fn opens(seen: &Arc<Mutex<Vec<syscall::Call>>>) -> Vec<Option<u64>> {
    seen.lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            syscall::Call::FilesOpen(o) => Some(o.offset),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_body_ending_short_is_read_on_and_a_whole_answer_to_a_range_is_skipped() {
    let text = "0123456789abcdef";
    for (short, whole) in [(Some(6), false), (Some(6), true)] {
        let store = Arc::new(FakeStore {
            object: Bytes::from_static(text.as_bytes()),
            short,
            whole,
            ..Default::default()
        });
        let (ctx, _) = with_store(store.clone(), |call| match call {
            syscall::Call::FilesOpen(_) => Ok(opened("https://store/get/0", 16)),
            _ => Err(("unexpected".into(), None)),
        });
        let f = File {
            id: "f-1".into(),
            ..Default::default()
        };
        let mut out = String::new();
        let mut r = ctx.files().open(&f).await.unwrap();
        r.read_to_string(&mut out).await.unwrap();
        assert_eq!(out, text, "whole: {whole}");
        assert_eq!(r.position(), 16);
        // the second GET asked from byte 6
        assert_eq!(store.gets.lock().unwrap()[1].1, 6);
    }
}

#[tokio::test]
async fn a_read_that_fails_keeps_failing() {
    let text = "0123456789abcdef";
    let store = Arc::new(FakeStore {
        object: Bytes::from_static(text.as_bytes()),
        drop_after: Some(4),
        ..Default::default()
    });
    let (ctx, seen) = with_store(store.clone(), |call| match call {
        syscall::Call::FilesOpen(o) if o.offset.is_none() => Ok(opened("https://store/get/0", 16)),
        syscall::Call::FilesOpen(_) => Err(("file_gone: collected".into(), None)),
        _ => Err(("unexpected".into(), None)),
    });
    let f = File {
        id: "f-1".into(),
        ..Default::default()
    };
    let mut r = ctx.files().open(&f).await.unwrap();
    // the URL in hand is refused at the drop, and opening again finds the file gone
    store
        .refuse_get
        .lock()
        .unwrap()
        .push("https://store/get/0".into());
    let mut out = Vec::new();
    let e: Error = r.read_to_end(&mut out).await.unwrap_err().into();
    assert_eq!((e.status(), e.code()), (410, "file_gone"));
    assert_eq!(out, b"0123");
    let e: Error = r.read_to_end(&mut out).await.unwrap_err().into();
    assert_eq!(e.code(), "file_gone");
    assert_eq!(opens(&seen), vec![None, Some(4)]);
}

/// One HTTP/1.1 exchange on a loopback socket: the request's head and body, then `answer`.
async fn serve_once(answer: &'static str) -> (String, tokio::task::JoinHandle<(String, Vec<u8>)>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let h = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let head_end = loop {
            let n = sock.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_lowercase();
        let len: usize = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length: "))
            .map(|v| v.trim().parse().unwrap())
            .unwrap_or(0);
        while buf.len() < head_end + len {
            let n = sock.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
        }
        sock.write_all(answer.as_bytes()).await.unwrap();
        (head, buf[head_end..].to_vec())
    });
    (format!("http://{addr}/o"), h)
}

#[tokio::test]
async fn the_http_client_sends_a_range_and_one_content_length_and_reads_the_etag() {
    use crate::files::Http;
    let http = Http(reqwest::Client::new());
    let (url, h) = serve_once(
        "HTTP/1.1 206 Partial Content\r\ncontent-length: 3\r\nconnection: close\r\n\r\nabc",
    )
    .await;
    let got = http.get(url, 5).await.unwrap();
    assert_eq!(got.status, 206);
    let body: Vec<Bytes> = got.body.map(|b| b.unwrap()).collect().await;
    assert_eq!(body.concat(), b"abc");
    let (head, _) = h.await.unwrap();
    assert!(head.starts_with("get /o "), "{head}");
    assert!(head.contains("\r\nrange: bytes=5-\r\n"), "{head}");

    let (url, h) = serve_once(
        "HTTP/1.1 200 OK\r\netag: \"e-7\"\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
    )
    .await;
    let put = http.put(url, Bytes::from_static(b"part")).await.unwrap();
    assert_eq!((put.status, put.etag.as_deref()), (200, Some("\"e-7\"")));
    let (head, body) = h.await.unwrap();
    assert!(head.starts_with("put /o "), "{head}");
    assert_eq!(head.matches("content-length:").count(), 1, "{head}");
    assert!(head.contains("\r\ncontent-length: 4\r\n"), "{head}");
    assert_eq!(body, b"part");
}
