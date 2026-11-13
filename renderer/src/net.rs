//! The window's network: the `net.*` messages of the host channel (docs/RUNTIME.md "The host channel") on real
//! WebSockets and HTTP. A game has no I/O; it posts `net.open` / `net.send` / `net.close` / `net.fetch`, and the app
//! that runs it owns the sockets. Here the app is `threert-render play`: every socket and every request has a thread
//! of its own, and what they receive waits in a queue the window empties once a frame (`poll`), so a socket's
//! messages reach the game as one `net.messages` batch a frame, in the order they arrived.
//!
//! Text messages only (a binary frame is dropped). Whoever runs `play` chose the game, so any ws / wss / http / https
//! address is allowed; an app that runs games it did not choose decides for itself what they may reach.
//!
//! Built with the `net` feature (on by default; tungstenite and ureq on rustls). Without it every request is answered
//! with a failure that says so, and the game goes on as it must with an app that has no network.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use threert_render::host::HostServices;

/// Sockets open at once, requests under way at once.
const MAX_SOCKETS: usize = 16;
const MAX_FETCHES: usize = 32;

/// What a socket's or a request's thread reports, by the game's id for it.
#[cfg_attr(not(feature = "net"), allow(dead_code))]
enum Event {
    Opened(Value),
    Message(Value, String),
    Closed(Value, u16, String),
    /// Status (0: the request failed), body, the failure.
    Fetched(Value, u16, String, Option<String>),
}

/// What the window tells a socket's thread.
#[cfg_attr(not(feature = "net"), allow(dead_code))]
enum Out {
    Text(String),
    Close(u16, String),
}

pub struct Net {
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    sockets: HashMap<String, mpsc::Sender<Out>>,
    fetches: Arc<AtomicUsize>,
}

/// An id as a map's key: the game's string, or whatever JSON it used.
fn key(id: &Value) -> String {
    id.as_str().map_or_else(|| id.to_string(), String::from)
}

impl Net {
    pub fn new() -> Net {
        // (The TLS provider of this process, for both crates.)
        #[cfg(feature = "net")]
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (tx, rx) = mpsc::channel();
        Net { tx, rx, sockets: HashMap::new(), fetches: Arc::default() }
    }

    fn open(&mut self, id: Value, m: &Value) {
        let k = key(&id);
        let refuse = |why: &str| {
            let _ = self.tx.send(Event::Closed(id.clone(), 1006, why.to_string()));
        };
        let Some(url) = m["url"].as_str() else { return refuse("net.open: no url") };
        if self.sockets.contains_key(&k) {
            // (Not a `net.closed`: the socket of that id is open, and stays.)
            return eprintln!("net: net.open: a socket \"{k}\" is open already");
        }
        if self.sockets.len() >= MAX_SOCKETS {
            return refuse(&format!("net.open: {MAX_SOCKETS} sockets are open already"));
        }
        let protocols: Vec<String> = m["protocols"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(String::from)).collect()).unwrap_or_default();
        let (out, queue) = mpsc::channel();
        self.sockets.insert(k, out);
        let (url, events) = (url.to_string(), self.tx.clone());
        #[cfg(feature = "net")]
        std::thread::spawn(move || imp::socket(id, &url, &protocols, queue, events));
        #[cfg(not(feature = "net"))]
        {
            let _ = (url, protocols, queue);
            let _ = events.send(Event::Closed(id, 1006, NO_NET.to_string()));
        }
    }

    fn fetch(&mut self, id: Value, m: &Value) {
        let fail = |why: &str| {
            let _ = self.tx.send(Event::Fetched(id.clone(), 0, String::new(), Some(why.to_string())));
        };
        let Some(url) = m["url"].as_str() else { return fail("net.fetch: no url") };
        if self.fetches.load(Ordering::Relaxed) >= MAX_FETCHES {
            return fail(&format!("net.fetch: {MAX_FETCHES} requests are under way already"));
        }
        let method = m["method"].as_str().unwrap_or("GET").to_uppercase();
        let headers: Vec<(String, String)> = m["headers"].as_object().map(|h| h.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default();
        let body = m["body"].as_str().map(String::from);
        let (url, events, count) = (url.to_string(), self.tx.clone(), self.fetches.clone());
        count.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "net")]
        std::thread::spawn(move || {
            let r = imp::fetch(&method, &url, &headers, body);
            count.fetch_sub(1, Ordering::Relaxed);
            let _ = events.send(match r {
                Ok((status, body)) => Event::Fetched(id, status, body, None),
                Err(e) => Event::Fetched(id, 0, String::new(), Some(e)),
            });
        });
        #[cfg(not(feature = "net"))]
        {
            let _ = (method, url, headers, body);
            count.fetch_sub(1, Ordering::Relaxed);
            let _ = events.send(Event::Fetched(id, 0, String::new(), Some(NO_NET.to_string())));
        }
    }
}

