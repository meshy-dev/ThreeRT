//! Sound for games: the `audio.*` messages of the host channel (docs/RUNTIME.md "Sound"; packages/gameplay
//! src/audio.ts), played by the engine. Every app that embeds the engine (`threert-render play`, the ThreeRT clients
//! on macOS, iOS, Linux, Android) hands the game's posts and its camera to an [`Audio`]; the decoding (decode.rs), the
//! placing and the mixing (mix.rs) are this module's, so a game sounds the same wherever it runs. Only the device
//! differs a platform (out.rs).
//!
//! ```text
//! game:  audio.play("assets/sounds/shot.ogg", { at: muzzle, gain: 0.9, pitch: 1.03 })
//!        -> { t: "audio.play", sound: "assets/sounds/shot.ogg", at: [x, y, z], gain: 0.9, pitch: 1.03 }
//! app:   audio.post(&message)            each message the game posted
//!        audio.listener(&camera)         once a frame: the ears are the camera the frame was drawn from
//!        audio.take_log()                what to tell the developer (a sound that is missing, the device)
//! ```
//!
//! A sound is a file of the game's package (the path is the package's, as the scene's files are), decoded once and
//! kept until the game closes: in the background after `audio.preload`, otherwise when first played. The device opens
//! when the first sound plays and closes with the game (`close`) or while the app is in the background (`set_paused`);
//! where none opens the game goes on, silent, and the log says why once.

mod decode;
mod mix;
mod out;

pub use decode::{decode, EXTENSIONS};
pub use mix::{pan_gains, place, Listener, Sound, DEFAULT_FALLOFF, MAX_COPIES, MAX_VOICES};

