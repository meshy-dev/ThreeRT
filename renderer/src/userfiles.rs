//! The app's side of `files.*` and `store.*` (docs/RUNTIME.md "Files"): the files the player picked, read only, and
//! the game's own storage on this device. A game has no I/O; it asks the app, and the app answers with what the player
//! allowed (a grant: the folder or files picked) or with what is under the game's store directory, and nothing else.
//!
//! - [`Store`]: one game's store, a directory: paths checked (`files::check_store_path`), writes atomic (a temporary
//!   file beside the file, renamed over it), a quota.
//! - [`Grant`]: what a pick allowed: the folder's matching files (to 6 levels, at most 4,000, hidden ones left out) or
//!   the files picked; `files.read` reads those and nothing else.
//! - [`UserFiles`]: both as `HostServices` for a window. The work (a walk of a folder, reading and writing tens of
//!   megabytes, base64) is done on a thread of its own, one request after another in the order they were posted; the
//!   answers wait in a queue the window empties once a frame. Who asks the player is a [`Picker`]: `threert-render
//!   play` answers with the paths of its `--pick` flag ([`queued_picks`]), an app with a dialog.

use crate::files::check_store_path;
use crate::host::HostServices;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// The largest file `files.read` and `store.read` answer with, and `store.write` takes.
pub const MAX_FILE: u64 = 64 << 20;
/// A game's store holds at most this many bytes.
pub const QUOTA: u64 = 1 << 30;
/// A folder's entries: how deep (a file at depth 6 is `a/b/c/d/e/f.bsp`), how many.
pub const MAX_DEPTH: usize = 6;
pub const MAX_ENTRIES: usize = 4000;

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

// ------------------------------------------------------------------------------------------------ the store

/// A game's store: a directory of its own on this device.
pub struct Store {
    root: PathBuf,
    quota: u64,
    /// Bytes under the root, once counted (kept as writes and removes change it).
    used: Option<u64>,
    /// Names of temporary files.
    seq: u64,
}

impl Store {
    /// The store in `root` (made when first written to).
    pub fn new(root: impl Into<PathBuf>) -> Store {
        Store { root: root.into(), quota: QUOTA, used: None, seq: 0 }
    }

    /// The same with another quota (bytes).
    pub fn with_quota(root: impl Into<PathBuf>, quota: u64) -> Store {
        Store { quota, ..Store::new(root) }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Bytes under the root (every file, also the ones that are not the game's: a temporary file a crash left).
    pub fn used(&mut self) -> u64 {
        if let Some(u) = self.used {
            return u;
        }
        let mut n = 0;
        walk_all(&self.root, &mut |_, len| n += len);
        self.used = Some(n);
        n
    }

    fn file(&self, path: &str) -> Result<PathBuf, String> {
        check_store_path(path)?;
        Ok(self.root.join(path))
    }

    /// Writes the file `path` (a whole file, atomically: one who reads it sees the old bytes or the new ones).
    /// `"quota"` when the store would hold more than its quota.
    pub fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        let file = self.file(path)?;
        if bytes.len() as u64 > MAX_FILE {
            return Err("too large".into());
        }
        let old = match std::fs::symlink_metadata(&file) {
            Ok(m) if m.is_dir() => return Err(format!("{path}: a directory is there")),
            Ok(m) => m.len(),
            Err(_) => 0,
        };
        let used = self.used();
        if used - old.min(used) + bytes.len() as u64 > self.quota {
            return Err("quota".into());
        }
        let dir = file.parent().expect("a store path has a parent: the root");
        std::fs::create_dir_all(dir).map_err(|e| format!("{path}: {e}"))?;
        // (A name a store path cannot have: `~` is not among its characters, so it is never a game's file, and `list`
        // leaves it out if a crash leaves it behind.)
        self.seq += 1;
        let tmp = dir.join(format!("{}~{}-{}.tmp", file.file_name().unwrap().to_string_lossy(), std::process::id(), self.seq));
        let written = (|| -> std::io::Result<()> {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            std::fs::rename(&tmp, &file)
        })();
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            self.used = None;
            return Err(format!("{path}: {e}"));
        }
        self.used = Some(used - old.min(used) + bytes.len() as u64);
        Ok(())
    }

