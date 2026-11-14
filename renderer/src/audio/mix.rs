//! The mixer: every voice a game plays, placed against the listener and summed into the device's frames. It runs in the
//! output's callback (out.rs) and takes what the app asks of it as commands over a channel, never a lock: the app's
//! thread and the device's never wait for each other.
//!
//! How a sound is heard (docs/RUNTIME.md "Sound", the contract every app that plays sound keeps):
//!
//! - **Distance:** a sound at `at` keeps its level out to `falloff` metres (2 by default), then falls as falloff /
//!   distance (−6 dB a doubling: 20 m from a falloff of 2 is −20 dB) and fades out between 100 and 120 m.
//! - **Direction:** panned by the angle from the listener's forward (equal power, never more than 80 % to one side:
//!   both ears hear a shot); within `falloff` it comes back towards the middle. Behind the listener and far away it is
//!   duller (a low pass from 18 kHz down with distance, halved behind).
//! - **Head-relative:** a sound without `at` (a UI tick, the player's own breath) is in the middle at its gain.
//! - **Voices:** 48 at once; a ninth copy of one sound ends its oldest; past 48 the quietest one-shot goes. Changes in
//!   level are ramped over the block (no clicks); a stop fades (50 ms unless the game says).
//! - **Master:** the app's volume, then a soft limiter above −2 dB.

use glam::Vec3;
use std::collections::VecDeque;
use std::f32::consts::{FRAC_PI_4, TAU};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

pub const MAX_VOICES: usize = 48;
pub const MAX_COPIES: usize = 8;
pub const DEFAULT_FALLOFF: f32 = 2.0;
pub const FADE_FROM: f32 = 100.0;
pub const SILENT_AT: f32 = 120.0;
const MAX_PAN: f32 = 0.8;
const DEFAULT_STOP_FADE: f32 = 0.05;

/// A decoded sound: interleaved f32 frames at its own rate, one or two channels.
#[derive(Debug)]
pub struct Sound {
    pub name: String,
    pub rate: u32,
    pub channels: usize,
    pub data: Vec<f32>,
}

impl Sound {
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels.max(1)
    }
    pub fn seconds(&self) -> f32 {
        self.frames() as f32 / self.rate.max(1) as f32
    }
}

/// Where the ears are: the camera the frame was drawn from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub pos: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
}

impl Default for Listener {
    fn default() -> Self {
        Listener { pos: Vec3::ZERO, forward: Vec3::NEG_Z, right: Vec3::X }
    }
}

/// A sound to start.
#[derive(Clone, Debug)]
pub struct Play {
    pub sound: Arc<Sound>,
    pub at: Option<Vec3>,
    pub gain: f32,
    pub pitch: f32,
    pub looped: bool,
    pub key: Option<String>,
    pub falloff: f32,
}

/// What the app asks of the mixer.
#[derive(Debug)]
pub enum Cmd {
    Play(Play),
    /// Fades out the voices of `key` (all of them: None), over seconds.
    Stop(Option<String>, f32),
    /// Moves the voices of `key`.
    Move(String, Vec3),
    Listener(Listener),
    Volume(f32),
    /// Paused: the device gets silence and voices stand still (the app in the background).
    Paused(bool),
}

/// Gain and pan of a sound at `at` for `l` (the contract above), and the low pass's cutoff in Hz.
pub fn place(l: &Listener, at: Vec3, falloff: f32) -> (f32, f32, f32) {
    let falloff = falloff.max(0.05);
    let to = at - l.pos;
    let d = to.length();
    let mut gain = if d <= falloff { 1.0 } else { falloff / d };
    if d > FADE_FROM {
        gain *= ((SILENT_AT - d) / (SILENT_AT - FADE_FROM)).clamp(0.0, 1.0);
    }
    if d < 1e-4 {
        return (gain, 0.0, 18_000.0);
    }
    let dir = to / d;
    let x = dir.dot(l.right);
    let z = dir.dot(l.forward);
    let pan = (x * (d / falloff).min(1.0)).clamp(-1.0, 1.0) * MAX_PAN;
    let behind = (-z).max(0.0);
    let cutoff = 18_000.0 / (1.0 + d / 40.0) * (1.0 - 0.5 * behind);
    (gain, pan, cutoff)
}

/// Equal-power gains of a pan in −1..1, the middle at 1 on both sides.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let th = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (th.cos() * std::f32::consts::SQRT_2, th.sin() * std::f32::consts::SQRT_2)
}

