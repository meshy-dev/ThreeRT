//! `assets.*` on the host channel (docs/RUNTIME.md "Assets"), for any app that holds a `GameSession`: a game asks for
//! a file to be downloaded into its asset root, under `cache/`, and reads it with `readAsset` once the answer comes.
//! The files are kept across runs, so the cache is what a game has offline.
//!
//! `AssetFetcher` is a [`HostServices`]: hand it the game's posts (it takes `assets.fetch` and `assets.cancel` and
//! ignores the rest), poll it once a frame for the answers, list its `caps` in the `launch` message. Downloads run on
//! threads of their own, at most `MAX_ASSETS` at once, the rest in the order they came.
//!
//! What a game may reach is the embedding app's to decide: by default `https://` only, anywhere. An app with a network
//! policy gives its own check of an address (`with_policy`), which every redirect is held to as well (the fetcher
//! follows them itself, one at a time), and, to hold every address a name resolves to, its own HTTP agent
//! (`with_agent`: a `ureq::Agent` with the app's resolver, for one).
//! `threert-render play` (src/net.rs) allows `http://` too: whoever runs it chose the game.
//!
//! Behind the `assets` cargo feature (ureq on rustls; on with `net`, and so by default).

use crate::host::HostServices;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// Downloads under way at once (more wait in order), the largest file one may bring.
pub const MAX_ASSETS: usize = 16;
pub const MAX_ASSET: u64 = 64 << 20;
/// A download in all; its connection.
const DOWNLOAD: Duration = Duration::from_secs(300);
/// Redirects a download follows, each one checked by the policy.
const REDIRECTS: usize = 5;
const CONNECT: Duration = Duration::from_secs(10);

/// Whether a game may download from an address: `Err` says why not (the answer's `error`).
pub type Policy = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// The default policy: `https://` addresses only.
pub fn https_only() -> Policy {
    Arc::new(|url: &str| if url.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("https://")) { Ok(()) } else { Err(format!("{url}: an asset's address is https://")) })
}

/// `http://` and `https://` addresses (`threert-render play`).
pub fn http_or_https() -> Policy {
    Arc::new(|url: &str| {
        let l = url.get(..8).unwrap_or(url).to_ascii_lowercase();
        if l.starts_with("http://") || l.starts_with("https://") { Ok(()) } else { Err(format!("{url}: an asset's address is http:// or https://")) }
    })
}

/// The file an `assets.fetch`'s `path` names under `root`: `cache/` and below, relative, no `.` or `..` parts, no
/// backslashes or colons, nothing that names a download's own files (`.part`, the records); None for any other.
pub fn cache_file(root: &Path, path: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = path.split('/').collect();
    let ok = parts.len() >= 2
        && parts[0] == "cache"
        && parts.iter().all(|p| !p.is_empty() && *p != "." && *p != ".." && !p.contains('\\') && !p.contains(':'))
        && !parts.last().is_some_and(|n| n.ends_with(".part") || (n.starts_with('.') && n.ends_with(".fetched")));
    ok.then(|| parts.iter().fold(root.to_path_buf(), |f, p| f.join(p)))
}