    /// The file `path`. `"absent"` when there is none.
    pub fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let file = self.file(path)?;
        match std::fs::symlink_metadata(&file) {
            Ok(m) if m.is_file() && m.len() > MAX_FILE => Err("too large".into()),
            Ok(m) if m.is_file() => std::fs::read(&file).map_err(|e| format!("{path}: {e}")),
            _ => Err("absent".into()),
        }
    }

    /// The files whose path starts with `prefix` (a plain string: `"maps/"` is everything under maps), sorted.
    pub fn list(&self, prefix: &str) -> Vec<(String, u64)> {
        let mut out = vec![];
        walk_all(&self.root, &mut |rel, len| {
            if rel.starts_with(prefix) && check_store_path(rel).is_ok() {
                out.push((rel.to_string(), len));
            }
        });
        out.sort();
        out
    }

    /// Removes the file `path`, or the directory `path` and everything in it. Nothing there is not an error.
    pub fn remove(&mut self, path: &str) -> Result<(), String> {
        let file = self.file(path)?;
        let r = match std::fs::symlink_metadata(&file) {
            Ok(m) if m.is_dir() => std::fs::remove_dir_all(&file),
            Ok(_) => std::fs::remove_file(&file),
            Err(_) => return Ok(()),
        };
        self.used = None;
        r.map_err(|e| format!("{path}: {e}"))
    }
}

/// Every file under `root` with its path relative to it (`/`-separated), symbolic links left out.
fn walk_all(root: &Path, f: &mut dyn FnMut(&str, u64)) {
    fn go(dir: &Path, rel: &str, f: &mut dyn FnMut(&str, u64)) {
        let Ok(list) = std::fs::read_dir(dir) else { return };
        for e in list.flatten() {
            let Ok(m) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if m.is_dir() {
                go(&e.path(), &path, f);
            } else if m.is_file() {
                f(&path, e.metadata().map_or(0, |m| m.len()));
            }
        }
    }
    go(root, "", f)
}

// ------------------------------------------------------------------------------------------------ picks

/// How the player picks: a folder (its matching files are the grant) or files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickMode {
    Folder,
    Files,
}

/// What the game asked the player for (`files.pick`).
#[derive(Clone, Debug)]
pub struct PickRequest {
    pub mode: PickMode,
    /// Lower-case extensions with the dot; empty: every file.
    pub accept: Vec<String>,
    pub title: Option<String>,
}

/// Asks the player: the folder (one path) or the files they chose, or why there are none (`"canceled"`, `"busy"`,
/// `"unsupported"`, `"denied"`, a sentence).
pub type Picker = Box<dyn FnMut(&PickRequest) -> Result<Vec<PathBuf>, String> + Send>;

/// `threert-render play --pick a,b`: the first `files.pick` is answered with `a`, the second with `b`, then each is
/// `"canceled"`. Without paths every pick is `"unsupported"`.
pub fn queued_picks(paths: Vec<PathBuf>) -> Picker {
    let none = paths.is_empty();
    let mut queue = std::collections::VecDeque::from(paths);
    Box::new(move |_| match queue.pop_front() {
        Some(p) => Ok(vec![p]),
        None => Err(if none { "unsupported" } else { "canceled" }.into()),
    })
}

/// What a pick allowed.
#[derive(Clone, Debug)]
pub struct Grant {
    /// The folder's name, or the file's.
    pub name: String,
    /// Paths as the game names them (relative to the folder, `/`-separated; a file's name) and their sizes, sorted.
    pub entries: Vec<(String, u64)>,
    files: HashMap<String, PathBuf>,
}

