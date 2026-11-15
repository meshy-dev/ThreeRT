//! The scene's read-only data as the kernel binds it: two buffers, each a row of named sections.
//!
//! `Table`s (pathtrace.wgsl `tables`, an array of vec4s): the scene's records. `Stream`s (`words`, an array of
//! 32-bit words): its flat arrays. A section holds one of them and the room after it, and the kernel finds it by the
//! element `Params::table_first` / `Params::word_first` name (`Heap::firsts`), so what a scene has more of is
//! another section, not another binding. Sections start at multiples of `ALIGN` bytes: a pass that writes one binds it as a buffer of
//! its own (`Heap::binding`), and a binding's offset is a multiple of that (WebGPU's
//! `minStorageBufferOffsetAlignment`). One that outgrows its room is given more in a new buffer, the contents copied
//! on the GPU (`Heap::grow`); the scene's bind group is then made again (`Gpu::rebind_scene`).

use std::marker::PhantomData;

/// Where a section may start, and the least it holds (an empty table still reads as zeros), in bytes.
pub const ALIGN: u64 = 256;
const LEAST: u64 = 512;

/// A heap's sections: their count, their element's size in bytes and each one's place in `Params`.
pub trait Section: Copy {
    const COUNT: usize;
    const UNIT: u64;
    const LABEL: &'static str;
    fn index(self) -> usize;
}

/// The scene's tables, in `Params::table_first`'s order (pathtrace.wgsl `material_at` and the others). The first
/// is at element 0 whatever grows, so the kernel reads it with no offset: the one read most that has no view of its
/// own (merged.rs `TABLE_VIEWS`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Table {
    /// The emitters' tree (ltree.rs), a vec4 a node.
    LightTree,
    /// `scene::GpuMaterial`.
    Materials,
    /// `scene::GpuLight`.
    Lights,
    /// One sphere (centre, radius) per `LIGHT_GROUP` local lights (lib.rs `light_groups`).
    LightGroups,
    /// The emitter CDF: (triangle bits, cumulative weight, weight, area) per emissive triangle (lib.rs `emitter_cdf`).
    Emitters,
    /// Two vec4s a texture (texpack.rs).
    TexInfo,
    /// `Scene::object_stride` vec4s an object.
    Objects,
    /// `scene::GpuInstance`: the model instances' records, then the procedural objects'.
    Instances,
    /// `scene::MediumGpu`.
    Media,
    /// The GGX energy table, 512 vec4s.
    Energy,
}

impl Section for Table {
    const COUNT: usize = 10;
    const UNIT: u64 = 16;
    const LABEL: &'static str = "tables";
    fn index(self) -> usize {
        self as usize
    }
}

/// The scene's flat arrays of 32-bit words, in `Params::word_first`'s order (pathtrace.wgsl `env_cdf_at`, `pword`,
/// `vcolor_word`). The first is at word 0 whatever grows, so the kernel reads it with no offset: the one read most.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stream {
    /// The environment's sampling CDF (floats): the rows' marginal CDF, then each row's.
    EnvCdf,
    /// The procedural geometry's region (scene::ProcLayout): headers and materials, then the geometries' blobs.
    Procs,
    /// Vertex colours: three sRGB8 words a triangle, in the triangles' GPU order.
    VertexColors,
}

impl Section for Stream {
    const COUNT: usize = 3;
    const UNIT: u64 = 4;
    const LABEL: &'static str = "words";
    fn index(self) -> usize {
        self as usize
    }
}

/// The kernel's group-1 buffers as the parts of one: the merged layout's (gpu.rs `KernelBufs`; pathtrace.wgsl
/// `MergedLive`, whose members are where these sections are: a test holds it). The radiance cache's keys and sums
/// and the loop guard's count, atomic words at fixed places; then the vec4 arrays, the sections of the struct's last
/// member: the cache's values, what the live passes read of a trace (the specular-lobe rays, the motion vectors, the
/// direct light), the light tiles' lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Live {
    CacheKeys,
    CacheAcc,
    Guard,
    CacheVal,
    Refl,
    Motion,
    Direct,
    Lists,
}

impl Section for Live {
    const COUNT: usize = 8;
    const UNIT: u64 = 4;
    const LABEL: &'static str = "live";
    fn index(self) -> usize {
        self as usize
    }
}

/// A heap: its buffer, and each section's first byte and size.
pub struct Heap<S: Section> {
    pub buffer: wgpu::Buffer,
    at: Vec<u64>,
    cap: Vec<u64>,
    _s: PhantomData<S>,
}

/// Where sections of at least `bytes` each are, their sizes, and the buffer's.
fn place(bytes: &[u64]) -> (Vec<u64>, Vec<u64>, u64) {
    let cap: Vec<u64> = bytes.iter().map(|b| (*b).max(LEAST).next_multiple_of(ALIGN)).collect();
    let at: Vec<u64> = cap.iter().scan(0u64, |end, c| Some(std::mem::replace(end, *end + c))).collect();
    let size = cap.iter().sum();
    (at, cap, size)
}

