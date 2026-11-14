//! The sound device: a thread of its own opens the system's default output and lets the mixer fill it. Only this part
//! differs between platforms, and it does nothing but move the mixer's frames:
//!
//! - macOS, iOS, Windows: cpal (Core Audio, WASAPI);
//! - Linux (the Steam Deck among them): ALSA's `default` device, the library loaded at run time (libasound.so.2: on
//!   SteamOS and every desktop it is PipeWire's or PulseAudio's), so a program built for Linux links no sound library
//!   and runs, silent, where there is none;
//! - Android: AAudio (libaaudio.so, Android 8 and later), loaded at run time too.
//!
//! The stream is opened again when the device goes (headphones pulled, the default output changed), with the same
//! mixer. Elsewhere, or when no device opens, `open` says why and the app plays nothing.

use super::mix::Mixer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct Output {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// The device thread's report: once, when the stream runs (frames a second, channels, the device's name) or did
    /// not open.
    pub ready: Receiver<Result<(u32, usize, String), String>>,
}

impl Drop for Output {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

/// What a backend reports once its stream runs.
type Ready = Sender<Result<(u32, usize, String), String>>;

/// Opens the default output for `mixer` (its rate is set to the device's) on a thread of its own, without waiting
/// for it: `ready` says how it went. Dropping the output closes the device; the mixer and its voices stay.
pub fn open(mixer: Arc<Mutex<Mixer>>) -> std::io::Result<Output> {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, ready) = channel();
    let st = stop.clone();
    let thread = std::thread::Builder::new().name("threert-audio".into()).spawn(move || run(mixer, st, tx))?;
    Ok(Output { stop, thread: Some(thread), ready })
}

/// The device thread: opens, runs until stopped, opens again after the device went (at most once a second).
fn run(mixer: Arc<Mutex<Mixer>>, stop: Arc<AtomicBool>, ready: Ready) {
    let mut ready = Some(ready);
    while !stop.load(Ordering::SeqCst) {
        let lost = Arc::new(AtomicBool::new(false));
        if let Err(e) = backend::stream(&mixer, &stop, &lost, &mut ready) {
            if let Some(r) = ready.take() {
                let _ = r.send(Err(e));
                return;
            }
        }
        if !stop.load(Ordering::SeqCst) {
            std::thread::park_timeout(Duration::from_secs(1));
        }
    }
}

/// Waits until the app stops the output or the stream reports the device gone.
#[allow(dead_code)]
fn wait(stop: &AtomicBool, lost: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) && !lost.load(Ordering::SeqCst) {
        std::thread::park_timeout(Duration::from_millis(250));
    }
}

/// The mixer's frames as f32, `channels` a frame (a buffer reused from call to call).
#[allow(dead_code)]
fn fill(mixer: &Mutex<Mixer>, out: &mut [f32], channels: usize) {
    match mixer.lock() {
        Ok(mut m) => m.render(out, channels),
        Err(_) => out.fill(0.0),
    }
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "windows"))]
mod backend {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SizedSample};

    pub fn stream(mixer: &Arc<Mutex<Mixer>>, stop: &AtomicBool, lost: &Arc<AtomicBool>, ready: &mut Option<Ready>) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no sound output device")?;
        let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "the default output".into());
        let config = device.default_output_config().map_err(|e| format!("{name}: {e}"))?;
        let (rate, channels) = (config.sample_rate(), config.channels() as usize);
        mixer.lock().map_err(|_| "mixer")?.set_rate(rate);
        let sc: cpal::StreamConfig = config.config();
        let s = match config.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &sc, mixer, lost),
            cpal::SampleFormat::I16 => build::<i16>(&device, &sc, mixer, lost),
            cpal::SampleFormat::I32 => build::<i32>(&device, &sc, mixer, lost),
            cpal::SampleFormat::U16 => build::<u16>(&device, &sc, mixer, lost),
            f => return Err(format!("{name}: sample format {f} not handled")),
        }
        .map_err(|e| format!("{name}: {e}"))?;
        s.play().map_err(|e| format!("{name}: {e}"))?;
        if let Some(r) = ready.take() {
            let _ = r.send(Ok((rate, channels, name)));
        }
        wait(stop, lost);
        Ok(())
    }

    fn build<T: SizedSample + FromSample<f32>>(device: &cpal::Device, sc: &cpal::StreamConfig, mixer: &Arc<Mutex<Mixer>>, lost: &Arc<AtomicBool>) -> Result<cpal::Stream, cpal::BuildStreamError> {
        let ch = sc.channels as usize;
        let m = mixer.clone();
        let l = lost.clone();
        let mut buf: Vec<f32> = Vec::new();
        device.build_output_stream(
            sc,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                buf.resize(data.len(), 0.0);
                fill(&m, &mut buf, ch);
                for (d, s) in data.iter_mut().zip(&buf) {
                    *d = T::from_sample(*s);
                }
            },
            move |e| {
                // (The device went, or another error: the stream is made again.)
                if matches!(e, cpal::StreamError::DeviceNotAvailable) || !matches!(e, cpal::StreamError::BufferUnderrun) {
                    l.store(true, Ordering::SeqCst);
                }
            },
            None,
        )
    }
}

