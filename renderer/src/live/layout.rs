//! The live passes' layout. What live.wgsl declares is read from it when the crate is built (build.rs, with naga:
//! `BINDINGS`, each binding's name, number, kind and element stride; `ENTRY_POINTS`, the bindings each entry point
//! uses); here is only what the shader cannot say: what each binding is bound to and how many elements a buffer of
//! the passes' own holds (`RESOURCES`), and the passes' order and the grid each runs over (`PASSES`). `check` holds
//! the two to each other (a unit test, and every view as it starts). A buffer or a pass more: its declaration or
//! entry point in live.wgsl, a line here.

/// A binding live.wgsl declares.
pub(super) struct Binding {
    pub(super) name: &'static str,
    pub(super) binding: u32,
    pub(super) kind: Kind,
}

pub(super) enum Kind {
    Uniform,
    /// An array: its element's stride, and its length if the shader fixes it (else 0).
    Storage { read_only: bool, stride: u64, len: u64 },
    Texture(wgpu::TextureViewDimension),
    Sampler,
}

include!(concat!(env!("OUT_DIR"), "/live_layout.rs"));

/// What a binding is bound to.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Res {
    /// A buffer of the passes' own, of this many elements.
    Own(Count),
    /// The frame's parameters (`LiveParams`).
    Params,
    /// The trace's samples (the scene's accumulation buffer).
    Samples,
    /// What the kernel leaves for the passes (`gpu::KernelBufs`): the specular-lobe rays, the motion, the direct light.
    KernelRefl,
    KernelMotion,
    KernelDirect,
    /// The tonemap LUT and its sampler.
    Lut,
    LutSampler,
}

/// How many elements a buffer holds, for an output of w x h pixels (a traced grid is at most that).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Count {
    /// That many per output pixel (2: a lasting buffer's halves, or two slots).
    Pixels(u64),
    /// One per texel of the quarter-resolution grid.
    Quarter,
    /// One per texel of the bloom pyramid's levels and stages.
    Bloom,
    /// The length the shader gives the array.
    Declared,
}

/// Each binding's resource, by the name live.wgsl gives it.
pub(super) const RESOURCES: &[(&str, Res)] = &[
    ("L", Res::Params),
    ("sample_px", Res::Samples),
    ("diffuse_hist", Res::Own(Count::Pixels(2))),
    ("color_hist", Res::Own(Count::Pixels(2))),
    ("gbuffer", Res::Own(Count::Pixels(2))),
    ("spec_hist", Res::Own(Count::Pixels(2))),
    ("diffuse_mom", Res::Own(Count::Pixels(2))),
    ("diffuse_fast", Res::Own(Count::Pixels(2))),
    ("up_state", Res::Own(Count::Pixels(2))),
    ("sample_vel", Res::Own(Count::Pixels(1))),
    ("out_px", Res::Own(Count::Pixels(1))),
    ("lut", Res::Lut),
    ("lut_samp", Res::LutSampler),
    ("ae_bins", Res::Own(Count::Declared)),
    ("ae_state", Res::Own(Count::Declared)),
    ("lumq", Res::Own(Count::Quarter)),
    ("lumb", Res::Own(Count::Quarter)),
    ("fa", Res::Own(Count::Pixels(1))),
    ("fb", Res::Own(Count::Pixels(1))),
    ("bloom_a", Res::Own(Count::Bloom)),
    ("bloom_b", Res::Own(Count::Quarter)),
    ("spec_pp", Res::Own(Count::Pixels(2))),
    ("sb", Res::Own(Count::Pixels(1))),
    ("refl_in", Res::KernelRefl),
    ("refl_res", Res::Own(Count::Pixels(1))),
    ("motion_in", Res::KernelMotion),
    ("direct_in", Res::KernelDirect),
    ("lock_fwd", Res::Own(Count::Pixels(1))),
];

/// The grid a pass runs over, in invocations (workgroups of 8 x 8).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Grid {
    /// The traced grid.
    Traced,
    /// The previous frame's traced grid.
    PrevTraced,
    /// The output grid.
    Output,
    /// A quarter of it each way.
    Quarter,
    /// Its 16 x 16 tiles.
    Tiles16,
    /// One workgroup.
    One,
}