const USAGE: wgpu::BufferUsages = wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_DST).union(wgpu::BufferUsages::COPY_SRC);

impl<S: Section> Heap<S> {
    /// The bytes of a heap whose sections hold `bytes` each.
    pub fn size_for(bytes: &[u64]) -> u64 {
        place(bytes).2
    }

    /// Where its sections start, and the bytes each holds.
    pub fn starts_for(bytes: &[u64]) -> (Vec<u64>, Vec<u64>) {
        let (at, cap, _) = place(bytes);
        (at, cap)
    }

    /// A heap of zeros with sections of at least `bytes` each (`S::COUNT` of them), to be written through the queue.
    pub fn new(device: &wgpu::Device, bytes: &[u64]) -> Self {
        assert_eq!(bytes.len(), S::COUNT);
        let (at, cap, size) = place(bytes);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some(S::LABEL), size, usage: USAGE, mapped_at_creation: false });
        Heap { buffer, at, cap, _s: PhantomData }
    }

    /// The same, filled while it is made: `fill` is given `put(section, byte, data)`, which writes into the buffer's
    /// mapping (no copy through the queue).
    pub fn filled(device: &wgpu::Device, bytes: &[u64], fill: impl FnOnce(&mut dyn FnMut(S, u64, &[u8]))) -> Self {
        assert_eq!(bytes.len(), S::COUNT);
        let (at, cap, size) = place(bytes);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some(S::LABEL), size, usage: USAGE, mapped_at_creation: true });
        {
            let mut m = buffer.slice(..).get_mapped_range_mut().expect("mapped at creation");
            fill(&mut |s: S, byte: u64, data: &[u8]| {
                let k = s.index();
                assert!(byte + data.len() as u64 <= cap[k], "{}: section {k} holds {} bytes, {} written at {byte}", S::LABEL, cap[k], data.len());
                let o = (at[k] + byte) as usize;
                m.slice(o..o + data.len()).copy_from_slice(data);
            });
        }
        buffer.unmap();
        Heap { buffer, at, cap, _s: PhantomData }
    }

    /// A section's first byte in the buffer.
    pub fn at(&self, s: S) -> u64 {
        self.at[s.index()]
    }

    /// The bytes a section holds.
    pub fn cap(&self, s: S) -> u64 {
        self.cap[s.index()]
    }

    /// Each section's first element, for `Params`.
    pub fn firsts(&self) -> Vec<u32> {
        self.at.iter().map(|a| (a / S::UNIT) as u32).collect()
    }

    /// `data` into a section, `byte` bytes from its start.
    pub fn write(&self, q: &wgpu::Queue, s: S, byte: u64, data: &[u8]) {
        let k = s.index();
        assert!(byte + data.len() as u64 <= self.cap[k], "{}: section {k} holds {} bytes, {} written at {byte}", S::LABEL, self.cap[k], data.len());
        q.write_buffer(&self.buffer, self.at[k] + byte, data);
    }

    /// A section as a buffer of its own.
    pub fn binding(&self, s: S) -> wgpu::BindingResource<'_> {
        let k = s.index();
        wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.buffer, offset: self.at[k], size: std::num::NonZeroU64::new(self.cap[k]) })
    }

    /// The bytes of this heap with section `s` holding `bytes`.
    pub fn size_with(&self, s: S, bytes: u64) -> u64 {
        let mut want = self.cap.clone();
        want[s.index()] = bytes;
        place(&want).2
    }

    /// Section `s` with room for `bytes`: a new buffer, every section's contents copied on the GPU and submitted now,
    /// so writes queued after this land on the copy. The sections after it move: whoever holds the buffer or an
    /// offset (the scene's bind group, `Params`) reads them again.
    pub fn grow(&mut self, device: &wgpu::Device, q: &wgpu::Queue, s: S, bytes: u64) {
        let mut want = self.cap.clone();
        want[s.index()] = bytes;
        let (at, cap, size) = place(&want);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some(S::LABEL), size, usage: USAGE, mapped_at_creation: false });
        let mut e = device.create_command_encoder(&Default::default());
        for k in 0..S::COUNT {
            e.copy_buffer_to_buffer(&self.buffer, self.at[k], &buffer, at[k], self.cap[k].min(cap[k]));
        }
        q.submit([e.finish()]);
        (self.buffer, self.at, self.cap) = (buffer, at, cap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_start_where_a_pass_can_bind_them() {
        let (at, cap, size) = place(&[0, 1, 513, 4096]);
        assert_eq!(cap, [512, 512, 768, 4096]);
        assert_eq!(at, [0, 512, 1024, 1792]);
        assert_eq!(size, 5888);
        assert!(at.iter().all(|a| a % ALIGN == 0));
    }
}