fn accepted(name: &str, accept: &[String]) -> bool {
    let lower = name.to_ascii_lowercase();
    accept.is_empty() || accept.iter().any(|a| lower.ends_with(a.as_str()))
}

fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

impl Grant {
    /// A folder: its files that match `accept`, at most `MAX_DEPTH` levels down and `MAX_ENTRIES` of them, hidden
    /// files and directories and symbolic links left out.
    pub fn folder(dir: &Path, accept: &[String]) -> Result<Grant, String> {
        let dir = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
        if !dir.is_dir() {
            return Err(format!("{} is not a folder", name_of(&dir)));
        }
        let mut g = Grant { name: name_of(&dir), entries: vec![], files: HashMap::new() };
        fn go(g: &mut Grant, dir: &Path, rel: &str, depth: usize, accept: &[String]) {
            let Ok(list) = std::fs::read_dir(dir) else { return };
            let mut list: Vec<_> = list.flatten().collect();
            list.sort_by_key(|e| e.file_name());
            for e in list {
                if g.entries.len() >= MAX_ENTRIES {
                    return;
                }
                let name = e.file_name().to_string_lossy().into_owned();
                let Ok(t) = e.file_type() else { continue };
                if name.starts_with('.') {
                    continue;
                }
                let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                if t.is_dir() && depth < MAX_DEPTH {
                    go(g, &e.path(), &path, depth + 1, accept);
                } else if t.is_file() && accepted(&name, accept) {
                    g.entries.push((path.clone(), e.metadata().map_or(0, |m| m.len())));
                    g.files.insert(path, e.path());
                }
            }
        }
        go(&mut g, &dir, "", 1, accept);
        g.entries.sort();
        Ok(g)
    }

    /// Files the player chose: each by its name (the first of two of one name). A folder among them stands for the
    /// files in it (not below).
    pub fn files(paths: &[PathBuf], accept: &[String]) -> Result<Grant, String> {
        let mut g = Grant { name: paths.first().map(|p| name_of(p)).unwrap_or_default(), entries: vec![], files: HashMap::new() };
        let mut add = |p: PathBuf| {
            let name = name_of(&p);
            if !name.starts_with('.') && accepted(&name, accept) && !g.files.contains_key(&name) {
                if let Some(m) = std::fs::metadata(&p).ok().filter(|m| m.is_file()) {
                    g.entries.push((name.clone(), m.len()));
                    g.files.insert(name, p);
                }
            }
        };
        for p in paths {
            if p.is_dir() {
                let mut list: Vec<PathBuf> = std::fs::read_dir(p).map_err(|e| format!("{}: {e}", p.display()))?.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| e.path()).collect();
                list.sort();
                list.into_iter().for_each(&mut add);
            } else if p.is_file() {
                add(p.clone());
            } else {
                return Err(format!("{}: no such file", name_of(p)));
            }
        }
        g.entries.sort();
        Ok(g)
    }

    /// The file `path` of the grant: `"denied"` for one that is not among its entries.
    pub fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let file = self.files.get(path).ok_or("denied")?;
        let len = std::fs::metadata(file).map_err(|e| format!("{path}: {e}"))?.len();
        if len > MAX_FILE {
            return Err("too large".into());
        }
        std::fs::read(file).map_err(|e| format!("{path}: {e}"))
    }
}

// ------------------------------------------------------------------------------------------------ the services

/// The requests' side that does the work: the store, the grants of the session, the picker.
pub struct Requests {
    pub store: Option<Store>,
    picker: Option<Picker>,
    grants: HashMap<String, Grant>,
}

impl Requests {
    pub fn new(store: Option<Store>, picker: Option<Picker>) -> Requests {
        Requests { store, picker, grants: HashMap::new() }
    }

