//! Where the engine's library reads files from. Natively: the file system, as before (these are `std::fs`'s and the
//! `image` / `gltf` crates' own readers, called from one place). In a browser (wasm32) there is no file system: the
//! page fetches what a scene names (`World::files`) and hands the bytes over with `provide` before the scene is built,
//! and the two presets are compiled in. The paths are the ones the native build would open (asset root joined with
//! the scene's uri), so the same scene code names the same files on both.
//!
//! An asset is a file of the game's package (`models/car.glb`, under the asset root) or one the game keeps in its
//! store (`user:maps/de_dust2/map.glb`: docs/RUNTIME.md "Files"), under the directory the app names with
//! `set_user_root`. `asset_path` is where either is; every reader of a scene's files goes through it.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// The scheme of an asset in the game's store.
pub const USER_SCHEME: &str = "user:";

/// The longest path in a game's store.
pub const MAX_STORE_PATH: usize = 200;

static USER_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// The directory `user:` assets resolve against: the store of the game the app runs (`None`: no store, and a `user:`
/// asset is missing as a missing file is). One a process; an app sets it before it builds the game's scene.
pub fn set_user_root(root: Option<&Path>) {
    *USER_ROOT.write().unwrap_or_else(|e| e.into_inner()) = root.map(Path::to_path_buf);
}

/// The directory `set_user_root` named.
pub fn user_root() -> Option<PathBuf> {
    USER_ROOT.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A path in a game's store: relative, `/`-separated, segments of `[A-Za-z0-9._-]`, no `.` or `..` segment, at most
/// 200 characters. The error says what is wrong with it.
pub fn check_store_path(path: &str) -> std::result::Result<(), String> {
    if path.is_empty() {
        return Err("a store path is not empty".into());
    }
    if path.len() > MAX_STORE_PATH {
        return Err(format!("a store path is at most {MAX_STORE_PATH} characters"));
    }
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(format!("\"{path}\": a store path is relative, with no empty, \".\" or \"..\" segment"));
        }
        if !seg.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
            return Err(format!("\"{path}\": a store path's segments are of A-Z a-z 0-9 . _ -"));
        }
    }
    Ok(())
}

/// Where the asset `uri` is: under `assets` for a file of the game's package, under the user root for `user:<path>`.
/// A `user:` asset without a user root, or with a path that is not a store path, is a path that is not there
/// (`user:/missing`: it is missing, as a missing file is, and never outside the store).
pub fn asset_path(assets: &Path, uri: &str) -> PathBuf {
    match uri.strip_prefix(USER_SCHEME) {
        Some(rel) => match (user_root(), check_store_path(rel)) {
            (Some(root), Ok(())) => root.join(rel),
            _ => PathBuf::from("user:/missing"),
        },
        None => assets.join(uri),
    }
}

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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// A one-triangle glTF with its buffer inline.
    fn triangle_gltf() -> String {
        use base64::Engine;
        let mut buf: Vec<u8> = [0f32, 0., 0., 1., 0., 0., 0., 1., 0.].iter().flat_map(|v| v.to_le_bytes()).collect();
        buf.extend([0u16, 1, 2, 0].iter().flat_map(|v| v.to_le_bytes()));
        let data = base64::engine::general_purpose::STANDARD.encode(&buf);
        format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1}}]}}],
            "buffers":[{{"byteLength":44,"uri":"data:application/octet-stream;base64,{data}"}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":6}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
                         {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}]}}"#
        )
    }

    /// `user:` assets (docs/RUNTIME.md "Files"): a scene names a file of the game's store, and the engine reads it from
    /// the directory the app gave, never outside it; without one the asset is missing. (One test: the user root is the
    /// process's.)
    #[test]
    fn user_assets_resolve_against_the_store() {
        let dir = std::env::temp_dir().join(format!("threert-user-assets-{}", std::process::id()));
        let (assets, store) = (dir.join("game"), dir.join("store"));
        std::fs::create_dir_all(assets.join("models")).unwrap();
        std::fs::create_dir_all(store.join("maps/de_x")).unwrap();
        std::fs::write(store.join("maps/de_x/map.gltf"), triangle_gltf()).unwrap();
        std::fs::write(assets.join("models/tri.gltf"), triangle_gltf()).unwrap();
        // The scene's paths: a store path after `user:`, nothing else.
        for ok in ["user:maps/de_x/map.gltf", "models/tri.gltf"] {
            assert!(crate::world::check_uri(ok).is_ok(), "{ok}");
        }
        for bad in ["user:../game/models/tri.gltf", "user:/etc/passwd", "user:", "user:a b", "other:x", "../x", "/abs"] {
            assert!(crate::world::check_uri(bad).is_err(), "{bad}");
        }
        set_user_root(None);
        assert_eq!(asset_path(&assets, "models/tri.gltf"), assets.join("models/tri.gltf"));
        assert!(!asset_path(&assets, "user:maps/de_x/map.gltf").exists(), "no store: missing");
        assert!(crate::scene::model_gltf("user:maps/de_x/map.gltf", &assets).is_err());
        set_user_root(Some(&store));
        assert_eq!(asset_path(&assets, "user:maps/de_x/map.gltf"), store.join("maps/de_x/map.gltf"));
        assert_eq!(asset_path(&assets, "user:../game/models/tri.gltf"), PathBuf::from("user:/missing"), "never outside the store");
        let m = crate::scene::model_gltf("user:maps/de_x/map.gltf", &assets).unwrap();
        assert_eq!(m.triangles(glam::Mat4::IDENTITY, None, None).unwrap().len(), 1);
        assert_eq!(crate::scene::model_gltf("models/tri.gltf", &assets).unwrap().triangles(glam::Mat4::IDENTITY, None, None).unwrap().len(), 1);
        // A sound from the store too.
        #[cfg(feature = "audio")]
        {
            let samples: Vec<i16> = (0..2205).map(|i| ((i as f32 * 0.1).sin() * 8000.0) as i16).collect();
            std::fs::write(store.join("maps/de_x/ambient.wav"), crate::audio::tests_wav(22_050, &samples)).unwrap();
            let mut a = crate::audio::Audio::new();
            a.open(&assets);
            assert!(a.sound("user:maps/de_x/ambient.wav").is_some());
            assert!(a.sound("user:maps/de_x/missing.wav").is_none());
            assert!(a.sound("user:../game/x.wav").is_none());
            assert_eq!(a.take_log().len(), 2);
        }
        set_user_root(None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