#[cfg(all(target_os = "linux", not(target_os = "android")))]
mod backend {
    use super::*;
    use libloading::{Library, Symbol};
    use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CStr};

    const RATE: u32 = 48_000;
    const CHANNELS: usize = 2;
    /// Frames a write: 10 ms.
    const PERIOD: usize = 480;
    const SND_PCM_STREAM_PLAYBACK: c_int = 0;
    const SND_PCM_FORMAT_S16_LE: c_int = 2;
    const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;

    type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
    type SetParams = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
    type WriteI = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
    type Recover = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;
    type Close = unsafe extern "C" fn(*mut c_void) -> c_int;
    type StrError = unsafe extern "C" fn(c_int) -> *const c_char;

    pub fn stream(mixer: &Arc<Mutex<Mixer>>, stop: &AtomicBool, _lost: &Arc<AtomicBool>, ready: &mut Option<Ready>) -> Result<(), String> {
        // SAFETY: libasound's documented C API, called as its headers declare it; the handle is used on this thread
        // only and closed before the library goes.
        unsafe {
            let lib = Library::new("libasound.so.2").map_err(|e| format!("no ALSA library (libasound.so.2): {e}"))?;
            let sym = |n: &[u8]| -> Result<*const (), String> { lib.get::<*const ()>(n).map(|s: Symbol<*const ()>| *s).map_err(|e| format!("ALSA: {e}")) };
            let open: Open = std::mem::transmute(sym(b"snd_pcm_open\0")?);
            let set_params: SetParams = std::mem::transmute(sym(b"snd_pcm_set_params\0")?);
            let writei: WriteI = std::mem::transmute(sym(b"snd_pcm_writei\0")?);
            let recover: Recover = std::mem::transmute(sym(b"snd_pcm_recover\0")?);
            let close: Close = std::mem::transmute(sym(b"snd_pcm_close\0")?);
            let strerror: StrError = std::mem::transmute(sym(b"snd_strerror\0")?);
            let err = |c: c_int| CStr::from_ptr(strerror(c)).to_string_lossy().into_owned();
            let mut pcm: *mut c_void = std::ptr::null_mut();
            let r = open(&mut pcm, c"default".as_ptr(), SND_PCM_STREAM_PLAYBACK, 0);
            if r < 0 {
                return Err(format!("ALSA default device: {}", err(r)));
            }
            // 48 kHz stereo 16-bit, resampled by ALSA when the device is otherwise, 40 ms of latency.
            let r = set_params(pcm, SND_PCM_FORMAT_S16_LE, SND_PCM_ACCESS_RW_INTERLEAVED, CHANNELS as c_uint, RATE, 1, 40_000);
            if r < 0 {
                close(pcm);
                return Err(format!("ALSA default device: {}", err(r)));
            }
            mixer.lock().map_err(|_| "mixer")?.set_rate(RATE);
            if let Some(r) = ready.take() {
                let _ = r.send(Ok((RATE, CHANNELS, "ALSA default".into())));
            }
            let mut f = vec![0.0f32; PERIOD * CHANNELS];
            let mut s = vec![0i16; PERIOD * CHANNELS];
            let mut failures = 0;
            while !stop.load(Ordering::SeqCst) {
                fill(mixer, &mut f, CHANNELS);
                for (d, x) in s.iter_mut().zip(&f) {
                    *d = (x.clamp(-1.0, 1.0) * 32767.0) as i16;
                }
                let mut off = 0usize;
                while off < PERIOD && !stop.load(Ordering::SeqCst) {
                    let n = writei(pcm, s[off * CHANNELS..].as_ptr() as *const c_void, (PERIOD - off) as c_ulong);
                    if n < 0 {
                        // An underrun or a suspend is recovered; anything else is the device gone: open it again.
                        if recover(pcm, n as c_int, 1) < 0 {
                            failures += 1;
                            break;
                        }
                    } else {
                        off += n as usize;
                    }
                }
                if failures > 0 {
                    break;
                }
            }
            close(pcm);
            Ok(())
        }
    }
}

#[cfg(target_os = "android")]
mod backend {
    use super::*;
    use libloading::Library;
    use std::ffi::c_void;

    const AAUDIO_FORMAT_PCM_FLOAT: i32 = 2;
    const AAUDIO_PERFORMANCE_MODE_LOW_LATENCY: i32 = 12;
    const AAUDIO_USAGE_GAME: i32 = 14;
    const AAUDIO_CALLBACK_RESULT_CONTINUE: i32 = 0;

    type Builder = *mut c_void;
    type Stream = *mut c_void;
    type DataCb = extern "C" fn(Stream, *mut c_void, *mut c_void, i32) -> i32;
    type ErrorCb = extern "C" fn(Stream, *mut c_void, i32);

    /// What the callbacks get: the mixer, the channels, and the flag that says the device went.
    struct User {
        mixer: Arc<Mutex<Mixer>>,
        channels: usize,
        lost: Arc<AtomicBool>,
    }