/// What is known of a file a download brought (`.<name>.fetched` beside it, JSON): where it came from, its hash, its
/// size.
pub fn record_of(file: &Path) -> PathBuf {
    let name = file.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    file.with_file_name(format!(".{name}.fetched"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

struct Job {
    id: Value,
    url: String,
    path: String,
    file: PathBuf,
    sha256: Option<String>,
    policy: Policy,
}

fn answer(job: &Job, status: u16, size: u64, cached: bool) -> Value {
    json!({ "t": "assets.fetched", "id": job.id, "path": job.path, "status": status, "size": size, "cached": cached })
}

fn failed(id: &Value, path: &str, why: &str) -> Value {
    json!({ "t": "assets.fetched", "id": id, "path": path, "status": 0, "size": 0, "cached": false, "error": why })
}

/// The answer for a file that is there already: from the same address as when it came, or with the same sha256 (the
/// record's, or the file's own when it has no record). None: it is to be downloaded.
fn cached(job: &Job) -> Option<Value> {
    let size = std::fs::metadata(&job.file).ok().filter(|m| m.is_file())?.len();
    let record: Option<Value> = std::fs::read(record_of(&job.file)).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let same = match &record {
        Some(r) if r["size"].as_u64() == Some(size) => r["url"].as_str() == Some(job.url.as_str()) || (job.sha256.is_some() && r["sha256"].as_str() == job.sha256.as_deref()),
        Some(_) => false,
        None => job.sha256.as_ref().is_some_and(|w| size <= MAX_ASSET && std::fs::read(&job.file).is_ok_and(|b| &sha256_hex(&b) == w)),
    };
    same.then(|| answer(job, 200, size, true))
}

/// The address a redirect's `Location` names, from the address that answered it: absolute (`scheme://...`), from the
/// host's root (`/...`), from the scheme (`//host/...`), or beside the answering path. None for one this can't read.
fn redirect_to(from: &str, location: &str) -> Option<String> {
    let location = location.trim();
    if location.is_empty() {
        return None;
    }
    let scheme_end = from.find("://")?;
    if location.split(['/', '?', '#']).next().is_some_and(|head| head.contains(':')) {
        return Some(location.to_string());
    }
    if location.starts_with("//") {
        return Some(format!("{}:{location}", &from[..scheme_end]));
    }
    let after = scheme_end + 3;
    let host_end = from[after..].find(['/', '?', '#']).map_or(from.len(), |i| after + i);
    if location.starts_with('/') {
        return Some(format!("{}{location}", &from[..host_end]));
    }
    let path = &from[host_end..from[host_end..].find(['?', '#']).map_or(from.len(), |i| host_end + i)];
    let dir = path.rfind('/').map_or("/", |i| &path[..=i]);
    Some(format!("{}{dir}{location}", &from[..host_end]))
}

/// A download: the body into `<file>.part` (its size and, when asked, its sha256 checked), renamed to the file and
/// recorded beside it. A status other than 200 writes nothing and is answered as it is.
fn download(agent: &ureq::Agent, job: &Job, cancel: &AtomicBool) -> Value {
    use sha2::Digest;
    use std::io::{Read, Write};
    let fail = |why: String| failed(&job.id, &job.path, &why);
    // (Redirects one at a time, so that the policy sees each address, not only the first.)
    let mut url = job.url.clone();
    let mut hops = 0;
    let mut response = loop {
        let r = match agent.get(&url).call() {
            Ok(r) => r,
            Err(e) => return fail(format!("{url}: {e}")),
        };
        let location = r.headers().get("location").and_then(|l| l.to_str().ok());
        let Some(next) = location.filter(|_| r.status().is_redirection()).and_then(|l| redirect_to(&url, l)) else { break r };
        if hops == REDIRECTS {
            return fail(format!("{}: more than {REDIRECTS} redirects", job.url));
        }
        if let Err(e) = (job.policy)(&next) {
            return fail(format!("{} redirects to {e}", job.url));
        }
        (url, hops) = (next, hops + 1);
    };
    let status = response.status().as_u16();
    if status != 200 {
        return answer(job, status, 0, false);
    }
    if let Some(dir) = job.file.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return fail(format!("{}: {e}", job.path));
        }
    }
    let part = job.file.with_file_name(format!("{}.part", job.file.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())));
    let result = (|| -> Result<(u64, String), String> {
        let mut out = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", job.path))?;
        let mut body = response.body_mut().with_config().limit(MAX_ASSET + 1).reader();
        let (mut hash, mut size, mut buf) = (sha2::Sha256::new(), 0u64, vec![0u8; 1 << 16]);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("canceled".into());
            }
            let n = body.read(&mut buf).map_err(|e| if size >= MAX_ASSET { format!("{url}: larger than 64 MiB") } else { format!("{url}: {e}") })?;
            if n == 0 {
                break;
            }
            size += n as u64;
            if size > MAX_ASSET {
                return Err(format!("{url}: larger than 64 MiB"));
            }
            hash.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", job.path))?;
        }
        out.flush().map_err(|e| e.to_string())?;
        Ok((size, hash.finalize().iter().map(|b| format!("{b:02x}")).collect()))
    })();
    let (size, sha) = match result {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return fail(e);
        }
    };
    if job.sha256.as_ref().is_some_and(|want| *want != sha) {
        let _ = std::fs::remove_file(&part);
        return fail(format!("{}: its sha256 is {sha}, not {}", job.url, job.sha256.as_deref().unwrap_or("")));
    }
    if let Err(e) = std::fs::rename(&part, &job.file) {
        let _ = std::fs::remove_file(&part);
        return fail(format!("{}: {e}", job.path));
    }
    let _ = std::fs::write(record_of(&job.file), json!({ "url": job.url, "sha256": sha, "size": size }).to_string());
    answer(job, 200, size, false)
}