struct Voice {
    play: Play,
    /// Position in the sound, frames (fractional: pitch and rate).
    pos: f64,
    step: f64,
    /// The gains the last block ended at (each block ramps from them to its own).
    cur: (f32, f32),
    /// Fade: its level now, and how much it changes a frame (negative: stopping).
    fade: f32,
    fade_step: f32,
    /// Low pass state per channel, and its coefficient now.
    lp: (f32, f32),
    lp_a: f32,
    /// What it is heard at now (for choosing what goes when there are too many).
    level: f32,
    age: u64,
    started: bool,
}

pub struct Mixer {
    rate: u32,
    rx: Receiver<Cmd>,
    voices: Vec<Voice>,
    listener: Listener,
    volume: f32,
    paused: bool,
    ages: u64,
    /// Plays that came in while paused wait, at most a few (a paused game starts nothing of note).
    held: VecDeque<Play>,
}

impl Mixer {
    pub fn new(rate: u32, rx: Receiver<Cmd>) -> Mixer {
        Mixer { rate: rate.max(8000), rx, voices: Vec::with_capacity(MAX_VOICES + 1), listener: Listener::default(), volume: 1.0, paused: false, ages: 0, held: VecDeque::new() }
    }

    pub fn voices(&self) -> usize {
        self.voices.len()
    }

    /// The device's rate (it may change when the device is opened again): the voices keep their pitch.
    pub fn set_rate(&mut self, rate: u32) {
        self.rate = rate.max(8000);
        for v in &mut self.voices {
            v.step = v.play.sound.rate as f64 / self.rate as f64 * v.play.pitch.clamp(0.25, 4.0) as f64;
        }
    }

    fn command(&mut self, c: Cmd) {
        match c {
            Cmd::Play(p) => {
                if self.paused {
                    if self.held.len() < 8 && p.looped {
                        self.held.push_back(p);
                    }
                } else {
                    self.start(p);
                }
            }
            Cmd::Stop(key, secs) => {
                let step = 1.0 / (secs.max(0.0) * self.rate as f32).max(1.0);
                for v in &mut self.voices {
                    if key.is_none() || v.play.key == key {
                        v.fade_step = -step;
                    }
                }
                self.held.retain(|p| key.is_some() && p.key != key);
            }
            Cmd::Move(key, at) => {
                for v in &mut self.voices {
                    if v.play.key.as_deref() == Some(key.as_str()) {
                        v.play.at = Some(at);
                    }
                }
            }
            Cmd::Listener(l) => self.listener = l,
            Cmd::Volume(v) => self.volume = v.clamp(0.0, 1.0),
            Cmd::Paused(p) => {
                self.paused = p;
                if !p {
                    while let Some(h) = self.held.pop_front() {
                        self.start(h);
                    }
                }
            }
        }
    }

    fn start(&mut self, p: Play) {
        if p.sound.frames() == 0 {
            return;
        }
        // A ninth copy of one sound: its oldest goes. Too many in all: the quietest one-shot.
        let copies: Vec<usize> = (0..self.voices.len()).filter(|&i| Arc::ptr_eq(&self.voices[i].play.sound, &p.sound) && self.voices[i].fade_step >= 0.0).collect();
        if copies.len() >= MAX_COPIES {
            let oldest = copies.into_iter().min_by_key(|&i| self.voices[i].age).unwrap();
            self.voices.swap_remove(oldest);
        }
        if self.voices.len() >= MAX_VOICES {
            let quiet = (0..self.voices.len()).filter(|&i| !self.voices[i].play.looped).min_by(|&a, &b| self.voices[a].level.total_cmp(&self.voices[b].level));
            match quiet {
                Some(i) => {
                    self.voices.swap_remove(i);
                }
                None => return,
            }
        }
        let step = p.sound.rate as f64 / self.rate as f64 * p.pitch.clamp(0.25, 4.0) as f64;
        self.ages += 1;
        self.voices.push(Voice { play: p, pos: 0.0, step, cur: (0.0, 0.0), fade: 1.0, fade_step: 0.0, lp: (0.0, 0.0), lp_a: 1.0, level: 1.0, age: self.ages, started: false });
    }