    extern "C" fn data(_s: Stream, user: *mut c_void, audio: *mut c_void, frames: i32) -> i32 {
        // SAFETY: `user` is the `User` this stream was opened with, alive until the stream is closed; `audio` is
        // `frames` frames of f32 of `channels` each, AAudio's buffer for this call.
        let u = unsafe { &*(user as *const User) };
        let out = unsafe { std::slice::from_raw_parts_mut(audio as *mut f32, frames.max(0) as usize * u.channels) };
        fill(&u.mixer, out, u.channels);
        AAUDIO_CALLBACK_RESULT_CONTINUE
    }

    extern "C" fn error(_s: Stream, user: *mut c_void, _e: i32) {
        // SAFETY: as in `data`.
        let u = unsafe { &*(user as *const User) };
        u.lost.store(true, Ordering::SeqCst);
    }

    pub fn stream(mixer: &Arc<Mutex<Mixer>>, stop: &AtomicBool, lost: &Arc<AtomicBool>, ready: &mut Option<Ready>) -> Result<(), String> {
        // SAFETY: AAudio's documented C API (aaudio/AAudio.h, API 26), called as declared; the stream is closed (no
        // callback runs after `AAudioStream_close` returns) before `user` and the library go.
        unsafe {
            let lib = Library::new("libaaudio.so").map_err(|e| format!("no AAudio (libaaudio.so): {e}"))?;
            macro_rules! f {
                ($n:literal, $t:ty) => {
                    *lib.get::<$t>(concat!($n, "\0").as_bytes()).map_err(|e| format!("AAudio: {e}"))?
                };
            }
            let create = f!("AAudio_createStreamBuilder", unsafe extern "C" fn(*mut Builder) -> i32);
            let set_format = f!("AAudioStreamBuilder_setFormat", unsafe extern "C" fn(Builder, i32));
            let set_channels = f!("AAudioStreamBuilder_setChannelCount", unsafe extern "C" fn(Builder, i32));
            let set_perf = f!("AAudioStreamBuilder_setPerformanceMode", unsafe extern "C" fn(Builder, i32));
            let set_data = f!("AAudioStreamBuilder_setDataCallback", unsafe extern "C" fn(Builder, DataCb, *mut c_void));
            let set_error = f!("AAudioStreamBuilder_setErrorCallback", unsafe extern "C" fn(Builder, ErrorCb, *mut c_void));
            let open = f!("AAudioStreamBuilder_openStream", unsafe extern "C" fn(Builder, *mut Stream) -> i32);
            let delete = f!("AAudioStreamBuilder_delete", unsafe extern "C" fn(Builder) -> i32);
            let rate_of = f!("AAudioStream_getSampleRate", unsafe extern "C" fn(Stream) -> i32);
            let start = f!("AAudioStream_requestStart", unsafe extern "C" fn(Stream) -> i32);
            let close = f!("AAudioStream_close", unsafe extern "C" fn(Stream) -> i32);
            // (Android 9: usage "game"; on 8 the symbol is missing and the default, media, is kept.)
            let set_usage = lib.get::<unsafe extern "C" fn(Builder, i32)>(b"AAudioStreamBuilder_setUsage\0").ok().map(|s| *s);

            const CHANNELS: usize = 2;
            let user = Box::into_raw(Box::new(User { mixer: mixer.clone(), channels: CHANNELS, lost: lost.clone() }));
            let mut b: Builder = std::ptr::null_mut();
            let r = create(&mut b);
            if r != 0 {
                drop(Box::from_raw(user));
                return Err(format!("AAudio: builder ({r})"));
            }
            set_format(b, AAUDIO_FORMAT_PCM_FLOAT);
            set_channels(b, CHANNELS as i32);
            set_perf(b, AAUDIO_PERFORMANCE_MODE_LOW_LATENCY);
            if let Some(u) = set_usage {
                u(b, AAUDIO_USAGE_GAME);
            }
            set_data(b, data, user as *mut c_void);
            set_error(b, error, user as *mut c_void);
            let mut s: Stream = std::ptr::null_mut();
            let r = open(b, &mut s);
            delete(b);
            if r != 0 {
                drop(Box::from_raw(user));
                return Err(format!("AAudio: no output stream ({r})"));
            }
            let rate = rate_of(s).max(8000) as u32;
            if let Ok(mut m) = mixer.lock() {
                m.set_rate(rate);
            }
            let r = start(s);
            if r != 0 {
                close(s);
                drop(Box::from_raw(user));
                return Err(format!("AAudio: the stream did not start ({r})"));
            }
            if let Some(r) = ready.take() {
                let _ = r.send(Ok((rate, CHANNELS, "AAudio".into())));
            }
            wait(stop, lost);
            close(s);
            drop(Box::from_raw(user));
            Ok(())
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows", target_os = "linux", target_os = "android")))]
mod backend {
    use super::*;
    pub fn stream(_: &Arc<Mutex<Mixer>>, _: &AtomicBool, _: &Arc<AtomicBool>, _: &mut Option<Ready>) -> Result<(), String> {
        Err("no sound output on this platform".into())
    }
}