/// The `assets.*` requests of one game (above). Dropping it (the game closed) stops what is under way.
pub struct AssetFetcher {
    root: PathBuf,
    policy: Policy,
    agent: ureq::Agent,
    tx: mpsc::Sender<Value>,
    rx: mpsc::Receiver<Value>,
    waiting: VecDeque<Job>,
    running: HashMap<String, Arc<AtomicBool>>,
    counts: AssetCounts,
}

/// What a fetcher did so far (for an app's status page).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AssetCounts {
    pub requests: u64,
    pub downloaded: u64,
    pub cached: u64,
    pub failed: u64,
    pub bytes: u64,
}

fn key(id: &Value) -> String {
    id.as_str().map_or_else(|| id.to_string(), String::from)
}

impl AssetFetcher {
    /// Downloads into `<root>/cache/...` (the game's asset root, which `readAsset` reads), `https://` only, through
    /// an agent of its own (no proxy unless the environment names one; rustls with the webpki roots).
    pub fn new(root: PathBuf) -> AssetFetcher {
        // (The TLS provider of this process, if nobody installed one.)
        let _ = rustls::crypto::ring::default_provider().install_default();
        let agent = ureq::Agent::config_builder().timeout_connect(Some(CONNECT)).timeout_global(Some(DOWNLOAD)).http_status_as_error(false).max_redirects(0).build().into();
        let (tx, rx) = mpsc::channel();
        AssetFetcher { root, policy: https_only(), agent, tx, rx, waiting: VecDeque::new(), running: HashMap::new(), counts: AssetCounts::default() }
    }

    /// Which addresses a game may download from (default `https_only`). Checked before a request is queued.
    pub fn with_policy(mut self, policy: Policy) -> AssetFetcher {
        self.policy = policy;
        self
    }

