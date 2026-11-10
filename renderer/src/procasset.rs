//! Procedural geometry data (docs/MATERIAL_FUNCTIONS.md, "Geometry functions"): a geometry's boxes and the game's
//! data words, validated and identified by the SHA-256 of their bytes, whether the scene code passed them inline or
//! they come from an asset file. Asset files may come from untrusted games: every field is checked before use and
//! nothing is read past what the file holds.
//!
//! File format (little-endian throughout):
//!
//! | bytes | field |
//! |---|---|
//! | 0..8 | magic `TRTPROC\0` |
//! | 8..12 | format version, 1 |
//! | 12..16 | byte-order mark 0x01020304 (written little-endian: bytes 04 03 02 01) |
//! | 16..20 | box count n (≤ `world::PROC_MAX_BOXES`) |
//! | 20..24 | data word count m (≤ `PROC_MAX_WORDS`) |
//! | 24..32 | reserved, 0 |
//! | 32..64 | SHA-256 of the payload (bytes 64..) |
//! | 64..64 + 24 n | boxes: min xyz, max xyz as f32, finite, min ≤ max |
//! | .. + 4 m | data: u32 words |
//!
//! The file is exactly 64 + 24 n + 4 m bytes long.

use crate::world::PROC_MAX_BOXES;
use anyhow::{anyhow, bail, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;

pub const MAGIC: &[u8; 8] = b"TRTPROC\0";
pub const VERSION: u32 = 1;
pub const BOM: u32 = 0x0102_0304;
pub const HEADER: usize = 64;
/// Data words one geometry may hold (4 GiB of data; the binding's own limit applies on top).
pub const PROC_MAX_WORDS: usize = 1 << 30;

/// A geometry's boxes (min xyz, max xyz each, object space) and data words, checked, with the SHA-256 of their bytes as
/// laid out in the file's payload (its identity: GPU copies of the data are reused by it).
#[derive(Debug)]
pub struct ProcBlob {
    pub boxes: Vec<f32>,
    pub data: Vec<u32>,
    pub hash: [u8; 32],
    /// The software traversal's BVH over the boxes (`sw_bvh`), built the first time a software render asks.
    bvh: std::sync::OnceLock<(Vec<u32>, Vec<u32>)>,
}

fn check_boxes(boxes: &[f32]) -> Result<()> {
    if boxes.len() % 6 != 0 {
        bail!("boxes must come as min xyz, max xyz");
    }
    if boxes.len() / 6 > PROC_MAX_BOXES {
        bail!("more than {PROC_MAX_BOXES} boxes");
    }
    if boxes.chunks(6).any(|b| !b.iter().all(|v| v.is_finite()) || b[0] > b[3] || b[1] > b[4] || b[2] > b[5]) {
        bail!("a box is not finite or has min > max");
    }
    Ok(())
}

fn payload_hash(boxes: &[f32], data: &[u32]) -> [u8; 32] {
    let mut h = Sha256::new();
    for v in boxes {
        h.update(v.to_le_bytes());
    }
    if cfg!(target_endian = "little") {
        h.update(bytemuck::cast_slice::<u32, u8>(data));
    } else {
        for w in data {
            h.update(w.to_le_bytes());
        }
    }
    h.finalize().into()
}

impl ProcBlob {
    /// Boxes and data the scene code passed (checked, hashed).
    pub fn new(boxes: Vec<f32>, data: Vec<u32>) -> Result<ProcBlob> {
        check_boxes(&boxes)?;
        if data.len() > PROC_MAX_WORDS {
            bail!("more than {PROC_MAX_WORDS} data words");
        }
        let hash = payload_hash(&boxes, &data);
        Ok(ProcBlob { boxes, data, hash, bvh: Default::default() })
    }

    /// An asset file's bytes: every header field checked, the length exact, the hash verified.
    pub fn parse(bytes: &[u8]) -> Result<ProcBlob> {
        if bytes.len() < HEADER {
            bail!("{} bytes: shorter than the {HEADER}-byte header", bytes.len());
        }
        if &bytes[0..8] != MAGIC {
            bail!("not a procedural geometry file (bad magic)");
        }
        let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        let version = u32_at(8);
        if version != VERSION {
            bail!("format version {version}; this renderer reads version {VERSION}");
        }
        if u32_at(12) != BOM {
            bail!("byte-order mark {:#010x}: the file must be little-endian", u32_at(12));
        }
        let (n, m) = (u32_at(16) as usize, u32_at(20) as usize);
        if n > PROC_MAX_BOXES {
            bail!("{n} boxes: more than {PROC_MAX_BOXES}");
        }
        if m > PROC_MAX_WORDS {
            bail!("{m} data words: more than {PROC_MAX_WORDS}");
        }
        if bytes[24..32].iter().any(|&b| b != 0) {
            bail!("reserved header bytes are not zero");
        }
        let want = (HEADER as u64) + 24 * n as u64 + 4 * m as u64;
        if bytes.len() as u64 != want {
            bail!("{} bytes where its header says {want} (64 + 24 x {n} boxes + 4 x {m} words)", bytes.len());
        }
        let payload = &bytes[HEADER..];
        let hash: [u8; 32] = Sha256::digest(payload).into();
        if hash[..] != bytes[32..64] {
            bail!("content hash does not match the payload");
        }
        let boxes: Vec<f32> = payload[..24 * n].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        let data: Vec<u32> = payload[24 * n..].chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        check_boxes(&boxes)?;
        Ok(ProcBlob { boxes, data, hash, bvh: Default::default() })
    }

    /// The asset file of these boxes and data (tests; tools/lib/proc-asset.ts writes them for games).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + 24 * (self.boxes.len() / 6) + 4 * self.data.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&BOM.to_le_bytes());
        out.extend_from_slice(&((self.boxes.len() / 6) as u32).to_le_bytes());
        out.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&self.hash);
        for v in &self.boxes {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for w in &self.data {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out
    }

    /// Words on the GPU: boxes, data, then with the software traversal its BVH (`sw_bvh`).
    pub fn words(&self, sw: bool) -> usize {
        self.boxes.len() + self.data.len() + if sw { let (n, o) = self.sw_bvh(); n.len() + o.len() } else { 0 }
    }

    /// The software traversal's BVH over the boxes (bvh::build_boxes): its nodes as words (8 each: bmin, left or first,
    /// bmax, count), and the box index of each leaf slot (the geometry function gets the box's own index).
    pub fn sw_bvh(&self) -> &(Vec<u32>, Vec<u32>) {
        self.bvh.get_or_init(|| {
            let b = crate::bvh::build_boxes(&self.boxes);
            (bytemuck::cast_slice(&b.nodes).to_vec(), b.order)
        })
    }
}

/// An asset file under the asset root (the path already checked by the world), parsed once per file version.
pub fn load(uri: &str, assets: &Path) -> Result<Arc<ProcBlob>> {
    static CACHE: std::sync::Mutex<Option<std::collections::HashMap<String, Arc<ProcBlob>>>> = std::sync::Mutex::new(None);
    let path = assets.join(uri);
    let key = crate::scene::file_key(&path);
    if let Some(b) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&key)) {
        return Ok(b.clone());
    }
    let bytes = std::fs::read(&path).map_err(|e| anyhow!("procedural geometry {uri}: {e}"))?;
    let b = Arc::new(ProcBlob::parse(&bytes).map_err(|e| anyhow!("procedural geometry {uri}: {e}"))?);
    let mut c = CACHE.lock().unwrap();
    let c = c.get_or_insert_with(std::collections::HashMap::new);
    if c.len() > 256 {
        c.clear();
    }
    c.insert(key, b.clone());
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ProcBlob {
        ProcBlob::new(vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, -1.0, -1.0, -1.0, 0.0, 0.0, 0.0], vec![0xffff_ffff, 0x7fc0_0001, 0, 1]).unwrap()
    }

    #[test]
    fn round_trip() {
        let a = sample();
        let b = ProcBlob::parse(&a.encode()).unwrap();
        assert_eq!((a.boxes, a.data, a.hash), (b.boxes, b.data, b.hash));
    }

    /// The software traversal's BVH over boxes: every box in exactly one leaf slot, the root bounding them all.
    #[test]
    fn box_bvh_covers_every_box() {
        let mut boxes = vec![];
        for k in 0..1000 {
            let (x, y) = ((k % 40) as f32, (k / 40) as f32 * 0.5);
            boxes.extend_from_slice(&[x, y, 0.0, x + 0.5, y + 0.25, 0.1]);
        }
        let b = ProcBlob::new(boxes, vec![]).unwrap();
        let (nodes, order) = b.sw_bvh();
        let mut seen = order.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..1000).collect::<Vec<u32>>());
        let root: &[crate::bvh::GpuNode] = bytemuck::cast_slice(&nodes[..8]);
        assert_eq!((root[0].bmin, root[0].bmax), ([0.0, 0.0, 0.0], [39.5, 12.25, 0.1]));
        assert_eq!(b.words(true), 6000 + nodes.len() + 1000);
    }

    /// Every malformed file is refused with an error, never a panic or a read past its end.
    #[test]
    fn malformed_files_are_refused() {
        let good = sample().encode();
        let bad = |f: &dyn Fn(&mut Vec<u8>)| {
            let mut b = good.clone();
            f(&mut b);
            ProcBlob::parse(&b).unwrap_err().to_string()
        };
        assert!(ProcBlob::parse(&good[..10]).is_err());
        assert!(bad(&|b| b[0] = b'X').contains("magic"));
        assert!(bad(&|b| b[8] = 2).contains("version"));
        assert!(bad(&|b| b[12..16].copy_from_slice(&BOM.to_be_bytes())).contains("little-endian"));
        assert!(bad(&|b| b.truncate(b.len() - 4)).contains("header says"));
        assert!(bad(&|b| b.push(0)).contains("header says"));
        assert!(bad(&|b| b[16..20].copy_from_slice(&u32::MAX.to_le_bytes())).contains("boxes"));
        assert!(bad(&|b| b[20..24].copy_from_slice(&0x4000_0000u32.to_le_bytes())).contains("header says"), "huge counts against a short file");
        assert!(bad(&|b| b[24] = 1).contains("reserved"));
        assert!(bad(&|b| b[64 + 24 * 2] ^= 1).contains("hash"), "a flipped data bit");
        // A box with min > max under a matching hash: refused by the box check.
        let mut nb = ProcBlob { boxes: vec![1.0, 0.0, 0.0, 0.0, 1.0, 1.0], data: vec![], hash: [0; 32], bvh: Default::default() };
        nb.hash = payload_hash(&nb.boxes, &nb.data);
        assert!(ProcBlob::parse(&nb.encode()).unwrap_err().to_string().contains("min > max"));
        nb.boxes[0] = f32::NAN;
        nb.hash = payload_hash(&nb.boxes, &nb.data);
        assert!(ProcBlob::parse(&nb.encode()).is_err());
    }
}