use glam::Vec3;
use mix::{Cmd, Mixer, Play};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Plays a frame may start (more are dropped: a shotgun's pellets on a wall are one sound each).
pub const MAX_PLAYS_A_FRAME: usize = 32;

enum Device {
    Closed,
    Opening(out::Output),
    Open(#[allow(dead_code)] out::Output),
    /// It did not open: not tried again until the next game.
    Failed,
}

pub struct Audio {
    tx: Sender<Cmd>,
    mixer: Arc<Mutex<Mixer>>,
    device: Device,
    root: Option<PathBuf>,
    sounds: HashMap<String, Option<Arc<Sound>>>,
    decoded_tx: Sender<(String, Result<Sound, String>)>,
    decoded: Receiver<(String, Result<Sound, String>)>,
    listener: Listener,
    volume: f32,
    paused: bool,
    plays: usize,
    log: Vec<(&'static str, String)>,
    /// What the device was opened as, for the app to show (None: not open).
    pub device_name: Option<String>,
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl Audio {
    /// No device yet: it opens when a game plays its first sound.
    pub fn new() -> Audio {
        let (tx, rx) = channel();
        let (decoded_tx, decoded) = channel();
        Audio {
            tx,
            mixer: Arc::new(Mutex::new(Mixer::new(48_000, rx))),
            device: Device::Closed,
            root: None,
            sounds: HashMap::new(),
            decoded_tx,
            decoded,
            listener: Listener::default(),
            volume: 1.0,
            paused: false,
            plays: 0,
            log: vec![],
            device_name: None,
        }
    }

    /// A game opened: its files are under `root` (the package's asset root). What the last one played stops.
    pub fn open(&mut self, root: &Path) {
        self.close();
        self.root = Some(root.to_path_buf());
    }

    /// The game closed: its sounds stop, are forgotten, and the device closes.
    pub fn close(&mut self) {
        let _ = self.tx.send(Cmd::Stop(None, 0.0));
        self.sounds.clear();
        self.root = None;
        if !matches!(self.device, Device::Closed) {
            // (The mixer drops the stopped voices on its next block; the device goes now.)
            self.device = Device::Closed;
            self.device_name = None;
            if let Ok(mut m) = self.mixer.lock() {
                let mut scratch = [0.0f32; 2];
                m.render(&mut scratch, 2);
            }
        }
    }

    /// The app's volume, 0 to 1 (0: muted; the device stays as it is).
    pub fn set_volume(&mut self, v: f32) {
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 };
        if v != self.volume {
            self.volume = v;
            let _ = self.tx.send(Cmd::Volume(v));
        }
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// The app went to the background (true) or came back: the device is let go of and opened again; looping sounds
    /// go on where they were, one-shots that came in between are dropped.
    pub fn set_paused(&mut self, paused: bool) {
        if paused == self.paused {
            return;
        }
        self.paused = paused;
        let _ = self.tx.send(Cmd::Paused(paused));
        if paused {
            if matches!(self.device, Device::Open(_) | Device::Opening(_)) {
                self.device = Device::Closed;
                self.device_name = None;
            }
        } else if self.root.is_some() && self.mixer.lock().map(|m| m.voices() > 0).unwrap_or(false) {
            self.start_device();
        }
    }

    /// Where the ears are: the camera the frame was drawn from (position, forward, right). Once a frame, after the
    /// game's messages of that frame.
    pub fn listener(&mut self, pos: Vec3, forward: Vec3, right: Vec3) {
        self.plays = 0;
        let l = Listener { pos, forward: forward.normalize_or(Vec3::NEG_Z), right: right.normalize_or(Vec3::X) };
        if l != self.listener {
            self.listener = l;
            let _ = self.tx.send(Cmd::Listener(l));
        }
        self.poll();
    }

    /// What happened that the developer should know, since the last call: (level, text).
    pub fn take_log(&mut self) -> Vec<(&'static str, String)> {
        self.poll();
        std::mem::take(&mut self.log)
    }

    /// One message the game posted. Not an `audio.*` one: false, not this module's.
    pub fn post(&mut self, m: &Value) -> bool {
        let Some(t) = m["t"].as_str() else { return false };
        match t {
            "audio.play" => self.play(m),
            "audio.stop" => {
                let key = m["key"].as_str().map(String::from);
                let _ = self.tx.send(Cmd::Stop(key, mix::stop_fade(m["fade"].as_f64())));
            }
            "audio.move" => {
                if let (Some(key), Some(at)) = (m["key"].as_str(), vec3(&m["at"])) {
                    let _ = self.tx.send(Cmd::Move(key.to_string(), at));
                }
            }
            "audio.preload" => {
                for s in m["sounds"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    self.preload(s);
                }
            }
            _ => return t.starts_with("audio."),
        }
        true
    }

    fn play(&mut self, m: &Value) {
        let Some(name) = m["sound"].as_str() else {
            self.log.push(("warn", "audio.play without a sound".into()));
            return;
        };
        if self.plays >= MAX_PLAYS_A_FRAME {
            return;
        }
        self.plays += 1;
        let Some(sound) = self.sound(name) else { return };
        let num = |k: &str, d: f32| m[k].as_f64().map(|v| v as f32).filter(|v| v.is_finite()).unwrap_or(d);
        let play = Play {
            sound,
            at: vec3(&m["at"]),
            gain: num("gain", 1.0).clamp(0.0, 4.0),
            pitch: num("pitch", 1.0).clamp(0.25, 4.0),
            looped: m["loop"].as_bool().unwrap_or(false),
            key: m["key"].as_str().map(String::from),
            falloff: num("falloff", DEFAULT_FALLOFF).clamp(0.05, 1000.0),
        };
        if !self.paused && matches!(self.device, Device::Closed) {
            self.start_device();
        }
        let _ = self.tx.send(Cmd::Play(play));
    }

    fn start_device(&mut self) {
        match out::open(self.mixer.clone()) {
            Ok(o) => self.device = Device::Opening(o),
            Err(e) => {
                self.log.push(("warn", format!("no sound: {e}")));
                self.device = Device::Failed;
            }
        }
    }

    /// The decoded sound `name`, decoded now if it is not yet (None: missing or unreadable, said once).
    fn sound(&mut self, name: &str) -> Option<Arc<Sound>> {
        self.poll();
        if let Some(s) = self.sounds.get(name) {
            return s.clone();
        }
        let r = self.path(name).and_then(|p| std::fs::read(&p).map_err(|e| format!("{name}: {e}"))).and_then(|b| decode(name, b).map_err(|e| format!("{e:#}")));
        let s = match r {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => {
                self.log.push(("warn", format!("sound {e}")));
                None
            }
        };
        self.sounds.insert(name.to_string(), s.clone());
        s
    }

    /// Decodes `name` on a thread of its own, unless it is decoded or on its way.
    fn preload(&mut self, name: &str) {
        if self.sounds.contains_key(name) {
            return;
        }
        let path = match self.path(name) {
            Ok(p) => p,
            Err(e) => {
                self.log.push(("warn", format!("sound {e}")));
                self.sounds.insert(name.to_string(), None);
                return;
            }
        };
        // (On its way: an entry is made when it arrives; a play before that decodes it itself.)
        let (tx, n) = (self.decoded_tx.clone(), name.to_string());
        std::thread::spawn(move || {
            let r = std::fs::read(&path).map_err(|e| format!("{n}: {e}")).and_then(|b| decode(&n, b).map_err(|e| format!("{e:#}")));
            let _ = tx.send((n, r));
        });
    }

    fn path(&self, name: &str) -> Result<PathBuf, String> {
        let root = self.root.as_ref().ok_or_else(|| format!("{name}: no game is open"))?;
        let rel = Path::new(name.trim_start_matches("./"));
        if name.is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(format!("{name}: a sound is a path inside the game's package"));
        }
        Ok(root.join(rel))
    }

    fn poll(&mut self) {
        while let Ok((name, r)) = self.decoded.try_recv() {
            if self.root.is_none() || self.sounds.contains_key(&name) {
                continue;
            }
            let s = match r {
                Ok(s) => Some(Arc::new(s)),
                Err(e) => {
                    self.log.push(("warn", format!("sound {e}")));
                    None
                }
            };
            self.sounds.insert(name, s);
        }
        if let Device::Opening(o) = &self.device {
            match o.ready.try_recv() {
                Ok(Ok((rate, channels, name))) => {
                    self.log.push(("info", format!("sound: {name}, {rate} Hz, {channels} channels")));
                    self.device_name = Some(name);
                    if let Device::Opening(o) = std::mem::replace(&mut self.device, Device::Closed) {
                        self.device = Device::Open(o);
                    }
                }
                Ok(Err(e)) => {
                    self.log.push(("warn", format!("no sound: {e}")));
                    self.device = Device::Failed;
                }
                Err(_) => {}
            }
        }
    }
}

fn vec3(v: &Value) -> Option<Vec3> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    let f = |i: usize| a[i].as_f64().map(|x| x as f32).filter(|x| x.is_finite());
    Some(Vec3::new(f(0)?, f(1)?, f(2)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plays_a_package_sound_and_refuses_paths_outside_it() {
        let dir = std::env::temp_dir().join(format!("threert-audio-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let samples: Vec<i16> = (0..2205).map(|i| ((i as f32 * 0.1).sin() * 8000.0) as i16).collect();
        std::fs::write(dir.join("assets/a.wav"), decode::tests_wav(22_050, &samples)).unwrap();
        let mut a = Audio::new();
        a.open(&dir);
        assert!(a.sound("assets/a.wav").is_some());
        assert!(a.sound("../etc/passwd").is_none());
        assert!(a.sound("assets/missing.wav").is_none());
        let log = a.take_log();
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(a.post(&json!({ "t": "audio.stop" })));
        assert!(!a.post(&json!({ "t": "hud" })));
        a.close();
        assert!(a.sounds.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
