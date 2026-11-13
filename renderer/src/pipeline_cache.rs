//! Compiled pipelines kept on disk between runs (`gpu::GpuOptions::pipeline_cache`; THREERT_PIPELINE_CACHE=<directory>).
//! A phone's driver compiles a kernel for seconds (Adreno 740: ten), and every variant a game uses again each time it
//! is opened; with the cache the second opening reads what the first compiled (docs/RUNTIME.md "Android"). Off unless
//! a host names a directory; only where the backend has such a cache (Vulkan: wgpu's `PIPELINE_CACHE` feature).
//!
//! The file, one per adapter:
//!  - **Its name** is wgpu's key for the adapter (`wgpu::util::pipeline_cache_key`: backend, vendor, device), so two
//!    GPUs never read each other's. Inside, wgpu's own header holds the driver's cache identity: data from another
//!    driver version is not used (`fallback`: an empty cache then, and the next save replaces the file).
//!  - **Its bytes** are what `wgpu::PipelineCache::get_data` returned, sealed: a magic, the length and the data's
//!    SHA-256 before them. A file that is cut short, damaged or not ours is not handed to the driver (which need not
//!    check what it is given); it counts as no file.
//!  - **Two processes may share it** (two games of one app, a game opened twice). A save writes the whole file under
//!    a name of its own (process id, a count) beside it and renames it over the old one: a reader finds the old file
//!    or the new one, whole, and a process stopped halfway leaves the old one. The last save wins; what the other
//!    process compiled meanwhile is compiled once more, by whoever misses it, and saved then.
//!  - **It may be deleted** at any time, and is not read past `MAX_BYTES` (a cache nobody cleared starts again).

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const MAGIC: &[u8; 8] = b"3RTPIPE1";
const HEADER: usize = 8 + 8 + 32;
/// The largest file read: past it the cache starts empty (every kernel variant of every game ever opened adds to it).
pub const MAX_BYTES: u64 = 256 << 20;
/// A save's own file that old was left by a process that stopped: removed when the cache is opened.
const STALE: std::time::Duration = std::time::Duration::from_secs(600);

/// The file's bytes for a cache's data.
pub fn seal(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + data.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(&Sha256::digest(data));
    out.extend_from_slice(data);
    out
}

/// The cache's data in a file's bytes, if they are what `seal` wrote.
pub fn unseal(file: &[u8]) -> Option<&[u8]> {
    let (head, data) = file.split_at_checked(HEADER)?;
    let len = u64::from_le_bytes(head[8..16].try_into().ok()?);
    (&head[..8] == MAGIC && len == data.len() as u64 && head[16..] == Sha256::digest(data)[..]).then_some(data)
}

/// The cache file's name for an adapter (None: its backend has no pipeline cache).
pub fn file_name(info: &wgpu::AdapterInfo) -> Option<String> {
    wgpu::util::pipeline_cache_key(info).map(|key| format!("{key}.bin"))
}

/// `bytes` as the whole of `path`: written beside it under a name no other save has, then renamed over it.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static SAVES: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.{}.tmp", std::process::id(), SAVES.fetch_add(1, Ordering::Relaxed)));
    let tmp = path.with_file_name(name);
    let done = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done
}

/// Removes the saves' own files beside `path` that a stopped process left (`STALE`).
fn sweep(path: &Path) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let ours = e.file_name().to_str().is_some_and(|n| n.starts_with(name) && n.ends_with(".tmp"));
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > STALE);
        if ours && old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// The sealed data of the file at `path`, if there is one to use.