    /// The HTTP agent the downloads use: the app's own (its resolver, its proxy). It should not turn statuses into
    /// errors (`http_status_as_error(false)`), so that a 404 is answered as one, and should follow no redirects
    /// (`max_redirects(0)`), so that the fetcher follows them with the policy checked at each (ones the agent follows
    /// itself are the agent's to check).
    pub fn with_agent(mut self, agent: ureq::Agent) -> AssetFetcher {
        self.agent = agent;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn counts(&self) -> AssetCounts {
        self.counts
    }

    /// Downloads under way, requests waiting.
    pub fn pending(&self) -> (usize, usize) {
        (self.running.len(), self.waiting.len())
    }

    fn fetch(&mut self, id: Value, m: &Value) {
        self.counts.requests += 1;
        // (An id under way or waiting already: this one is not answered, the first is. Checked before a refusal, whose
        // answer would otherwise stand for the first.)
        let k = key(&id);
        if self.running.contains_key(&k) || self.waiting.iter().any(|j| key(&j.id) == k) {
            return;
        }
        let path = m["path"].as_str().unwrap_or("").to_string();
        let refuse = |why: String| {
            let _ = self.tx.send(failed(&id, &path, &why));
        };
        let Some(url) = m["url"].as_str() else { return refuse("assets.fetch: no url".into()) };
        if let Err(e) = (self.policy)(url) {
            return refuse(e);
        }
        let Some(file) = cache_file(&self.root, &path) else { return refuse(format!("assets.fetch: \"{path}\" is not a path under cache/ (no ., .., backslashes or colons)")) };
        let sha256 = m["sha256"].as_str().map(str::to_ascii_lowercase);
        self.waiting.push_back(Job { id, url: url.to_string(), path, file, sha256, policy: self.policy.clone() });
        self.start();
    }

    fn start(&mut self) {
        while self.running.len() < MAX_ASSETS {
            let Some(job) = self.waiting.pop_front() else { break };
            let cancel = Arc::new(AtomicBool::new(false));
            self.running.insert(key(&job.id), cancel.clone());
            let (tx, agent) = (self.tx.clone(), self.agent.clone());
            let _ = std::thread::Builder::new().name("threert-asset".into()).spawn(move || {
                let a = cached(&job).unwrap_or_else(|| download(&agent, &job, &cancel));
                let _ = tx.send(a);
            });
        }
    }
}

impl HostServices for AssetFetcher {
    fn post(&mut self, m: &Value) {
        let id = m["id"].clone();
        match m["t"].as_str() {
            Some("assets.fetch") => self.fetch(id, m),
            // (A request that waits is taken out; one under way stops, its partial file removed. Neither is answered.)
            Some("assets.cancel") => {
                let k = key(&id);
                self.waiting.retain(|j| key(&j.id) != k);
                if let Some(c) = self.running.get(&k) {
                    c.store(true, Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }

    /// The answers since the last call, in the order they came (a canceled download's left out).
    fn poll(&mut self) -> Vec<Value> {
        let mut out = vec![];
        while let Ok(a) = self.rx.try_recv() {
            // (A request refused at once was never under way.)
            if self.running.remove(&key(&a["id"])).is_none_or(|c| !c.load(Ordering::Relaxed)) {
                let c = &mut self.counts;
                if a["cached"] == json!(true) {
                    c.cached += 1;
                } else if a["status"] == json!(0) {
                    c.failed += 1;
                } else if a["status"] == json!(200) {
                    c.downloaded += 1;
                    c.bytes += a["size"].as_u64().unwrap_or(0);
                }
                out.push(a);
            }
        }
        self.start();
        out
    }

    fn caps(&self) -> Vec<&'static str> {
        vec!["assets"]
    }
}

impl Drop for AssetFetcher {
    fn drop(&mut self) {
        for c in self.running.values() {
            c.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where `assets.fetch` may write: `cache/` and below, nothing that climbs out or is a download's own file.
    #[test]
    fn asset_paths_stay_in_the_cache() {
        let root = Path::new("/r");
        assert_eq!(cache_file(root, "cache/tiles/15/1/2.ert"), Some(PathBuf::from("/r/cache/tiles/15/1/2.ert")));
        for bad in ["cache", "cache/", "tiles/a", "/cache/a", "cache/../a", "cache/./a", "cache//a", "cache/a\\b", "cache/c:x", "cache/a/..", "", "cache/a.part", "cache/.a.fetched"] {
            assert_eq!(cache_file(root, bad), None, "{bad}");
        }
        assert!(https_only()("https://x.org/a").is_ok() && https_only()("HTTPS://x.org/a").is_ok() && https_only()("http://x.org/a").is_err());
        assert!(http_or_https()("http://x.org/a").is_ok() && http_or_https()("file:///etc/hosts").is_err());
    }

    /// Against a server on this machine (http, so with `http_or_https`): a file downloaded into the cache and
    /// recorded, the same address again `cached`, the same hash from another address `cached`, a hash that is not the
    /// body's refused (nothing kept), a 404 answered as it is (nothing written), refusals (a path out of the cache,
    /// an address the policy refuses: the default refuses http), 24 requests at once (16 under way), a canceled one
    /// not answered.
    #[test]
    fn assets_come_into_the_cache() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in server.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let path = line.split(' ').nth(1).unwrap_or("/").to_string();
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) <= 2 {
                            break;
                        }
                    }
                    let mut stream = stream;
                    let (status, body) = if path.starts_with("/missing") { ("404 Not Found", String::new()) } else { ("200 OK", format!("body of {path}")) };
                    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                });
            }
        });
        let root = std::env::temp_dir().join(format!("threert-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // (No proxy: the server is this machine's.)
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).proxy(None).build().into();
        let mut f = AssetFetcher::new(root.clone()).with_policy(http_or_https()).with_agent(agent);
        assert_eq!(f.caps(), vec!["assets"]);
        let url = |p: &str| format!("http://127.0.0.1:{port}{p}");
        let wait = |f: &mut AssetFetcher, n: usize| {
            let mut got = vec![];
            for _ in 0..500 {
                got.extend(f.poll());
                if got.len() >= n {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            got
        };
        f.post(&json!({ "t": "assets.fetch", "id": "a", "url": url("/a.bin"), "path": "cache/x/a.bin" }));
        assert_eq!(wait(&mut f, 1), vec![json!({ "t": "assets.fetched", "id": "a", "path": "cache/x/a.bin", "status": 200, "size": 14, "cached": false })]);
        assert_eq!(std::fs::read_to_string(root.join("cache/x/a.bin")).unwrap(), "body of /a.bin");
        f.post(&json!({ "t": "assets.fetch", "id": "b", "url": url("/a.bin"), "path": "cache/x/a.bin" }));
        let sha = sha256_hex(b"body of /a.bin");
        f.post(&json!({ "t": "assets.fetch", "id": "c", "url": url("/mirror/a.bin"), "path": "cache/x/a.bin", "sha256": sha.to_uppercase() }));
        let got = wait(&mut f, 2);
        assert!(got.len() == 2 && got.iter().all(|m| m["cached"] == json!(true) && m["status"] == json!(200) && m["size"] == json!(14)), "{got:?}");
        f.post(&json!({ "t": "assets.fetch", "id": "d", "url": url("/d.bin"), "path": "cache/d.bin", "sha256": "00" }));
        let got = wait(&mut f, 1);
        assert!(got[0]["status"] == json!(0) && got[0]["error"].as_str().is_some_and(|e| e.contains("sha256")), "{got:?}");
        assert!(!root.join("cache/d.bin").exists() && !root.join("cache/d.bin.part").exists());
        f.post(&json!({ "t": "assets.fetch", "id": "e", "url": url("/missing"), "path": "cache/e.bin" }));
        let got = wait(&mut f, 1);
        assert!(got[0]["status"] == json!(404) && got[0]["cached"] == json!(false) && !root.join("cache/e.bin").exists(), "{got:?}");
        f.post(&json!({ "t": "assets.fetch", "id": "f", "url": url("/f"), "path": "cache/../f" }));
        f.post(&json!({ "t": "assets.fetch", "id": "g", "url": "file:///etc/hosts", "path": "cache/g" }));
        let got = wait(&mut f, 2);
        assert!(got.len() == 2 && got.iter().all(|m| m["status"] == json!(0) && m["error"].is_string()), "{got:?}");
        let mut strict = AssetFetcher::new(root.clone());
        strict.post(&json!({ "t": "assets.fetch", "id": "h", "url": url("/h"), "path": "cache/h" }));
        let got = strict.poll();
        assert!(got.len() == 1 && got[0]["error"].as_str().is_some_and(|e| e.contains("https://")), "{got:?}");
        for i in 0..24 {
            f.post(&json!({ "t": "assets.fetch", "id": format!("n{i}"), "url": url(&format!("/n{i}")), "path": format!("cache/n/{i}") }));
        }
        assert_eq!(f.pending(), (MAX_ASSETS, 8));
        f.post(&json!({ "t": "assets.cancel", "id": "n20" }));
        let mut got = wait(&mut f, 23);
        std::thread::sleep(Duration::from_millis(100));
        got.extend(f.poll());
        assert_eq!(got.len(), 23, "{got:?}");
        assert!(got.iter().all(|m| m["status"] == json!(200) && m["id"] != json!("n20")));
        assert!(!root.join("cache/n/20").exists() && f.pending() == (0, 0));
        // ("body of /nI": 11 bytes for the ten of one digit, 12 for the thirteen others that came.)
        assert_eq!(f.counts(), AssetCounts { requests: 31, downloaded: 24, cached: 2, failed: 3, bytes: 14 + 10 * 11 + 13 * 12 });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Where a redirect goes, from the address that answered it.
    #[test]
    fn redirects_resolve_from_the_answering_address() {
        let from = "https://x.org/a/b.bin?q=1";
        for (location, want) in [
            ("https://y.org/c", "https://y.org/c"),
            ("HTTP://y.org/c", "HTTP://y.org/c"),
            ("//y.org/c", "https://y.org/c"),
            ("/c/d", "https://x.org/c/d"),
            ("c.bin", "https://x.org/a/c.bin"),
            ("c.bin?v=2", "https://x.org/a/c.bin?v=2"),
            ("file:///etc/hosts", "file:///etc/hosts"),
        ] {
            assert_eq!(redirect_to(from, location).as_deref(), Some(want), "{location}");
        }
        assert_eq!(redirect_to("https://x.org", "c").as_deref(), Some("https://x.org/c"));
        assert_eq!(redirect_to(from, " "), None);
    }

    /// Redirects followed with the policy checked at each: one beside the path and one from the root brought, one to
    /// an address the policy refuses refused (nothing written), one that loops given up; the default agent follows
    /// none on its own. A second request under a running id is ignored even when it would be refused, and the first
    /// is still answered.
    #[test]
    fn redirects_are_held_to_the_policy() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in server.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let path = line.split(' ').nth(1).unwrap_or("/").to_string();
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) <= 2 {
                            break;
                        }
                    }
                    let location = match path.as_str() {
                        "/r/near" => Some("b.bin".to_string()),
                        "/root" => Some("/r/b.bin".to_string()),
                        "/away" => Some(format!("http://127.0.0.1:{port}/forbidden/b.bin")),
                        "/loop" => Some("/loop".to_string()),
                        _ => None,
                    };
                    let mut stream = stream;
                    let _ = match location {
                        Some(l) => write!(stream, "HTTP/1.1 302 Found\r\nLocation: {l}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                        None => write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\nbody of {path}", 8 + path.len()),
                    };
                });
            }
        });
        let root = std::env::temp_dir().join(format!("threert-assets-redirects-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).proxy(None).build().into();
        let open = http_or_https();
        let policy: Policy = Arc::new(move |url: &str| if url.contains("/forbidden/") { Err(format!("{url}: not here")) } else { open(url) });
        let mut f = AssetFetcher::new(root.clone()).with_policy(policy).with_agent(agent);
        let url = |p: &str| format!("http://127.0.0.1:{port}{p}");
        let wait = |f: &mut AssetFetcher, n: usize| {
            let mut got = vec![];
            for _ in 0..500 {
                got.extend(f.poll());
                if got.len() >= n {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            got.sort_by_key(|m| m["id"].as_str().unwrap_or("").to_string());
            got
        };
        for (id, p) in [("a", "/r/near"), ("b", "/root"), ("c", "/away"), ("d", "/loop")] {
            f.post(&json!({ "t": "assets.fetch", "id": id, "url": url(p), "path": format!("cache/{id}") }));
        }
        // (The same id again, and refused at once if it were taken: ignored.)
        f.post(&json!({ "t": "assets.fetch", "id": "a", "url": "file:///etc/hosts", "path": "cache/a" }));
        let got = wait(&mut f, 4);
        assert_eq!(got.len(), 4, "{got:?}");
        assert_eq!(got[0], json!({ "t": "assets.fetched", "id": "a", "path": "cache/a", "status": 200, "size": 16, "cached": false }));
        assert_eq!(got[1], json!({ "t": "assets.fetched", "id": "b", "path": "cache/b", "status": 200, "size": 16, "cached": false }));
        assert_eq!(std::fs::read_to_string(root.join("cache/a")).unwrap(), "body of /r/b.bin");
        assert!(got[2]["status"] == json!(0) && got[2]["error"].as_str().is_some_and(|e| e.contains("not here")) && !root.join("cache/c").exists(), "{got:?}");
        assert!(got[3]["status"] == json!(0) && got[3]["error"].as_str().is_some_and(|e| e.contains("redirects")) && !root.join("cache/d").exists(), "{got:?}");
        std::thread::sleep(Duration::from_millis(100));
        assert!(f.poll().is_empty() && f.pending() == (0, 0));
        let _ = std::fs::remove_dir_all(&root);
    }
}