    /// Fills `out` (interleaved, `channels` a frame): the first two channels are left and right, a mono device gets
    /// their mean, any others silence.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        while let Ok(c) = self.rx.try_recv() {
            self.command(c);
        }
        out.fill(0.0);
        let channels = channels.max(1);
        let frames = out.len() / channels;
        if self.paused || frames == 0 {
            return;
        }
        let rate = self.rate as f32;
        let l = self.listener;
        let n = frames as f32;
        for v in &mut self.voices {
            // Where it is heard this block.
            let (g, pan, cutoff) = match v.play.at {
                Some(at) => place(&l, at, v.play.falloff),
                None => (1.0, 0.0, 18_000.0),
            };
            let (pl, pr) = pan_gains(pan);
            let gain = v.play.gain.clamp(0.0, 4.0) * g;
            let target = (gain * pl, gain * pr);
            let a = 1.0 - (-TAU * cutoff.min(rate * 0.45) / rate).exp();
            if !v.started {
                // (The first block starts where it is: no ramp up from silence, no low pass's settling.)
                v.cur = target;
                v.lp_a = a;
                v.started = true;
            }
            v.level = gain * v.fade;
            let (dl, dr) = ((target.0 - v.cur.0) / n, (target.1 - v.cur.1) / n);
            let da = (a - v.lp_a) / n;
            let s = &v.play.sound;
            let sc = s.channels.max(1);
            let len = s.frames();
            let mut done = false;
            for f in 0..frames {
                let i = v.pos as usize;
                if i >= len {
                    if v.play.looped && len > 0 {
                        v.pos -= len as f64;
                        continue_loop(&mut v.pos, len);
                    } else {
                        done = true;
                        break;
                    }
                }
                let i = v.pos as usize;
                let t = (v.pos - i as f64) as f32;
                let j = if i + 1 < len { i + 1 } else if v.play.looped { 0 } else { i };
                let (xl, xr) = if sc == 1 {
                    let x = s.data[i] + (s.data[j] - s.data[i]) * t;
                    (x, x)
                } else {
                    let (a0, b0) = (s.data[i * sc], s.data[i * sc + 1]);
                    let (a1, b1) = (s.data[j * sc], s.data[j * sc + 1]);
                    (a0 + (a1 - a0) * t, b0 + (b1 - b0) * t)
                };
                v.lp.0 += v.lp_a * (xl - v.lp.0);
                v.lp.1 += v.lp_a * (xr - v.lp.1);
                v.lp_a += da;
                v.cur.0 += dl;
                v.cur.1 += dr;
                if v.fade_step != 0.0 {
                    v.fade = (v.fade + v.fade_step).clamp(0.0, 1.0);
                    if v.fade <= 0.0 {
                        done = true;
                        break;
                    }
                }
                let k = f * channels;
                let (ol, or) = (v.lp.0 * v.cur.0 * v.fade, v.lp.1 * v.cur.1 * v.fade);
                if channels == 1 {
                    out[k] += 0.5 * (ol + or);
                } else {
                    out[k] += ol;
                    out[k + 1] += or;
                }
                v.pos += v.step;
            }
            v.cur = target;
            v.lp_a = a;
            if done {
                v.fade = 0.0;
                v.fade_step = -1.0;
            }
        }
        self.voices.retain(|v| v.fade > 0.0);
        // The app's volume, then the limiter: transparent under −2 dB (0.8), a soft knee above it, never past 1.
        let vol = self.volume;
        for x in out.iter_mut() {
            let y = *x * vol;
            let a = y.abs();
            *x = if a <= 0.8 { y } else { y.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh()) };
        }
    }
}

fn continue_loop(pos: &mut f64, len: usize) {
    while *pos >= len as f64 {
        *pos -= len as f64;
    }
    if *pos < 0.0 {
        *pos = 0.0;
    }
}