fn load(path: &Path) -> Option<Vec<u8>> {
    if std::fs::metadata(path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let file = std::fs::read(path).ok()?;
    unseal(&file).map(|d| d.to_vec())
}

/// A device's pipeline cache and its file.
pub struct PipelineCache {
    cache: wgpu::PipelineCache,
    path: PathBuf,
    /// A pipeline was made with the cache since the last save.
    dirty: AtomicBool,
    /// The SHA-256 of the data last read or written: a save of the same data writes nothing.
    saved: std::sync::Mutex<[u8; 32]>,
}

impl PipelineCache {
    /// The feature the device must be made with for `open`, if the adapter has a cache to keep in `dir`.
    pub fn feature(adapter: &wgpu::Adapter, info: &wgpu::AdapterInfo) -> Option<wgpu::Features> {
        (adapter.features().contains(wgpu::Features::PIPELINE_CACHE) && file_name(info).is_some()).then_some(wgpu::Features::PIPELINE_CACHE)
    }

    /// The cache of `device` (made with `feature`), filled from its file in `dir` when that holds this adapter's data.
    pub fn open(device: &wgpu::Device, dir: &Path, info: &wgpu::AdapterInfo) -> Option<PipelineCache> {
        let path = dir.join(file_name(info)?);
        sweep(&path);
        let data = load(&path);
        // Safety: `data` is what `get_data` returned for an adapter of this key (the file's name), whole (`unseal`);
        // wgpu checks its own header against this adapter and driver, and with `fallback` starts empty otherwise.
        let cache = unsafe { device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor { label: Some("threert"), data: data.as_deref(), fallback: true }) };
        let saved = data.map_or([0; 32], |d| Sha256::digest(&d).into());
        Some(PipelineCache { cache, path, dirty: AtomicBool::new(false), saved: std::sync::Mutex::new(saved) })
    }

    /// The cache, for the descriptor of a pipeline about to be made (the next `save` writes it).
    pub fn for_pipeline(&self) -> &wgpu::PipelineCache {
        self.dirty.store(true, Ordering::Relaxed);
        &self.cache
    }

    /// The cache to its file, if a pipeline was made since the last save and the data changed (a pipeline the cache
    /// already had changes nothing). A failure to write is not an error: the next run compiles again.
    pub fn save(&self) {
        if !self.dirty.swap(false, Ordering::Relaxed) {
            return;
        }
        let Some(data) = self.cache.get_data() else { return };
        let hash: [u8; 32] = Sha256::digest(&data).into();
        let mut saved = self.saved.lock().unwrap();
        if *saved != hash && write_whole(&self.path, &seal(&data)).is_ok() {
            *saved = hash;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("threert-pipeline-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A file is used only if it is whole and ours: cut short, a byte changed, another length, another magic, or the
    /// bare data without the seal all count as no file.
    #[test]
    fn only_a_whole_file_is_used() {
        let data: Vec<u8> = (0..1000u32).flat_map(|k| k.to_le_bytes()).collect();
        let file = seal(&data);
        assert_eq!(unseal(&file), Some(&data[..]));
        assert_eq!(unseal(&seal(&[])), Some(&[][..]));
        assert_eq!(unseal(&file[..file.len() - 1]), None);
        assert_eq!(unseal(&file[..HEADER - 1]), None);
        assert_eq!(unseal(&[]), None);
        assert_eq!(unseal(&data), None);
        let mut flipped = file.clone();
        *flipped.last_mut().unwrap() ^= 1;
        assert_eq!(unseal(&flipped), None);
        let mut longer = file.clone();
        longer.push(0);
        assert_eq!(unseal(&longer), None);
        let mut other = file;
        other[0] = b'4';
        assert_eq!(unseal(&other), None);
    }

    /// The file is named for the adapter (wgpu's key: backend, vendor, device), so another GPU's is another file; a
    /// backend without a pipeline cache has none.
    #[test]
    fn the_file_is_named_for_the_adapter() {
        let mut adreno = wgpu::AdapterInfo::new(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Vulkan);
        (adreno.vendor, adreno.device) = (0x5143, 0x43050a01);
        let name = file_name(&adreno).unwrap();
        assert_eq!(name, format!("{}.bin", wgpu::util::pipeline_cache_key(&adreno).unwrap()));
        assert!(name.contains(&0x5143.to_string()) && name.contains(&0x43050a01u32.to_string()), "{name}");
        let mut other = adreno.clone();
        other.device += 1;
        assert_ne!(file_name(&other), Some(name));
        assert_eq!(file_name(&wgpu::AdapterInfo::new(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Metal)), None);
    }

    /// Saves from several threads at once (as two processes would: each under a name of its own, renamed over the
    /// file) leave one whole file that is one of theirs, and no file of a save behind; a reader meanwhile never sees
    /// a file that is not whole.
    #[test]
    fn concurrent_saves_leave_a_whole_file() {
        let dir = scratch("saves");
        let path = dir.join("cache.bin");
        let blobs: Vec<Vec<u8>> = (0..8u8).map(|k| vec![k; 200_000 + k as usize]).collect();
        write_whole(&path, &seal(&blobs[0])).unwrap();
        std::thread::scope(|s| {
            for b in &blobs {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..20 {
                        write_whole(path, &seal(b)).unwrap();
                    }
                });
            }
            for _ in 0..200 {
                let file = std::fs::read(&path).unwrap();
                assert!(unseal(&file).is_some(), "a reader saw a file that is not whole ({} bytes)", file.len());
            }
        });
        let last = load(&path).expect("a whole file");
        assert!(blobs.contains(&last));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "the saves' own files are gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a stopped process left is swept when it is old, a file that is not whole is not loaded, and neither is one
    /// past the size limit's check (a directory in its place: no file).
    #[test]
    fn a_damaged_file_and_a_stopped_save_are_ignored() {
        let dir = scratch("stale");
        let path = dir.join("cache.bin");
        write_whole(&path, &seal(b"pipelines")).unwrap();
        assert_eq!(load(&path).as_deref(), Some(&b"pipelines"[..]));
        // A save in progress (young) stays; one from long ago goes.
        let (young, old) = (dir.join("cache.bin.1.0.tmp"), dir.join("cache.bin.2.0.tmp"));
        std::fs::write(&young, b"half").unwrap();
        std::fs::write(&old, b"half").unwrap();
        let long_ago = std::time::SystemTime::now() - 2 * STALE;
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(long_ago).unwrap();
        sweep(&path);
        assert!(young.exists() && !old.exists() && path.exists());
        // Cut short on disk (a copy that failed, a full disk): no file.
        let whole = std::fs::read(&path).unwrap();
        std::fs::write(&path, &whole[..whole.len() - 3]).unwrap();
        assert_eq!(load(&path), None);
        assert_eq!(load(&dir.join("missing.bin")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