#[cfg(not(feature = "net"))]
const NO_NET: &str = "this threert-render was built without its network (the `net` feature)";

impl HostServices for Net {
    fn post(&mut self, m: &Value) {
        let id = m["id"].clone();
        match m["t"].as_str() {
            Some("net.open") => self.open(id, m),
            Some("net.fetch") => self.fetch(id, m),
            // (To a socket that is not open yet: sent once it is. To one that is gone, or never was: dropped.)
            Some("net.send") => {
                if let (Some(s), Some(data)) = (self.sockets.get(&key(&id)), m["data"].as_str()) {
                    let _ = s.send(Out::Text(data.to_string()));
                }
            }
            Some("net.close") => {
                if let Some(s) = self.sockets.get(&key(&id)) {
                    let code = m["code"].as_u64().filter(|c| *c == 1000 || (3000..5000).contains(c)).unwrap_or(1000) as u16;
                    let _ = s.send(Out::Close(code, m["reason"].as_str().unwrap_or("").chars().take(60).collect()));
                }
            }
            _ => {}
        }
    }

    fn poll(&mut self) -> Vec<Value> {
        let mut out: Vec<Value> = vec![];
        // Where each socket's batch of this frame is in `out`.
        let mut batch: HashMap<String, usize> = HashMap::new();
        while let Ok(e) = self.rx.try_recv() {
            match e {
                Event::Opened(id) => out.push(json!({ "t": "net.opened", "id": id })),
                Event::Message(id, text) => match batch.get(&key(&id)) {
                    Some(&k) => out[k]["list"].as_array_mut().expect("a batch").push(text.into()),
                    None => {
                        batch.insert(key(&id), out.len());
                        out.push(json!({ "t": "net.messages", "id": id, "list": [text] }));
                    }
                },
                Event::Closed(id, code, reason) => {
                    self.sockets.remove(&key(&id));
                    out.push(json!({ "t": "net.closed", "id": id, "code": code, "reason": reason }));
                }
                Event::Fetched(id, status, body, error) => {
                    let mut m = json!({ "t": "net.fetched", "id": id, "status": status, "body": body });
                    if let Some(e) = error {
                        m["error"] = e.into();
                    }
                    out.push(m);
                }
            }
        }
        out
    }
}