/// The passes, in the order a frame runs them (live.wgsl's buffer table says which share a buffer when).
pub(super) const PASSES: &[(&str, Grid)] = &[
    ("refl", Grid::Traced),
    ("dtemporal", Grid::Traced),
    ("atrous0", Grid::Traced),
    ("atrous1", Grid::Traced),
    ("atrous2", Grid::Traced),
    ("atrous3", Grid::Traced),
    ("atrous4", Grid::Traced),
    ("stemporal", Grid::Traced),
    ("sfilt0", Grid::Traced),
    ("sfilt1", Grid::Traced),
    ("sfilt2", Grid::Traced),
    ("lock_carry", Grid::PrevTraced),
    ("compose", Grid::Traced),
    ("judge", Grid::Traced),
    ("reject", Grid::Traced),
    ("upscale", Grid::Output),
    ("dof_tiles", Grid::Tiles16),
    ("dof", Grid::Output),
    ("mblur", Grid::Output),
    ("lum_down", Grid::Quarter),
    ("exposure", Grid::One),
    ("lum_blur", Grid::Quarter),
    ("bloom_down", Grid::Quarter),
    ("bloom_dn1", Grid::Quarter),
    ("bloom_dn2", Grid::Quarter),
    ("bloom_dn3", Grid::Quarter),
    ("bloom_dn4", Grid::Quarter),
    ("bloom_h5", Grid::Quarter),
    ("bloom_v5", Grid::Quarter),
    ("bloom_h4", Grid::Quarter),
    ("bloom_v4", Grid::Quarter),
    ("bloom_h3", Grid::Quarter),
    ("bloom_v3", Grid::Quarter),
    ("bloom_h2", Grid::Quarter),
    ("bloom_v2", Grid::Quarter),
    ("bloom_h1", Grid::Quarter),
    ("bloom_v1", Grid::Quarter),
    ("bloom_h0", Grid::Quarter),
    ("bloom_v0", Grid::Quarter),
    ("post", Grid::Output),
];

impl Grid {
    /// Invocations, for an output of `out`, a traced grid of `traced` and the previous frame's `prev`.
    pub(super) fn size(self, out: (u32, u32), traced: (u32, u32), prev: (u32, u32)) -> (u32, u32) {
        match self {
            Grid::Traced => traced,
            Grid::PrevTraced => (prev.0.max(1), prev.1.max(1)),
            Grid::Output => out,
            Grid::Quarter => (out.0.div_ceil(4), out.1.div_ceil(4)),
            Grid::Tiles16 => (out.0.div_ceil(16), out.1.div_ceil(16)),
            Grid::One => (8, 8),
        }
    }
}

/// The binding live.wgsl gives `name`.
pub(super) fn binding(name: &str) -> &'static Binding {
    BINDINGS.iter().find(|b| b.name == name).unwrap_or_else(|| panic!("live.wgsl binds no `{name}`"))
}

/// The resource of a binding.
pub(super) fn resource(b: &Binding) -> Res {
    RESOURCES.iter().find(|r| r.0 == b.name).unwrap_or_else(|| panic!("live.wgsl binds `{}` (binding {}): it has no line in RESOURCES", b.name, b.binding)).1
}

/// The bindings the entry point `name` uses.
pub(super) fn uses(name: &str) -> &'static [u32] {
    ENTRY_POINTS.iter().find(|e| e.0 == name).unwrap_or_else(|| panic!("live.wgsl has no entry point `{name}`")).1
}

/// The table and the shader agree: every binding has a resource that fits its kind and every resource a binding,
/// every entry point is a pass and every pass an entry point, each once.
pub(super) fn check() {
    let mut problems = vec![];
    for b in BINDINGS {
        match RESOURCES.iter().filter(|r| r.0 == b.name).count() {
            1 => {}
            0 => problems.push(format!("`{}` (binding {}) has no line in RESOURCES", b.name, b.binding)),
            _ => problems.push(format!("`{}` has several lines in RESOURCES", b.name)),
        }
    }
    for (name, res) in RESOURCES {
        let Some(b) = BINDINGS.iter().find(|b| b.name == *name) else {
            problems.push(format!("RESOURCES names `{name}`, which live.wgsl does not bind"));
            continue;
        };
        let fits = match (res, &b.kind) {
            (Res::Params, Kind::Uniform) | (Res::Lut, Kind::Texture(_)) | (Res::LutSampler, Kind::Sampler) => true,
            (Res::Samples | Res::KernelRefl | Res::KernelMotion | Res::KernelDirect, Kind::Storage { read_only, .. }) => *read_only,
            (Res::Own(Count::Declared), Kind::Storage { len, .. }) => *len > 0,
            (Res::Own(_), Kind::Storage { len, .. }) => *len == 0,
            _ => false,
        };
        if !fits {
            problems.push(format!("`{name}`: {res:?} for a binding that is not one"));
        }
    }
    for (name, _) in ENTRY_POINTS {
        match PASSES.iter().filter(|p| p.0 == *name).count() {
            1 => {}
            0 => problems.push(format!("live.wgsl's entry point `{name}` is not in PASSES")),
            _ => problems.push(format!("`{name}` is in PASSES more than once")),
        }
    }
    for (name, _) in PASSES {
        if !ENTRY_POINTS.iter().any(|e| e.0 == *name) {
            problems.push(format!("PASSES names `{name}`, which live.wgsl has no entry point for"));
        }
    }
    assert!(problems.is_empty(), "the live passes' layout (live/layout.rs) and live.wgsl disagree:\n  {}", problems.join("\n  "));
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_table_and_the_shader_agree() {
        super::check();
    }
}