/// How long a stop fades when the game does not say.
pub fn stop_fade(secs: Option<f64>) -> f32 {
    secs.map_or(DEFAULT_STOP_FADE, |s| s.clamp(0.0, 10.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn tone(frames: usize, rate: u32) -> Arc<Sound> {
        Arc::new(Sound { name: "t".into(), rate, channels: 1, data: (0..frames).map(|i| (i as f32 * 0.05).sin() * 0.5).collect() })
    }

    fn play(s: &Arc<Sound>, at: Option<Vec3>) -> Play {
        Play { sound: s.clone(), at, gain: 1.0, pitch: 1.0, looped: false, key: None, falloff: DEFAULT_FALLOFF }
    }

    fn energy(b: &[f32], ch: usize, c: usize) -> f32 {
        b.iter().skip(c).step_by(ch).map(|x| x * x).sum()
    }

    #[test]
    fn distance_falls_as_falloff_over_distance_and_fades_out() {
        let l = Listener::default();
        assert_eq!(place(&l, Vec3::new(0.0, 0.0, -1.5), 2.0).0, 1.0);
        assert!((place(&l, Vec3::new(0.0, 0.0, -20.0), 2.0).0 - 0.1).abs() < 1e-6);
        assert!((place(&l, Vec3::new(0.0, 0.0, -110.0), 2.0).0 - 2.0 / 110.0 * 0.5).abs() < 1e-6);
        assert_eq!(place(&l, Vec3::new(0.0, 0.0, -130.0), 2.0).0, 0.0);
    }

    #[test]
    fn a_sound_to_the_right_is_louder_on_the_right_and_one_in_front_is_in_the_middle() {
        let l = Listener::default();
        let (_, pan, _) = place(&l, Vec3::new(10.0, 0.0, 0.0), 2.0);
        assert!((pan - MAX_PAN).abs() < 1e-6);
        let (gl, gr) = pan_gains(pan);
        assert!(gr > 3.0 * gl, "{gl} {gr}");
        let (_, front, _) = place(&l, Vec3::new(0.0, 0.0, -10.0), 2.0);
        assert!(front.abs() < 1e-6);
        let (a, b) = pan_gains(0.0);
        assert!((a - 1.0).abs() < 1e-6 && (b - 1.0).abs() < 1e-6);
        // Behind is duller than in front at the same distance.
        assert!(place(&l, Vec3::new(0.0, 0.0, 10.0), 2.0).2 < place(&l, Vec3::new(0.0, 0.0, -10.0), 2.0).2);
    }

    #[test]
    fn mixes_places_and_ends_a_one_shot() {
        let (tx, rx) = channel();
        let mut m = Mixer::new(48_000, rx);
        let s = tone(4800, 48_000);
        tx.send(Cmd::Play(play(&s, Some(Vec3::new(5.0, 0.0, 0.0))))).unwrap();
        let mut b = vec![0.0; 1024 * 2];
        m.render(&mut b, 2);
        assert!(energy(&b, 2, 1) > 4.0 * energy(&b, 2, 0));
        for _ in 0..10 {
            m.render(&mut b, 2);
        }
        assert_eq!(m.voices(), 0);
        assert!(b.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn pitch_two_ends_in_half_the_time_and_a_loop_goes_on_until_stopped() {
        let (tx, rx) = channel();
        let mut m = Mixer::new(48_000, rx);
        let s = tone(4800, 48_000);
        tx.send(Cmd::Play(Play { pitch: 2.0, ..play(&s, None) })).unwrap();
        let mut b = vec![0.0; 2400 * 2];
        m.render(&mut b, 2);
        assert!(m.voices() == 0 || { m.render(&mut b, 2); m.voices() == 0 });
        tx.send(Cmd::Play(Play { looped: true, key: Some("amb".into()), ..play(&s, None) })).unwrap();
        for _ in 0..20 {
            m.render(&mut b, 2);
        }
        assert_eq!(m.voices(), 1);
        tx.send(Cmd::Stop(Some("amb".into()), 0.01)).unwrap();
        m.render(&mut b, 2);
        assert_eq!(m.voices(), 0);
    }

    #[test]
    fn copies_and_voices_are_bounded_and_the_limiter_holds() {
        let (tx, rx) = channel();
        let mut m = Mixer::new(48_000, rx);
        let s = tone(48_000, 48_000);
        for _ in 0..20 {
            tx.send(Cmd::Play(Play { gain: 4.0, ..play(&s, None) })).unwrap();
        }
        let mut b = vec![0.0; 256 * 2];
        m.render(&mut b, 2);
        assert_eq!(m.voices(), MAX_COPIES);
        assert!(b.iter().all(|x| x.abs() <= 1.0));
        for i in 0..60 {
            tx.send(Cmd::Play(play(&tone(48_000 + i, 48_000), None))).unwrap();
        }
        m.render(&mut b, 2);
        assert_eq!(m.voices(), MAX_VOICES);
    }

    #[test]
    fn paused_is_silent_and_volume_scales() {
        let (tx, rx) = channel();
        let mut m = Mixer::new(48_000, rx);
        let s = tone(48_000, 48_000);
        tx.send(Cmd::Play(Play { looped: true, key: Some("k".into()), ..play(&s, None) })).unwrap();
        let mut b = vec![0.0; 512];
        m.render(&mut b, 1);
        let full = energy(&b, 1, 0);
        tx.send(Cmd::Volume(0.5)).unwrap();
        m.render(&mut b, 1);
        assert!(energy(&b, 1, 0) < full * 0.5);
        tx.send(Cmd::Paused(true)).unwrap();
        m.render(&mut b, 1);
        assert!(b.iter().all(|x| *x == 0.0));
    }
}