    /// The families answered: `"files"` with a picker, `"store"` with a store.
    pub fn caps(&self) -> Vec<&'static str> {
        let mut c = vec![];
        if self.picker.is_some() {
            c.push("files");
        }
        if self.store.is_some() {
            c.push("store");
        }
        c
    }

    /// Whether a message is a request of these families (`files.pick`, `files.read`, `store.*`).
    pub fn takes(m: &Value) -> bool {
        matches!(m["t"].as_str(), Some("files.pick" | "files.read" | "store.write" | "store.read" | "store.list" | "store.remove"))
    }

    /// The answer to a request (`None`: not one of these families').
    pub fn answer(&mut self, m: &Value) -> Option<Value> {
        let id = m["id"].clone();
        let fail = |t: &str, reason: String| json!({ "t": t, "id": id, "ok": false, "reason": reason });
        let t = m["t"].as_str()?;
        Some(match t {
            "files.pick" => match self.pick(m) {
                Ok((pick, g)) => json!({ "t": "files.picked", "id": id, "ok": true, "pick": pick, "name": g.name, "entries": entries(&g.entries) }),
                Err(e) => fail("files.picked", e),
            },
            "files.read" => {
                let r = match (m["pick"].as_str().and_then(|p| self.grants.get(p)), m["path"].as_str()) {
                    (Some(g), Some(path)) => g.read(path),
                    _ => Err("denied".into()),
                };
                match r {
                    Ok(b) => json!({ "t": "files.data", "id": id, "ok": true, "data": b64().encode(&b), "size": b.len() }),
                    Err(e) => fail("files.data", e),
                }
            }
            "store.write" | "store.read" | "store.list" | "store.remove" => {
                let answer = match t {
                    "store.read" => "store.data",
                    "store.list" => "store.listed",
                    _ => "store.done",
                };
                let Some(store) = self.store.as_mut() else { return Some(fail(answer, "unsupported".into())) };
                let path = m["path"].as_str().unwrap_or("");
                match t {
                    "store.write" => {
                        let bytes = match (m["data"].as_str(), m["text"].as_str()) {
                            (Some(d), _) => b64().decode(d).map_err(|e| format!("data is not base64: {e}")),
                            (None, Some(text)) => Ok(text.as_bytes().to_vec()),
                            (None, None) => Err("no data and no text".into()),
                        };
                        match bytes.and_then(|b| store.write(path, &b)) {
                            Ok(()) => json!({ "t": answer, "id": id, "ok": true }),
                            Err(e) => fail(answer, e),
                        }
                    }
                    "store.read" => match store.read(path) {
                        Ok(b) if m["as"] == "text" => match String::from_utf8(b) {
                            Ok(text) => json!({ "t": answer, "id": id, "ok": true, "size": text.len(), "text": text }),
                            Err(_) => fail(answer, "not text".into()),
                        },
                        Ok(b) => json!({ "t": answer, "id": id, "ok": true, "size": b.len(), "data": b64().encode(&b) }),
                        Err(e) => fail(answer, e),
                    },
                    "store.list" => json!({ "t": answer, "id": id, "ok": true, "entries": entries(&store.list(m["prefix"].as_str().unwrap_or(""))) }),
                    _ => match store.remove(path) {
                        Ok(()) => json!({ "t": answer, "id": id, "ok": true }),
                        Err(e) => fail(answer, e),
                    },
                }
            }
            _ => return None,
        })
    }

    fn pick(&mut self, m: &Value) -> Result<(String, Grant), String> {
        let mode = match m["mode"].as_str() {
            Some("folder") => PickMode::Folder,
            Some("files") => PickMode::Files,
            _ => return Err("mode is \"folder\" or \"files\"".into()),
        };
        let accept = m["accept"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(|x| x.to_ascii_lowercase()).collect()).unwrap_or_default();
        let req = PickRequest { mode, accept, title: m["title"].as_str().map(String::from) };
        let picker = self.picker.as_mut().ok_or("unsupported")?;
        let paths = picker(&req)?;
        let g = match mode {
            PickMode::Folder => Grant::folder(paths.first().ok_or("canceled")?, &req.accept)?,
            PickMode::Files => Grant::files(&paths, &req.accept)?,
        };
        let pick = format!("pick-{}", self.grants.len() + 1);
        self.grants.insert(pick.clone(), g.clone());
        Ok((pick, g))
    }
}

