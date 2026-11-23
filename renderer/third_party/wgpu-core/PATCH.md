# wgpu 30.0.1 and wgpu-core 30.0.1, patched for ThreeRT

These are wgpu 30.0.1 (`third_party/wgpu`) and wgpu-core 30.0.1 (this directory) from crates.io (MIT OR Apache-2.0,
the licenses beside each) with one change: **a TLAS build writes only the instances that changed since the last one.**
`renderer/Cargo.toml` uses them through `[patch.crates-io]`.

## Why

A live view that streams a world adds and removes instances every frame (a city's cells, their kit pieces as
instances: docs/RUNTIME.md "Streaming"). Every commit builds the TLAS, and upstream's build is linear in the
instances on the CPU, three times over, whatever changed:

- `wgpu` hands wgpu-core every instance slot (`Tlas::instances`, empty ones too);
- wgpu-core resolves each one's BLAS id, then for each one inserts its BLAS into the command's tracker, serialises
  it into a fresh `Vec<u8>`, and pushes its BLAS onto the TLAS's dependencies (one entry per instance);
- every command buffer that then uses the TLAS validates and marks resident that dependency list, entry by entry.

With 470,000 instances (TokyoRT's streamed Shinjuku, its windows as kit instances) the build took 14.6 ms of a
commit and the frames paid for the list again. `Tlas::lowest_unmodified` was meant for this, but it records only how
far up a slot was touched, nothing uses it, and its comment says as much.

## What

- **wgpu** (`src/api/tlas.rs`, `src/api/device.rs`, `src/backend/wgpu_core.rs`): a `Tlas` keeps the range of slots
  touched since its last build (`dirty_lo..dirty_hi`, atomics: a build reads it through `&Tlas`). Every mutable
  accessor (`index_mut`, `get_mut_single`, `get_mut_slice`, both `IndexMut`s) widens it; a new TLAS starts with all of
  its slots dirty. A build hands wgpu-core that range alone, with where it starts (`first`) and how many slots the
  TLAS has (`total`), and resets it.
- **wgpu-core** (`src/ray_tracing.rs`, `src/resource.rs`, `src/device/ray_tracing.rs`, `src/command/ray_tracing.rs`,
  `src/lock/rank.rs`, `src/device/trace/record.rs`): `TlasPackage` / `OwnedTlasPackage` carry `first` and `total`. A
  TLAS keeps its slots between builds (`resource::TlasSlots`: each slot's BLAS, each BLAS once with how many slots name
  it). A build:
  - serialises the changed slots alone and copies them to their place in the TLAS's instance buffer (the slots keep
    their positions: upstream packs the present instances together, so one removed moved all after it);
  - writes an empty slot as a copy of the TLAS's first BLAS (`TlasSlots::null`, held for the TLAS's life) with mask 0
    and a zero transform: no extent, never hit;
  - builds over all `total` slots, from the instance buffer as it now is;
  - gives the TLAS its BLASes once each as its dependencies (validation and residency per use: the distinct BLASes,
    not the instances).

The validation upstream does per instance is done for the changed ones (custom index, vertex return), and the count
against `max_instances` for `total`.

## Measured

TokyoRT's day tour (90 s of fast flight over Shinjuku, streaming all the way; M3 Max; means after the first 20 s):

| | commit | of which the TLAS | CPU a frame | interval p50 / p95 |
|---|---|---|---|---|
| merged cells, upstream | 13.7 ms | 6.5 ms | 28.1 ms | 23.7 / 55.1 ms |
| merged cells, patched | 6.5 ms | 1.4 ms | 20.4 ms | 20.3 / 38.4 ms |
| kit instances (470 k), upstream | 18.9 ms | 14.6 ms | 33.6 ms | 34.0 / 69.8 ms |
| kit instances (470 k), patched | 4.9 ms | 2.4 ms | 16.4 ms | 16.7 / 43.3 ms |

`cargo test --release --lib`: 250 passed (the in-place tests add, move, hide and remove instances and compare with
the scene rebuilt).

## Updating wgpu

Take the new release's two crates, apply the change above, and keep this file. Upstream may grow partial TLAS updates
of its own (the comments on `Tlas::get_mut_slice` expect them): then this patch can go.
