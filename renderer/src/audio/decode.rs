//! A sound file to frames: WAV (PCM, float), Ogg Vorbis, MP3 and FLAC, by Symphonia (pure Rust: the same decoder on
//! every platform, so a sound is the same samples everywhere). One or two channels are kept (more: the first two);
//! at most ten minutes.

use super::mix::Sound;
use anyhow::{anyhow, bail, Context, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymErr;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

const MAX_SECONDS: usize = 600;

/// The file extensions that are sounds.
pub const EXTENSIONS: [&str; 6] = ["wav", "ogg", "oga", "mp3", "flac", "wave"];

pub fn decode(name: &str, bytes: Vec<u8>) -> Result<Sound> {
    let mut hint = Hint::new();
    if let Some(ext) = name.rsplit('.').next() {
        hint.with_extension(&ext.to_ascii_lowercase());
    }
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).with_context(|| format!("{name}: not a sound this engine reads (WAV, Ogg Vorbis, MP3, FLAC)"))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or_else(|| anyhow!("{name}: no audio track"))?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.ok_or_else(|| anyhow!("{name}: no sample rate"))?;
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).with_context(|| format!("{name}: its codec"))?;
    let mut channels = 0usize;
    let mut data: Vec<f32> = Vec::new();
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymErr::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymErr::ResetRequired) => break,
            Err(e) => return Err(anyhow!("{name}: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // (A damaged packet is skipped, as players do.)
            Err(SymErr::DecodeError(_)) => continue,
            Err(e) => return Err(anyhow!("{name}: {e}")),
        };
        let spec = *decoded.spec();
        let n = spec.channels.count();
        if channels == 0 {
            channels = n.min(2);
        }
        let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if b.capacity() < decoded.capacity() * n {
            *b = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        b.copy_interleaved_ref(decoded);
        for frame in b.samples().chunks(n) {
            data.extend_from_slice(&frame[..channels]);
        }
        if data.len() / channels.max(1) > MAX_SECONDS * rate as usize {
            bail!("{name}: longer than {} minutes", MAX_SECONDS / 60);
        }
    }
    if channels == 0 || data.is_empty() {
        bail!("{name}: no samples");
    }
    Ok(Sound { name: name.to_string(), rate, channels, data })
}

/// A 16-bit PCM WAV of `samples` (mono), for tests.
#[cfg(test)]
pub(crate) fn tests_wav(rate: u32, samples: &[i16]) -> Vec<u8> {
        let mut v = Vec::new();
        let data = (samples.len() * 2) as u32;
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * 2).to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data.to_le_bytes());
        for s in samples {
            v.extend_from_slice(&s.to_le_bytes());
        }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_wav() {
        let s = decode("a.wav", tests_wav(22_050, &[0, 16384, -16384, 32767])).unwrap();
        assert_eq!((s.rate, s.channels, s.frames()), (22_050, 1, 4));
        assert!((s.data[1] - 0.5).abs() < 1e-3 && (s.data[2] + 0.5).abs() < 1e-3);
    }

    #[test]
    fn says_what_is_not_a_sound() {
        let e = decode("a.ogg", b"not a sound".to_vec()).unwrap_err();
        assert!(format!("{e:#}").contains("a.ogg"), "{e:#}");
    }
}