fn entries(list: &[(String, u64)]) -> Value {
    list.iter().map(|(path, size)| json!({ "path": path, "size": size })).collect()
}

/// `files.*` and `store.*` for a window: [`Requests`] on a thread of its own.
pub struct UserFiles {
    caps: Vec<&'static str>,
    tx: mpsc::Sender<Value>,
    rx: mpsc::Receiver<Value>,
}

impl UserFiles {
    /// The store in `store` (none: no `store.*`), the player asked by `picker` (none: no `files.*`).
    pub fn new(store: Option<PathBuf>, picker: Option<Picker>) -> UserFiles {
        let mut r = Requests::new(store.map(Store::new), picker);
        let caps = r.caps();
        let (tx, requests) = mpsc::channel::<Value>();
        let (answers, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("threert-userfiles".into())
            .spawn(move || {
                for m in requests {
                    if let Some(a) = r.answer(&m) {
                        if answers.send(a).is_err() {
                            break;
                        }
                    }
                }
            })
            .expect("a thread for the user's files");
        UserFiles { caps, tx, rx }
    }
}

impl HostServices for UserFiles {
    fn post(&mut self, m: &Value) {
        if Requests::takes(m) {
            let _ = self.tx.send(m.clone());
        }
    }
    fn poll(&mut self) -> Vec<Value> {
        self.rx.try_iter().collect()
    }
    fn caps(&self) -> Vec<&'static str> {
        self.caps.clone()
    }
}

