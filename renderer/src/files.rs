//! Where the engine's library reads files from. Natively: the file system, as before (these are `std::fs`'s and the
//! `image` / `gltf` crates' own readers, called from one place). In a browser (wasm32) there is no file system: the
//! page fetches what a scene names (`World::files`) and hands the bytes over with `provide` before the scene is built,
//! and the two presets are compiled in. The paths are the ones the native build would open (asset root joined with
//! the scene's uri), so the same scene code names the same files on both.

use anyhow::Result;
use std::path::Path;

#[cfg(target_arch = "wasm32")]
thread_local! {
    static PROVIDED: std::cell::RefCell<std::collections::HashMap<std::path::PathBuf, std::sync::Arc<[u8]>>> = Default::default();
}

/// A file's bytes for the path the engine will ask for (a browser's page, after it fetched them).
#[cfg(target_arch = "wasm32")]
pub fn provide(path: &Path, bytes: Vec<u8>) {
    PROVIDED.with(|p| p.borrow_mut().insert(path.to_path_buf(), bytes.into()));
}

/// The file's bytes.
pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    return std::fs::read(path);
    #[cfg(target_arch = "wasm32")]
    return PROVIDED
        .with(|p| p.borrow().get(path).map(|b| b.to_vec()))
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("{}: not provided (a page fetches a scene's files before it is built)", path.display())));
}

/// The file's length and modification time (ns since the epoch), for the caches' keys; (0, 0) for one that is not
/// there. In a browser: its length (bytes provided anew under a path are another version only if their length differs;
/// a page that replaces a file reloads).
pub fn version(path: &Path) -> (u64, u128) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let m = std::fs::metadata(path).ok();
        let mtime = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
        (m.map_or(0, |m| m.len()), mtime)
    }
    #[cfg(target_arch = "wasm32")]
    {
        (PROVIDED.with(|p| p.borrow().get(path).map_or(0, |b| b.len() as u64)), 0)
    }
}

/// A texture file decoded to RGBA8.
pub fn image(path: &Path) -> Result<image::RgbaImage> {
    #[cfg(not(target_arch = "wasm32"))]
    return Ok(image::open(path)?.to_rgba8());
    #[cfg(target_arch = "wasm32")]
    return Ok(image::load_from_memory(&read(path)?)?.to_rgba8());
}

/// A glTF / GLB file: its document, and the binary chunk of a GLB. (The buffers and images it names beside it are
/// read by `gltf_base`.)
pub fn gltf(path: &Path) -> gltf::Result<gltf::Gltf> {
    #[cfg(not(target_arch = "wasm32"))]
    return gltf::Gltf::open(path);
    #[cfg(target_arch = "wasm32")]
    return gltf::Gltf::from_slice(&read(path).map_err(gltf::Error::Io)?);
}

/// A render preset's JSON: `<dir>/<name>.json`; in a browser, the two the engine ships, compiled in.
pub fn preset(dir: &Path, name: &str) -> std::io::Result<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    return std::fs::read(dir.join(format!("{name}.json")));
    #[cfg(target_arch = "wasm32")]
    {
        let _ = dir;
        match name {
            "cinematic" => Ok(include_bytes!("../../presets/cinematic.json").to_vec()),
            "preview" => Ok(include_bytes!("../../presets/preview.json").to_vec()),
            other => Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("preset {other}: a browser build has cinematic and preview"))),
        }
    }
}