#[cfg(feature = "net")]
mod imp {
    use super::{Event, Out};
    use serde_json::Value;
    use std::net::{TcpStream, ToSocketAddrs};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use tungstenite::client::IntoClientRequest;
    use tungstenite::protocol::{frame::coding::CloseCode, CloseFrame, WebSocketConfig};
    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Error, Message, WebSocket};

    /// To connect and to shake hands; a request in all; a close's answer.
    const CONNECT: Duration = Duration::from_secs(10);
    const FETCH: Duration = Duration::from_secs(30);
    const CLOSE: Duration = Duration::from_secs(2);
    /// How long a socket's thread waits for the server before it looks at what the game wants sent: what a `net.send`
    /// waits at most.
    const TURN: Duration = Duration::from_millis(2);
    /// The largest message a socket takes, the largest body a request reads.
    const MAX_MESSAGE: usize = 1 << 20;
    const MAX_BODY: u64 = 4 << 20;

    fn connect(url: &str, protocols: &[String]) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
        let mut request = url.into_client_request().map_err(|e| format!("{url}: {e}"))?;
        let uri = request.uri().clone();
        let port = match uri.scheme_str() {
            Some("ws") => uri.port_u16().unwrap_or(80),
            Some("wss") => uri.port_u16().unwrap_or(443),
            _ => return Err(format!("{url}: a socket's address is ws:// or wss://")),
        };
        let host = uri.host().ok_or_else(|| format!("{url}: no host"))?.trim_start_matches('[').trim_end_matches(']').to_string();
        if !protocols.is_empty() {
            request.headers_mut().insert("Sec-WebSocket-Protocol", protocols.join(", ").parse().map_err(|_| "net.open: protocols are not header text".to_string())?);
        }
        let mut last = format!("{host}: no address");
        let stream = (host.as_str(), port).to_socket_addrs().map_err(|e| format!("{host}: {e}"))?.find_map(|a| TcpStream::connect_timeout(&a, CONNECT).map_err(|e| last = format!("{a}: {e}")).ok()).ok_or(last)?;
        let io = |e: std::io::Error| e.to_string();
        stream.set_nodelay(true).map_err(io)?;
        stream.set_read_timeout(Some(CONNECT)).map_err(io)?;
        stream.set_write_timeout(Some(CONNECT)).map_err(io)?;
        // (The same socket: its timeouts are set through this handle once the handshake is over.)
        let control = stream.try_clone().map_err(io)?;
        let config = WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE)).max_frame_size(Some(MAX_MESSAGE));
        let (ws, _) = tungstenite::client_tls_with_config(request, stream, Some(config), None).map_err(|e| format!("{url}: {e}"))?;
        control.set_read_timeout(Some(TURN)).map_err(io)?;
        control.set_write_timeout(None).map_err(io)?;
        Ok(ws)
    }

    /// A socket's thread: connects, then turns between what the game wants sent and what the server says, until
    /// either side closes or the window is gone.
    pub fn socket(id: Value, url: &str, protocols: &[String], out: mpsc::Receiver<Out>, events: mpsc::Sender<Event>) {
        let mut ws = match connect(url, protocols) {
            Ok(ws) => ws,
            Err(e) => {
                let _ = events.send(Event::Closed(id, 1006, e));
                return;
            }
        };
        let _ = events.send(Event::Opened(id.clone()));
        // When we asked to close.
        let mut closing: Option<Instant> = None;
        let (code, reason) = 'open: loop {
            loop {
                match out.try_recv() {
                    Ok(Out::Text(text)) if closing.is_none() => {
                        if let Err(e) = ws.send(Message::text(text)) {
                            break 'open (1006, e.to_string());
                        }
                    }
                    Ok(Out::Text(_)) => {}
                    Ok(Out::Close(code, reason)) => {
                        if closing.is_none() {
                            let _ = ws.close(Some(CloseFrame { code: CloseCode::from(code), reason: reason.into() }));
                            closing = Some(Instant::now());
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    // (The window is gone.)
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if closing.is_none() {
                            let _ = ws.close(None);
                            closing = Some(Instant::now());
                        }
                        break;
                    }
                }
            }
            match ws.read() {
                Ok(Message::Text(text)) => {
                    if events.send(Event::Message(id.clone(), text.to_string())).is_err() {
                        return;
                    }
                }
                // The server's close: its answer to ours, or its own (ours to it goes out with the flush). The
                // socket is done either way; a server that keeps the connection open after it is not waited for.
                Ok(Message::Close(f)) => {
                    let _ = ws.flush();
                    break f.map_or((1005, String::new()), |f| (u16::from(f.code), f.reason.to_string()));
                }
                Ok(_) => {}
                Err(Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(Error::ConnectionClosed | Error::AlreadyClosed) => break (1005, String::new()),
                Err(e) => break (1006, e.to_string()),
            }
            if closing.is_some_and(|t| t.elapsed() > CLOSE) {
                break (1006, "the server did not answer the close".to_string());
            }
        };
        let _ = events.send(Event::Closed(id, code, reason));
    }

    /// One HTTP request: its status and its body as text.
    pub fn fetch(method: &str, url: &str, headers: &[(String, String)], body: Option<String>) -> Result<(u16, String), String> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!("{url}: a request's address is http:// or https://"));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(FETCH)).http_status_as_error(false).build().into();
        let mut request = ureq::http::Request::builder().method(method).uri(url);
        for (k, v) in headers {
            request = request.header(k.as_str(), v.as_str());
        }
        let bad = |e: ureq::http::Error| format!("net.fetch: {e}");
        let mut response = match body {
            Some(text) => agent.run(request.body(text).map_err(bad)?),
            None => agent.run(request.body(()).map_err(bad)?),
        }
        .map_err(|e| format!("{url}: {e}"))?;
        let status = response.status().as_u16();
        let text = response.body_mut().with_config().limit(MAX_BODY).read_to_string().map_err(|e| format!("{url}: {e}"))?;
        Ok((status, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the threads reported since the last frame, as the game gets it: a socket's messages in one batch, in
    /// order, between its `net.opened` and its `net.closed`; a closed socket's id is free again.
    #[test]
    fn a_frame_gets_one_batch_a_socket() {
        let mut net = Net::new();
        let (a, b) = (json!("a"), json!(7));
        net.sockets.insert("a".into(), mpsc::channel().0);
        for e in [
            Event::Opened(a.clone()),
            Event::Message(a.clone(), "1".into()),
            Event::Message(b.clone(), "x".into()),
            Event::Message(a.clone(), "2".into()),
            Event::Fetched(json!("page"), 200, "body".into(), None),
            Event::Closed(a.clone(), 1000, String::new()),
            Event::Fetched(json!("gone"), 0, String::new(), Some("no route".into())),
        ] {
            net.tx.send(e).unwrap();
        }
        assert_eq!(
            net.poll(),
            vec![
                json!({ "t": "net.opened", "id": "a" }),
                json!({ "t": "net.messages", "id": "a", "list": ["1", "2"] }),
                json!({ "t": "net.messages", "id": 7, "list": ["x"] }),
                json!({ "t": "net.fetched", "id": "page", "status": 200, "body": "body" }),
                json!({ "t": "net.closed", "id": "a", "code": 1000, "reason": "" }),
                json!({ "t": "net.fetched", "id": "gone", "status": 0, "body": "", "error": "no route" }),
            ]
        );
        assert!(net.sockets.is_empty() && net.poll().is_empty());
    }

    /// Requests the window refuses itself are answered like any other, at the next frame; posts of other families
    /// and sends to sockets that are not there are not this module's.
    #[test]
    fn what_cannot_be_asked_is_answered_with_a_failure() {
        let mut net = Net::new();
        net.post(&json!({ "t": "net.open", "id": "a" }));
        net.post(&json!({ "t": "net.fetch", "id": "f" }));
        net.post(&json!({ "t": "net.send", "id": "nobody", "data": "x" }));
        net.post(&json!({ "t": "net.close", "id": "nobody" }));
        net.post(&json!({ "t": "hud", "hud": {} }));
        let out = net.poll();
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!((out[0]["t"].as_str(), out[0]["code"].as_u64(), out[0]["reason"].as_str()), (Some("net.closed"), Some(1006), Some("net.open: no url")));
        assert_eq!((out[1]["t"].as_str(), out[1]["status"].as_u64(), out[1]["error"].as_str()), (Some("net.fetched"), Some(0), Some("net.fetch: no url")));
        // An address that is no socket's fails on the socket's thread, and says why.
        #[cfg(feature = "net")]
        {
            net.post(&json!({ "t": "net.open", "id": "b", "url": "http://127.0.0.1:9/" }));
            net.post(&json!({ "t": "net.fetch", "id": "g", "url": "ftp://127.0.0.1/" }));
            let mut got = vec![];
            for _ in 0..200 {
                got.extend(net.poll());
                if got.len() == 2 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            got.sort_by_key(|m| m["t"].as_str().map(String::from));
            assert_eq!(got.len(), 2, "{got:?}");
            assert!(got[0]["reason"].as_str().is_some_and(|r| r.contains("ws:// or wss://")), "{got:?}");
            assert!(got[1]["error"].as_str().is_some_and(|r| r.contains("http:// or https://")), "{got:?}");
            assert!(net.sockets.is_empty());
        }
    }
}