/// A post as the window shows it on stdout: a `store.write`'s bytes as their count.
pub fn without_payload(m: &Value) -> Value {
    if m["t"] != "store.write" {
        return m.clone();
    }
    let mut m = m.clone();
    if let Some(o) = m.as_object_mut() {
        for k in ["data", "text"] {
            if let Some(v) = o.remove(k) {
                o.insert(format!("{k}Length"), v.as_str().map_or(0, str::len).into());
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("threert-userfiles-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn ask(r: &mut Requests, m: Value) -> Value {
        r.answer(&m).expect("an answer")
    }

    #[test]
    fn store_paths_are_checked() {
        for bad in ["", "/abs", "a//b", "a/", "./a", "a/./b", "..", "a/../b", "a b", "a\\b", "maps/é.bsp", "c:x", "a~b", &"x".repeat(201)] {
            assert!(check_store_path(bad).is_err(), "{bad:?} is refused");
        }
        for good in ["a", "maps/de_dust2/map.glb", ".hidden", "a.b-c_d/E9", &"x".repeat(200)] {
            assert!(check_store_path(good).is_ok(), "{good:?} is a store path");
        }
        let root = temp("paths");
        let mut r = Requests::new(Some(Store::new(&root)), None);
        for bad in ["../outside", "/etc/passwd", "a/../../b", "x y"] {
            let a = ask(&mut r, json!({ "t": "store.write", "id": 1, "path": bad, "text": "no" }));
            assert_eq!((a["t"].as_str(), a["ok"].as_bool()), (Some("store.done"), Some(false)), "{bad}: {a}");
            let a = ask(&mut r, json!({ "t": "store.read", "id": 2, "path": bad, "as": "text" }));
            assert_eq!(a["ok"], false, "{a}");
            let a = ask(&mut r, json!({ "t": "store.remove", "id": 3, "path": bad }));
            assert_eq!(a["ok"], false, "{a}");
        }
        assert!(!root.parent().unwrap().join("outside").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_store_writes_reads_lists_and_removes() {
        let root = temp("store");
        let mut r = Requests::new(Some(Store::new(root.join("game"))), None);
        assert_eq!(r.caps(), vec!["store"]);
        let a = ask(&mut r, json!({ "t": "store.write", "id": "w1", "path": "maps/de_x/map.glb", "data": b64().encode([0u8, 1, 2, 255]) }));
        assert_eq!(a, json!({ "t": "store.done", "id": "w1", "ok": true }));
        ask(&mut r, json!({ "t": "store.write", "id": "w2", "path": "maps/de_x/info.json", "text": "{\"name\":\"x\"}" }));
        ask(&mut r, json!({ "t": "store.write", "id": "w3", "path": "settings.json", "text": "é" }));
        let a = ask(&mut r, json!({ "t": "store.read", "id": "r1", "path": "maps/de_x/map.glb", "as": "base64" }));
        assert_eq!(a, json!({ "t": "store.data", "id": "r1", "ok": true, "size": 4, "data": "AAEC/w==" }));
        let a = ask(&mut r, json!({ "t": "store.read", "id": "r2", "path": "settings.json", "as": "text" }));
        assert_eq!((a["text"].as_str(), a["size"].as_u64()), (Some("é"), Some(2)));
        let a = ask(&mut r, json!({ "t": "store.read", "id": "r3", "path": "maps/de_x/map.glb", "as": "text" }));
        assert_eq!(a["reason"], "not text");
        let a = ask(&mut r, json!({ "t": "store.read", "id": "r4", "path": "nothing.here", "as": "text" }));
        assert_eq!(a, json!({ "t": "store.data", "id": "r4", "ok": false, "reason": "absent" }));
        let a = ask(&mut r, json!({ "t": "store.list", "id": "l1" }));
        assert_eq!(a["entries"], json!([{ "path": "maps/de_x/info.json", "size": 12 }, { "path": "maps/de_x/map.glb", "size": 4 }, { "path": "settings.json", "size": 2 }]));
        let a = ask(&mut r, json!({ "t": "store.list", "id": "l2", "prefix": "maps/" }));
        assert_eq!(a["entries"].as_array().unwrap().len(), 2);
        // A leftover temporary file (a crash in a write) is not the game's.
        std::fs::write(root.join("game/maps/map.glb~1-1.tmp"), "x").unwrap();
        assert_eq!(ask(&mut r, json!({ "t": "store.list", "id": "l3", "prefix": "maps/" }))["entries"].as_array().unwrap().len(), 2);
        // Overwritten whole.
        ask(&mut r, json!({ "t": "store.write", "id": "w4", "path": "settings.json", "text": "{}" }));
        assert_eq!(ask(&mut r, json!({ "t": "store.read", "id": "r5", "path": "settings.json", "as": "text" }))["text"], "{}");
        // A directory and what is in it; then nothing there, which is no error.
        assert_eq!(ask(&mut r, json!({ "t": "store.remove", "id": "d1", "path": "maps" })), json!({ "t": "store.done", "id": "d1", "ok": true }));
        assert_eq!(ask(&mut r, json!({ "t": "store.remove", "id": "d2", "path": "maps" }))["ok"], true);
        assert_eq!(ask(&mut r, json!({ "t": "store.list", "id": "l4" }))["entries"], json!([{ "path": "settings.json", "size": 2 }]));
        // Not base64, nothing to write, a directory in the way.
        assert_eq!(ask(&mut r, json!({ "t": "store.write", "id": "w5", "path": "a", "data": "**" }))["ok"], false);
        assert_eq!(ask(&mut r, json!({ "t": "store.write", "id": "w6", "path": "a" }))["ok"], false);
        ask(&mut r, json!({ "t": "store.write", "id": "w7", "path": "dir/f", "text": "x" }));
        assert_eq!(ask(&mut r, json!({ "t": "store.write", "id": "w8", "path": "dir", "text": "x" }))["ok"], false);
        // Messages of other families are not these services'.
        assert!(r.answer(&json!({ "t": "net.open", "id": 1 })).is_none());
        // Without a store: every store request says so.
        let mut none = Requests::new(None, None);
        assert!(none.caps().is_empty());
        assert_eq!(ask(&mut none, json!({ "t": "store.list", "id": 9 })), json!({ "t": "store.listed", "id": 9, "ok": false, "reason": "unsupported" }));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn writes_are_atomic_and_held_to_the_quota() {
        let root = temp("quota");
        let mut s = Store::with_quota(&root, 1000);
        s.write("a.bin", &[1; 600]).unwrap();
        assert_eq!(s.write("b.bin", &[2; 500]), Err("quota".into()));
        assert!(!root.join("b.bin").exists(), "a refused write leaves nothing");
        // Replacing a file counts its new size, not both.
        s.write("a.bin", &[3; 900]).unwrap();
        assert_eq!(s.read("a.bin").unwrap(), vec![3; 900]);
        assert_eq!(s.used(), 900);
        s.remove("a.bin").unwrap();
        s.write("b.bin", &[2; 1000]).unwrap();
        assert_eq!(s.used(), 1000);
        // The count is the disk's: a store opened anew counts what is there.
        assert_eq!(Store::with_quota(&root, 1000).write("c", b"x"), Err("quota".into()));
        // No temporary file is left beside a written one.
        let names: Vec<String> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["b.bin".to_string()]);
        // A write that fails (its directory is a file) leaves the old file and no temporary one.
        std::fs::write(root.join("f"), "file").unwrap();
        let mut s = Store::new(&root);
        assert!(s.write("f/g", b"x").is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        // A reader sees the old bytes or the new ones, never a part (the rename): many writes while a thread reads.
        let reader = {
            let path = root.join("big.bin");
            std::fs::write(&path, vec![0u8; 1 << 20]).unwrap();
            std::thread::spawn(move || {
                for _ in 0..200 {
                    let b = std::fs::read(&path).unwrap();
                    assert_eq!(b.len(), 1 << 20);
                    assert!(b.iter().all(|&x| x == b[0]), "a whole file");
                }
            })
        };
        for k in 1..40u8 {
            s.write("big.bin", &vec![k; 1 << 20]).unwrap();
        }
        reader.join().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn picks_answer_from_the_queue_and_reads_stay_inside_the_grant() {
        let root = temp("picks");
        let game = root.join("cstrike");
        for (p, body) in [("maps/de_x.bsp", "BSP"), ("maps/DE_Y.BSP", "BSP2"), ("maps/readme.txt", "no"), ("cstrike.wad", "WAD"), (".hidden/z.bsp", "no"), ("a/b/c/d/e/f.bsp", "deep6"), ("a/b/c/d/e/f/g.bsp", "deep7")] {
            std::fs::create_dir_all(game.join(p).parent().unwrap()).unwrap();
            std::fs::write(game.join(p), body).unwrap();
        }
        std::fs::write(root.join("secret.bsp"), "outside").unwrap();
        let picker = queued_picks(vec![game.clone(), game.join("maps/de_x.bsp")]);
        let mut r = Requests::new(None, Some(picker));
        assert_eq!(r.caps(), vec!["files"]);
        let a = ask(&mut r, json!({ "t": "files.pick", "id": "p1", "mode": "folder", "accept": [".bsp", ".WAD"], "title": "Your Counter-Strike folder" }));
        assert_eq!(a["ok"], true, "{a}");
        assert_eq!(a["name"], "cstrike");
        assert_eq!(a["entries"], json!([{ "path": "a/b/c/d/e/f.bsp", "size": 5 }, { "path": "cstrike.wad", "size": 3 }, { "path": "maps/DE_Y.BSP", "size": 4 }, { "path": "maps/de_x.bsp", "size": 3 }]));
        let pick = a["pick"].as_str().unwrap().to_string();
        let a = ask(&mut r, json!({ "t": "files.read", "id": "r1", "pick": pick, "path": "maps/de_x.bsp" }));
        assert_eq!(a, json!({ "t": "files.data", "id": "r1", "ok": true, "data": b64().encode("BSP"), "size": 3 }));
        for denied in ["maps/readme.txt", "../secret.bsp", "/etc/passwd", ".hidden/z.bsp", "a/b/c/d/e/f/g.bsp", "maps/../../secret.bsp"] {
            let a = ask(&mut r, json!({ "t": "files.read", "id": "r2", "pick": pick, "path": denied }));
            assert_eq!(a, json!({ "t": "files.data", "id": "r2", "ok": false, "reason": "denied" }), "{denied}");
        }
        assert_eq!(ask(&mut r, json!({ "t": "files.read", "id": "r3", "pick": "pick-9", "path": "maps/de_x.bsp" }))["reason"], "denied");
        // The second answer: one file.
        let a = ask(&mut r, json!({ "t": "files.pick", "id": "p2", "mode": "files", "accept": [".bsp"] }));
        assert_eq!((a["name"].as_str(), a["entries"].clone()), (Some("de_x.bsp"), json!([{ "path": "de_x.bsp", "size": 3 }])));
        let a = ask(&mut r, json!({ "t": "files.read", "id": "r4", "pick": a["pick"], "path": "de_x.bsp" }));
        assert_eq!(a["data"], b64().encode("BSP"));
        // The queue is empty: the player closed the picker.
        assert_eq!(ask(&mut r, json!({ "t": "files.pick", "id": "p3", "mode": "folder" })), json!({ "t": "files.picked", "id": "p3", "ok": false, "reason": "canceled" }));
        assert_eq!(ask(&mut r, json!({ "t": "files.pick", "id": "p4", "mode": "everything" }))["ok"], false);
        // Without `--pick`: unsupported. Without a picker at all: the same.
        let mut r = Requests::new(None, Some(queued_picks(vec![])));
        assert_eq!(ask(&mut r, json!({ "t": "files.pick", "id": 1, "mode": "folder" }))["reason"], "unsupported");
        let mut r = Requests::new(None, None);
        assert_eq!(ask(&mut r, json!({ "t": "files.pick", "id": 1, "mode": "folder" }))["reason"], "unsupported");
        // A folder asked for, a file given.
        let mut r = Requests::new(None, Some(queued_picks(vec![root.join("secret.bsp")])));
        assert_eq!(ask(&mut r, json!({ "t": "files.pick", "id": 1, "mode": "folder" }))["ok"], false);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_has_at_most_its_entries() {
        let root = temp("many");
        for i in 0..MAX_ENTRIES + 10 {
            std::fs::write(root.join(format!("{i:05}.bsp")), "").unwrap();
        }
        assert_eq!(Grant::folder(&root, &[]).unwrap().entries.len(), MAX_ENTRIES);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_services_answer_on_their_thread_in_order() {
        let root = temp("thread");
        std::fs::write(root.join("x.bsp"), "BSP").unwrap();
        let mut s = UserFiles::new(Some(root.join("store")), Some(queued_picks(vec![root.clone()])));
        assert_eq!(s.caps(), vec!["files", "store"]);
        s.post(&json!({ "t": "store.write", "id": 1, "path": "a.txt", "text": "one" }));
        s.post(&json!({ "t": "store.read", "id": 2, "path": "a.txt", "as": "text" }));
        s.post(&json!({ "t": "files.pick", "id": 3, "mode": "folder", "accept": [".bsp"] }));
        s.post(&json!({ "t": "net.open", "id": 4, "url": "wss://example.org" }));
        let mut got = vec![];
        let t0 = std::time::Instant::now();
        while got.len() < 3 && t0.elapsed().as_secs() < 10 {
            got.extend(s.poll());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(got.iter().map(|m| m["id"].as_u64().unwrap()).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(got[1]["text"], "one");
        assert_eq!(got[2]["entries"], json!([{ "path": "x.bsp", "size": 3 }]));
        let w = without_payload(&json!({ "t": "store.write", "id": 1, "path": "a", "data": "AAAA" }));
        assert_eq!(w, json!({ "t": "store.write", "id": 1, "path": "a", "dataLength": 4 }));
        let _ = std::fs::remove_dir_all(root);
    }
}
