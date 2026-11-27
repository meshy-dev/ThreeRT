use alloc::{sync::Arc, vec::Vec};
use core::{
    cmp::max,
    num::NonZeroU64,
    ops::Deref,
};

use wgt::{math::align_to, BufferUsages, BufferUses, Features};

use crate::{
    command::encoder::EncodingState,
    ray_tracing::{AsAction, AsBuild, TlasBuild, ValidateAsActionsError},
    resource::InvalidResourceError,
};
use crate::{command::EncoderStateError, device::resource::CommandIndices};
use crate::{
    command::{ArcCommand, ArcReferences, CommandBufferMutable},
    device::queue::TempResource,
    global::Global,
    id::CommandEncoderId,
    init_tracker::MemoryInitKind,
    ray_tracing::{
        ArcBlasAabbGeometry, ArcBlasBuildEntry, ArcBlasGeometries, ArcBlasTriangleGeometry,
        ArcTlasInstance, ArcTlasPackage, BlasBuildEntry, BlasGeometries,
        BuildAccelerationStructureError, OwnedBlasBuildEntry, OwnedTlasPackage, TlasPackage,
    },
    resource::{Blas, BlasCompactState, Labeled, StagingBuffer, Tlas},
    scratch::ScratchBuffer,
    snatch::SnatchGuard,
};
use crate::{lock::RwLockWriteGuard, resource::RawResourceAccess};

use crate::id::{BlasId, TlasId};

struct BlasStore<'a> {
    blas: Arc<Blas>,
    entries: hal::AccelerationStructureEntries<'a, dyn hal::DynBuffer>,
    scratch_buffer_offset: u64,
}

struct UnsafeTlasStore<'a> {
    tlas: Arc<Tlas>,
    entries: hal::AccelerationStructureEntries<'a, dyn hal::DynBuffer>,
    scratch_buffer_offset: u64,
}

struct TlasStore<'a> {
    internal: UnsafeTlasStore<'a>,
    /// ThreeRT patch: the staging buffer's runs of changed slots and where each goes in the instance buffer.
    copies: Vec<hal::BufferCopy>,
}

impl Global {
    fn resolve_blas_id(&self, blas_id: BlasId) -> Result<Arc<Blas>, InvalidResourceError> {
        self.hub.blas_s.get(blas_id).get()
    }

    fn resolve_tlas_id(&self, tlas_id: TlasId) -> Result<Arc<Tlas>, InvalidResourceError> {
        self.hub.tlas_s.get(tlas_id).get()
    }

    pub fn command_encoder_mark_acceleration_structures_built(
        &self,
        command_encoder_id: CommandEncoderId,
        blas_ids: &[BlasId],
        tlas_ids: &[TlasId],
    ) -> Result<(), EncoderStateError> {
        profiling::scope!("CommandEncoder::mark_acceleration_structures_built");

        let hub = &self.hub;

        let cmd_enc = hub.command_encoders.get(command_encoder_id);

        let mut cmd_buf_data = cmd_enc.data.lock();
        cmd_buf_data.with_buffer(
            crate::command::EncodingApi::Raw,
            |cmd_buf_data| -> Result<(), BuildAccelerationStructureError> {
                let device = &cmd_enc.device;
                device.check_is_valid()?;
                device.require_features(Features::EXPERIMENTAL_RAY_QUERY)?;

                let mut build_command = AsBuild::with_capacity(blas_ids.len(), tlas_ids.len());

                for blas in blas_ids {
                    let blas = hub.blas_s.get(*blas).get()?;
                    build_command.blas_s_built.push(blas);
                }

                for tlas in tlas_ids {
                    let tlas = hub.tlas_s.get(*tlas).get()?;
                    build_command.tlas_s_built.push(TlasBuild {
                        tlas,
                        dependencies: Vec::new(),
                    });
                }

                cmd_buf_data.as_actions.push(AsAction::Build(build_command));
                Ok(())
            },
        )
    }

    pub fn command_encoder_build_acceleration_structures<'a>(
        &self,
        command_encoder_id: CommandEncoderId,
        blas_iter: impl Iterator<Item = BlasBuildEntry<'a>>,
        tlas_iter: impl Iterator<Item = TlasPackage<'a>>,
    ) -> Result<(), EncoderStateError> {
        profiling::scope!("CommandEncoder::build_acceleration_structures");

        let hub = &self.hub;

        let cmd_enc = hub.command_encoders.get(command_encoder_id);
        let mut cmd_buf_data = cmd_enc.data.lock();

        cmd_buf_data.push_with(|| -> Result<_, BuildAccelerationStructureError> {
            let blas = blas_iter
                .map(|blas_entry| {
                    let geometries = match blas_entry.geometries {
                        BlasGeometries::TriangleGeometries(triangle_geometries) => {
                            let tri_geo = triangle_geometries
                                .map(|tg| {
                                    Ok(ArcBlasTriangleGeometry {
                                        size: tg.size.clone(),
                                        vertex_buffer: self.resolve_buffer_id(tg.vertex_buffer)?,
                                        index_buffer: tg
                                            .index_buffer
                                            .map(|id| self.resolve_buffer_id(id))
                                            .transpose()?,
                                        transform_buffer: tg
                                            .transform_buffer
                                            .map(|id| self.resolve_buffer_id(id))
                                            .transpose()?,
                                        first_vertex: tg.first_vertex,
                                        vertex_stride: tg.vertex_stride,
                                        first_index: tg.first_index,
                                        transform_buffer_offset: tg.transform_buffer_offset,
                                    })
                                })
                                .collect::<Result<_, BuildAccelerationStructureError>>()?;
                            ArcBlasGeometries::TriangleGeometries(tri_geo)
                        }
                        BlasGeometries::AabbGeometries(aabb_geometries) => {
                            let aabb_geo = aabb_geometries
                                .map(|ag| {
                                    Ok(ArcBlasAabbGeometry {
                                        size: ag.size.clone(),
                                        stride: ag.stride,
                                        aabb_buffer: self.resolve_buffer_id(ag.aabb_buffer)?,
                                        primitive_offset: ag.primitive_offset,
                                    })
                                })
                                .collect::<Result<_, BuildAccelerationStructureError>>()?;
                            ArcBlasGeometries::AabbGeometries(aabb_geo)
                        }
                    };
                    Ok(ArcBlasBuildEntry {
                        blas: self.resolve_blas_id(blas_entry.blas_id)?,
                        geometries,
                    })
                })
                .collect::<Result<_, BuildAccelerationStructureError>>()?;

            let tlas = tlas_iter
                .map(|tlas_package| {
                    let instances = tlas_package
                        .instances
                        .map(|instance| {
                            instance
                                .as_ref()
                                .map(|instance| {
                                    Ok(ArcTlasInstance {
                                        blas: self.resolve_blas_id(instance.blas_id)?,
                                        transform: *instance.transform,
                                        custom_data: instance.custom_data,
                                        mask: instance.mask,
                                    })
                                })
                                .transpose()
                        })
                        .collect::<Result<_, BuildAccelerationStructureError>>()?;
                    Ok(ArcTlasPackage {
                        tlas: self.resolve_tlas_id(tlas_package.tlas_id)?,
                        instances,
                        lowest_unmodified: tlas_package.lowest_unmodified,
                        ranges: tlas_package.ranges,
                        total: tlas_package.total,
                    })
                })
                .collect::<Result<_, BuildAccelerationStructureError>>()?;

            Ok(ArcCommand::BuildAccelerationStructures { blas, tlas })
        })
    }
}

pub(crate) fn build_acceleration_structures(
    state: &mut EncodingState,
    blas: Vec<OwnedBlasBuildEntry<ArcReferences>>,
    tlas: Vec<OwnedTlasPackage<ArcReferences>>,
) -> Result<(), BuildAccelerationStructureError> {
    state
        .device
        .require_features(Features::EXPERIMENTAL_RAY_QUERY)?;

    let mut build_command = AsBuild::with_capacity(blas.len(), tlas.len());
    let mut input_barriers = Vec::<hal::BufferBarrier<dyn hal::DynBuffer>>::new();
    let mut scratch_buffer_blas_size = 0;
    let mut blas_storage = Vec::with_capacity(blas.len());
    iter_blas(
        blas.iter(),
        &mut build_command,
        &mut input_barriers,
        &mut scratch_buffer_blas_size,
        &mut blas_storage,
        state,
    )?;

    let mut scratch_buffer_tlas_size: u64 = 0;
    let mut tlas_storage = Vec::<TlasStore>::with_capacity(tlas.len());
    let mut instance_buffer_staging_source = Vec::<u8>::new();

    for package in tlas.iter() {
        let tlas = &package.tlas;
        state.tracker.tlas_s.insert_single(tlas.clone());

        let scratch_buffer_offset = scratch_buffer_tlas_size;
        scratch_buffer_tlas_size = scratch_buffer_tlas_size.saturating_add(align_to(
            tlas.size_info.build_scratch_size,
            u64::from(state.device.alignments.ray_tracing_scratch_buffer_alignment),
        ));


        // ThreeRT patch (PATCH.md): the slots from `first` that changed; the TLAS keeps the rest from its last builds.
        let mut slots = tlas.slots.lock();
        let slots = &mut *slots;
        let total = package.total as usize;
        if total > tlas.max_instance_count as usize {
            return Err(BuildAccelerationStructureError::TlasInstanceCountExceeded(
                tlas.error_ident(),
                package.total,
                tlas.max_instance_count,
            ));
        }
        if slots.blas.len() != total {
            slots.blas.resize(total, None);
        }
        let key = |b: &Arc<Blas>| Arc::as_ptr(b) as usize;
        // What the empty slots name, and where they stand: the first instance the TLAS is given, before any slot of
        // this build is written (an empty slot ahead of it in the list is written with it too).
        let had_null = slots.null.is_some();
        if !had_null {
            if let Some(first) = package.instances.iter().flatten().next() {
                slots.null = Some(first.blas.clone());
                slots.null_at = [first.transform[3], first.transform[7], first.transform[11]];
            }
        }
        // An empty slot: the null BLAS at a point (its size times EMPTY_SCALE), mask 0: never hit, and an instance as
        // any other to the structure's build. (Not a zero transform: among instances without extent Metal's
        // structure lost a scene's only wall to every ray, and a ray from the world origin its hits.)
        const EMPTY_SCALE: f32 = 1.0e-6;
        let empty_device = state.device.clone();
        let empty = |null: &Arc<Blas>, at: [f32; 3]| {
            empty_device.raw().tlas_instance_to_bytes(hal::TlasInstance {
                transform: [EMPTY_SCALE, 0.0, 0.0, at[0], 0.0, EMPTY_SCALE, 0.0, at[1], 0.0, 0.0, EMPTY_SCALE, at[2]],
                custom_data: 0,
                mask: 0,
                blas_address: null.handle,
                pipeline_intersection_data_offset: 0,
            })
        };
        // (one copy a run of adjacent slots)
        fn put(copies: &mut Vec<hal::BufferCopy>, staging: &mut Vec<u8>, k: usize, bytes: Vec<u8>) {
            let (at, len) = (staging.len() as u64, bytes.len() as u64);
            match copies.last_mut() {
                Some(c) if c.src_offset + c.size.get() == at && c.dst_offset + c.size.get() == k as u64 * len => {
                    c.size = wgt::BufferSize::new(c.size.get() + len).unwrap();
                }
                _ => {
                    if let Some(size) = wgt::BufferSize::new(len) {
                        copies.push(hal::BufferCopy { src_offset: at, dst_offset: k as u64 * len, size });
                    }
                }
            }
            staging.extend(bytes);
        }
        // (the slots of the changed ranges, in order, as `instances` lists them)
        let slots_of = package.ranges.iter().flat_map(|r| r.start as usize..(r.end as usize).min(total));
        let mut copies: Vec<hal::BufferCopy> = Vec::new();
        for (k, instance) in slots_of.zip(package.instances.iter()) {
            if let Some(instance) = instance {
                if instance.custom_data >= (1u32 << 24u32) {
                    return Err(BuildAccelerationStructureError::TlasInvalidCustomIndex(
                        tlas.error_ident(),
                    ));
                }
                if tlas
                    .flags
                    .contains(wgpu_types::AccelerationStructureFlags::ALLOW_RAY_HIT_VERTEX_RETURN)
                    && !instance
                        .blas
                        .flags
                        .contains(wgpu_types::AccelerationStructureFlags::ALLOW_RAY_HIT_VERTEX_RETURN)
                {
                    return Err(
                        BuildAccelerationStructureError::TlasDependentMissingVertexReturn(
                            tlas.error_ident(),
                            instance.blas.error_ident(),
                        ),
                    );
                }
            }
            let new = instance.as_ref().map(|i| i.blas.clone());
            let same = match (&slots.blas[k], &new) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            if !same {
                if let Some(old) = slots.blas[k].take() {
                    let id = key(&old);
                    if let Some(e) = slots.refs.get_mut(&id) {
                        e.1 -= 1;
                        if e.1 == 0 {
                            slots.refs.remove(&id);
                        }
                    }
                }
                if let Some(b) = &new {
                    slots.refs.entry(key(b)).or_insert_with(|| (b.clone(), 0)).1 += 1;
                }
                slots.blas[k] = new;
            }
            let bytes = match instance {
                Some(instance) => state.device.raw().tlas_instance_to_bytes(hal::TlasInstance {
                    transform: instance.transform,
                    custom_data: instance.custom_data,
                    mask: instance.mask,
                    blas_address: instance.blas.handle,
                    pipeline_intersection_data_offset: 0,
                }),
                // (an empty slot; with no BLAS yet nothing names it, and the build takes no instance)
                None => match &slots.null {
                    Some(null) => empty(null, slots.null_at),
                    None => {
                        let size = state.device.raw().tlas_instance_to_bytes(hal::TlasInstance {
                            transform: [0.0; 12],
                            custom_data: 0,
                            mask: 0,
                            blas_address: 0,
                            pipeline_intersection_data_offset: 0,
                        });
                        alloc::vec![0u8; size.len()]
                    }
                },
            };
            put(&mut copies, &mut instance_buffer_staging_source, k, bytes);
        }
        // The TLAS's first BLAS has come: the slots that were empty before it were written as nothing (the builds
        // until now took no instance); each is written as an empty slot now.
        if !had_null {
            if let Some(null) = slots.null.clone() {
                let mut written = alloc::vec![false; total];
                for r in package.ranges.iter() {
                    for k in r.start as usize..(r.end as usize).min(total) {
                        written[k] = true;
                    }
                }
                for k in 0..total {
                    if !written[k] && slots.blas[k].is_none() {
                        put(&mut copies, &mut instance_buffer_staging_source, k, empty(&null, slots.null_at));
                    }
                }
                // (in slot order, as the copies of a build are)
                copies.sort_by_key(|c| c.dst_offset);
            }
        }
        // The TLAS's dependencies: each BLAS once (the null one among them).
        let mut dependencies: Vec<Arc<Blas>> = slots.refs.values().map(|(b, _)| b.clone()).collect();
        if let Some(null) = &slots.null {
            if !slots.refs.contains_key(&key(null)) {
                dependencies.push(null.clone());
            }
        }
        for blas in &dependencies {
            state.tracker.blas_s.insert_single(blas.clone());
        }
        // (no BLAS at all: an empty TLAS)
        let instance_count = if slots.null.is_some() { total as u32 } else { 0 };

        build_command.tlas_s_built.push(TlasBuild {
            tlas: tlas.clone(),
            dependencies,
        });

        tlas_storage.push(TlasStore {
            internal: UnsafeTlasStore {
                tlas: tlas.clone(),
                entries: hal::AccelerationStructureEntries::Instances(
                    hal::AccelerationStructureInstances {
                        buffer: Some(tlas.instance_buffer.as_ref()),
                        offset: 0,
                        count: instance_count,
                    },
                ),
                scratch_buffer_offset,
            },
            copies,
        });
    }

    let Some(scratch_size) =
        wgt::BufferSize::new(max(scratch_buffer_blas_size, scratch_buffer_tlas_size))
    else {
        // if the size is zero there is nothing to build
        return Ok(());
    };

    if scratch_size.get() == u64::MAX {
        return Err(crate::device::DeviceError::OutOfMemory.into());
    }

    let scratch_buffer = ScratchBuffer::new(state.device, scratch_size)?;

    let scratch_buffer_barrier = hal::BufferBarrier::<dyn hal::DynBuffer> {
        buffer: scratch_buffer.raw(),
        usage: hal::StateTransition {
            from: BufferUses::ACCELERATION_STRUCTURE_SCRATCH,
            to: BufferUses::ACCELERATION_STRUCTURE_SCRATCH,
        },
    };

    let mut tlas_descriptors = Vec::with_capacity(tlas_storage.len());

    for &TlasStore {
        internal:
            UnsafeTlasStore {
                ref tlas,
                ref entries,
                ref scratch_buffer_offset,
            },
        ..
    } in &tlas_storage
    {
        if tlas.update_mode == wgt::AccelerationStructureUpdateMode::PreferUpdate {
            log::warn!("build_acceleration_structures called with PreferUpdate, but only rebuild is implemented");
        }
        tlas_descriptors.push(hal::BuildAccelerationStructureDescriptor {
            entries,
            mode: hal::AccelerationStructureBuildMode::Build,
            flags: tlas.flags,
            source_acceleration_structure: None,
            destination_acceleration_structure: tlas.try_raw(state.snatch_guard)?,
            scratch_buffer: scratch_buffer.raw(),
            scratch_buffer_offset: *scratch_buffer_offset,
        })
    }

    let blas_present = !blas_storage.is_empty();
    let tlas_present = !tlas_storage.is_empty();

    let raw_encoder = &mut state.raw_encoder;

    let mut blas_s_compactable = Vec::new();
    let mut descriptors = Vec::with_capacity(blas.len());

    for storage in &blas_storage {
        descriptors.push(map_blas(
            storage,
            scratch_buffer.raw(),
            state.snatch_guard,
            &mut blas_s_compactable,
        )?);
    }

    build_blas(
        *raw_encoder,
        blas_present,
        tlas_present,
        input_barriers,
        &descriptors,
        scratch_buffer_barrier,
        blas_s_compactable,
    );

    if tlas_present {
        let staging_buffer = if !instance_buffer_staging_source.is_empty() {
            let mut staging_buffer = StagingBuffer::new(
                state.device,
                wgt::BufferSize::new(instance_buffer_staging_source.len() as u64).unwrap(),
            )?;
            staging_buffer.write(&instance_buffer_staging_source);
            let flushed = staging_buffer.flush();
            Some(flushed)
        } else {
            None
        };

        unsafe {
            if let Some(ref staging_buffer) = staging_buffer {
                raw_encoder.transition_buffers(&[hal::BufferBarrier::<dyn hal::DynBuffer> {
                    buffer: staging_buffer.raw(),
                    usage: hal::StateTransition {
                        from: BufferUses::MAP_WRITE,
                        to: BufferUses::COPY_SRC,
                    },
                }]);
            }
        }

        let mut instance_buffer_barriers = Vec::new();
        for TlasStore {
            internal: UnsafeTlasStore { ref tlas, .. },
            copies,
        } in &tlas_storage
        {
            if copies.is_empty() {
                continue;
            }
            instance_buffer_barriers.push(hal::BufferBarrier::<dyn hal::DynBuffer> {
                buffer: tlas.instance_buffer.as_ref(),
                usage: hal::StateTransition {
                    from: BufferUses::COPY_DST,
                    to: BufferUses::TOP_LEVEL_ACCELERATION_STRUCTURE_INPUT,
                },
            });
            unsafe {
                raw_encoder.transition_buffers(&[hal::BufferBarrier::<dyn hal::DynBuffer> {
                    buffer: tlas.instance_buffer.as_ref(),
                    usage: hal::StateTransition {
                        from: BufferUses::TOP_LEVEL_ACCELERATION_STRUCTURE_INPUT,
                        to: BufferUses::COPY_DST,
                    },
                }]);
                raw_encoder.copy_buffer_to_buffer(
                    staging_buffer.as_ref().unwrap().raw(),
                    tlas.instance_buffer.as_ref(),
                    copies,
                );
            }
        }

        unsafe {
            raw_encoder.transition_buffers(&instance_buffer_barriers);

            raw_encoder.build_acceleration_structures(&tlas_descriptors);

            raw_encoder.place_acceleration_structure_barrier(hal::AccelerationStructureBarrier {
                usage: hal::StateTransition {
                    from: hal::AccelerationStructureUses::BUILD_OUTPUT,
                    to: hal::AccelerationStructureUses::SHADER_INPUT,
                },
            });
        }

        if let Some(staging_buffer) = staging_buffer {
            state
                .temp_resources
                .push(TempResource::StagingBuffer(staging_buffer));
        }
    }

    state
        .temp_resources
        .push(TempResource::ScratchBuffer(scratch_buffer));

    state.as_actions.push(AsAction::Build(build_command));

    Ok(())
}

impl CommandBufferMutable {
    pub(crate) fn validate_acceleration_structure_actions(
        &self,
        snatch_guard: &SnatchGuard,
        command_index_guard: &mut RwLockWriteGuard<CommandIndices>,
    ) -> Result<(), ValidateAsActionsError> {
        profiling::scope!("CommandEncoder::[submission]::validate_as_actions");
        for action in &self.as_actions {
            match action {
                AsAction::Build(build) => {
                    let build_command_index = NonZeroU64::new(
                        command_index_guard.next_acceleration_structure_build_command_index,
                    )
                    .unwrap();

                    command_index_guard.next_acceleration_structure_build_command_index += 1;
                    for blas in build.blas_s_built.iter() {
                        let mut state_lock = blas.compacted_state.lock();
                        *state_lock = match *state_lock {
                            BlasCompactState::Compacted => {
                                unreachable!("Should be validated out in build.")
                            }
                            // Reset the compacted state to idle. This means any prepares, before mapping their
                            // internal buffer, will terminate.
                            _ => BlasCompactState::Idle,
                        };
                        *blas.built_index.write() = Some(build_command_index);
                    }

                    for tlas_build in build.tlas_s_built.iter() {
                        for blas in &tlas_build.dependencies {
                            if blas.built_index.read().is_none() {
                                return Err(ValidateAsActionsError::UsedUnbuiltBlas(
                                    blas.error_ident(),
                                    tlas_build.tlas.error_ident(),
                                ));
                            }
                        }
                        *tlas_build.tlas.built_index.write() = Some(build_command_index);
                        tlas_build
                            .tlas
                            .dependencies
                            .write()
                            .clone_from(&tlas_build.dependencies)
                    }
                }
                AsAction::UseTlas(tlas) => {
                    let tlas_build_index = tlas.built_index.read();
                    let dependencies = tlas.dependencies.read();

                    if (*tlas_build_index).is_none() {
                        return Err(ValidateAsActionsError::UsedUnbuiltTlas(tlas.error_ident()));
                    }
                    for blas in dependencies.deref() {
                        let blas_build_index = *blas.built_index.read();
                        if blas_build_index.is_none() {
                            return Err(ValidateAsActionsError::UsedUnbuiltBlas(
                                tlas.error_ident(),
                                blas.error_ident(),
                            ));
                        }
                        if blas_build_index.unwrap() > tlas_build_index.unwrap() {
                            return Err(ValidateAsActionsError::BlasNewerThenTlas(
                                blas.error_ident(),
                                tlas.error_ident(),
                            ));
                        }
                        blas.try_raw(snatch_guard)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn set_acceleration_structure_dependencies(&self, snatch_guard: &SnatchGuard) {
        profiling::scope!("CommandEncoder::[submission]::set_acceleration_structure_dependencies");
        let tlas_dependencies_locks: Vec<_> = self
            .as_actions
            .iter()
            .filter_map(|action| {
                if let AsAction::UseTlas(tlas) = action {
                    Some(tlas.dependencies.read())
                } else {
                    None
                }
            })
            .collect();
        let mut tlas_dependencies_lock_iter = tlas_dependencies_locks.iter();
        let mut dependencies = Vec::new();
        for action in &self.as_actions {
            match action {
                AsAction::Build(build) => {
                    for tlas_build in build.tlas_s_built.iter() {
                        for dependency in &tlas_build.dependencies {
                            if let Some(dependency) = dependency.raw(snatch_guard) {
                                dependencies.push(dependency);
                            }
                        }
                    }
                }
                AsAction::UseTlas(_tlas) => {
                    let tlas_dependencies = tlas_dependencies_lock_iter.next().unwrap(); // _tlas.dependencies.read();
                    for dependency in tlas_dependencies.iter() {
                        if let Some(dependency) = dependency.raw(snatch_guard) {
                            dependencies.push(dependency);
                        }
                    }
                }
            }
        }
        if !dependencies.is_empty() {
            unsafe {
                self.encoder
                    .raw
                    .set_acceleration_structure_dependencies(&self.encoder.list, &dependencies);
            }
        }
    }
}

///iterates over the blas iterator, and it's geometry, pushing the buffers into a storage vector (and also some validation).
fn iter_blas<'snatch_guard: 'buffers, 'buffers>(
    blas_iter: impl Iterator<Item = &'buffers OwnedBlasBuildEntry<ArcReferences>>,
    build_command: &mut AsBuild,
    input_barriers: &mut Vec<hal::BufferBarrier<'buffers, dyn hal::DynBuffer>>,
    scratch_buffer_blas_size: &mut u64,
    blas_storage: &mut Vec<BlasStore<'buffers>>,
    state: &mut EncodingState<'snatch_guard, '_>,
) -> Result<(), BuildAccelerationStructureError> {
    for entry in blas_iter {
        let blas = &entry.blas;
        state.tracker.blas_s.insert_single(blas.clone());

        build_command.blas_s_built.push(blas.clone());

        match &entry.geometries {
            ArcBlasGeometries::TriangleGeometries(triangle_geometries) => {
                let mut triangle_entries =
                    Vec::<hal::AccelerationStructureTriangles<dyn hal::DynBuffer>>::new();

                for (i, mesh) in triangle_geometries.iter().enumerate() {
                    let size_desc = match &blas.sizes {
                        wgt::BlasGeometrySizeDescriptors::Triangles { descriptors } => descriptors,
                        _ => {
                            return Err(BuildAccelerationStructureError::BlasGeometryKindMismatch(
                                blas.error_ident(),
                            ));
                        }
                    };
                    if i >= size_desc.len() {
                        return Err(BuildAccelerationStructureError::IncompatibleBlasBuildSizes(
                            blas.error_ident(),
                        ));
                    }
                    let size_desc = &size_desc[i];

                    if size_desc.flags != mesh.size.flags {
                        return Err(BuildAccelerationStructureError::IncompatibleBlasFlags(
                            blas.error_ident(),
                            size_desc.flags,
                            mesh.size.flags,
                        ));
                    }

                    if size_desc.vertex_count < mesh.size.vertex_count {
                        return Err(
                            BuildAccelerationStructureError::IncompatibleBlasVertexCount(
                                blas.error_ident(),
                                size_desc.vertex_count,
                                mesh.size.vertex_count,
                            ),
                        );
                    }

                    if size_desc.vertex_format != mesh.size.vertex_format {
                        return Err(BuildAccelerationStructureError::DifferentBlasVertexFormats(
                            blas.error_ident(),
                            size_desc.vertex_format,
                            mesh.size.vertex_format,
                        ));
                    }

                    if size_desc
                        .vertex_format
                        .min_acceleration_structure_vertex_stride()
                        > mesh.vertex_stride
                    {
                        return Err(BuildAccelerationStructureError::VertexStrideTooSmall(
                            blas.error_ident(),
                            size_desc
                                .vertex_format
                                .min_acceleration_structure_vertex_stride(),
                            mesh.vertex_stride,
                        ));
                    }

                    if mesh.vertex_stride
                        % size_desc
                            .vertex_format
                            .acceleration_structure_stride_alignment()
                        != 0
                    {
                        return Err(BuildAccelerationStructureError::VertexStrideUnaligned(
                            blas.error_ident(),
                            size_desc
                                .vertex_format
                                .acceleration_structure_stride_alignment(),
                            mesh.vertex_stride,
                        ));
                    }

                    match (size_desc.index_count, mesh.size.index_count) {
                        (Some(_), None) | (None, Some(_)) => {
                            return Err(
                                BuildAccelerationStructureError::BlasIndexCountProvidedMismatch(
                                    blas.error_ident(),
                                ),
                            )
                        }
                        (Some(create), Some(build)) if create < build => {
                            return Err(
                                BuildAccelerationStructureError::IncompatibleBlasIndexCount(
                                    blas.error_ident(),
                                    create,
                                    build,
                                ),
                            )
                        }
                        _ => {}
                    }

                    if size_desc.index_format != mesh.size.index_format {
                        return Err(BuildAccelerationStructureError::DifferentBlasIndexFormats(
                            blas.error_ident(),
                            size_desc.index_format,
                            mesh.size.index_format,
                        ));
                    }

                    if size_desc.index_count.is_some() && mesh.index_buffer.is_none() {
                        return Err(BuildAccelerationStructureError::MissingIndexBuffer(
                            blas.error_ident(),
                        ));
                    }
                    let vertex_buffer = mesh.vertex_buffer.clone();
                    let vertex_pending = state.tracker.buffers.set_single(
                        &vertex_buffer,
                        BufferUses::BOTTOM_LEVEL_ACCELERATION_STRUCTURE_INPUT,
                    );
                    let vertex_buffer = {
                        let vertex_raw = mesh.vertex_buffer.as_ref().try_raw(state.snatch_guard)?;
                        let vertex_buffer = &mesh.vertex_buffer;
                        vertex_buffer.check_usage(BufferUsages::BLAS_INPUT)?;

                        if let Some(barrier) = vertex_pending.map(|pending| {
                            pending.into_hal(vertex_buffer.as_ref(), state.snatch_guard)
                        }) {
                            input_barriers.push(barrier);
                        }
                        if u64::from(mesh.size.vertex_count)
                            .checked_add(u64::from(mesh.first_vertex))
                            .and_then(|end_vertex| end_vertex.checked_mul(mesh.vertex_stride))
                            .is_none_or(|end| vertex_buffer.size < end)
                        {
                            return Err(BuildAccelerationStructureError::InsufficientBufferSize {
                                buffer_ident: vertex_buffer.error_ident(),
                                offset: u64::from(mesh.first_vertex)
                                    .saturating_mul(mesh.vertex_stride),
                                region_size: u64::from(mesh.size.vertex_count)
                                    .saturating_mul(mesh.vertex_stride),
                                buffer_size: vertex_buffer.size,
                            });
                        }
                        let vertex_buffer_offset = mesh.first_vertex as u64 * mesh.vertex_stride;
                        state.buffer_memory_init_actions.extend(
                            vertex_buffer.initialization_status.read().create_action(
                                vertex_buffer,
                                vertex_buffer_offset
                                    ..(vertex_buffer_offset
                                        + mesh.size.vertex_count as u64 * mesh.vertex_stride),
                                MemoryInitKind::NeedsInitializedMemory,
                            ),
                        );
                        vertex_raw
                    };
                    let index_buffer = if let Some(ref index_buffer) = mesh.index_buffer {
                        if mesh.first_index.is_none()
                            || mesh.size.index_count.is_none()
                            || mesh.size.index_count.is_none()
                        {
                            return Err(BuildAccelerationStructureError::MissingAssociatedData(
                                index_buffer.error_ident(),
                            ));
                        }
                        let index_pending = state.tracker.buffers.set_single(
                            index_buffer,
                            BufferUses::BOTTOM_LEVEL_ACCELERATION_STRUCTURE_INPUT,
                        );
                        let index_raw = index_buffer.try_raw(state.snatch_guard)?;
                        index_buffer.check_usage(BufferUsages::BLAS_INPUT)?;

                        if let Some(barrier) = index_pending.map(|pending| {
                            pending.into_hal(index_buffer.as_ref(), state.snatch_guard)
                        }) {
                            input_barriers.push(barrier);
                        }
                        let index_stride = mesh.size.index_format.unwrap().byte_size();
                        // `hal::AccelerationStructureTriangleIndices` accepts only `u32` offset
                        let Some(vertex_offset) =
                            mesh.first_index.unwrap().checked_mul(index_stride)
                        else {
                            return Err(BuildAccelerationStructureError::OffsetLimitedTo4GB {
                                buffer_ident: index_buffer.error_ident(),
                                offset: u64::from(mesh.first_index.unwrap())
                                    .saturating_mul(u64::from(index_stride)),
                                count: u64::from(mesh.first_index.unwrap()),
                                stride: u64::from(index_stride),
                            });
                        };
                        let Some(indexes_size) =
                            mesh.size.index_count.unwrap().checked_mul(index_stride)
                        else {
                            return Err(BuildAccelerationStructureError::OffsetLimitedTo4GB {
                                buffer_ident: index_buffer.error_ident(),
                                offset: u64::from(mesh.size.index_count.unwrap())
                                    .saturating_mul(u64::from(index_stride)),
                                count: u64::from(mesh.size.index_count.unwrap()),
                                stride: u64::from(index_stride),
                            });
                        };

                        if mesh.size.index_count.unwrap() % 3 != 0 {
                            return Err(BuildAccelerationStructureError::InvalidIndexCount(
                                index_buffer.error_ident(),
                                mesh.size.index_count.unwrap(),
                            ));
                        }
                        if index_buffer.size < u64::from(vertex_offset)
                            || index_buffer.size - u64::from(vertex_offset)
                                < u64::from(indexes_size)
                        {
                            return Err(BuildAccelerationStructureError::InsufficientBufferSize {
                                buffer_ident: index_buffer.error_ident(),
                                offset: u64::from(vertex_offset),
                                region_size: u64::from(indexes_size),
                                buffer_size: index_buffer.size,
                            });
                        }

                        state.buffer_memory_init_actions.extend(
                            index_buffer.initialization_status.read().create_action(
                                index_buffer,
                                u64::from(vertex_offset)
                                    ..(u64::from(vertex_offset) + u64::from(indexes_size)),
                                MemoryInitKind::NeedsInitializedMemory,
                            ),
                        );
                        Some((index_raw, vertex_offset))
                    } else {
                        None
                    };
                    let transform_buffer = if let Some(ref transform_buffer) = mesh.transform_buffer
                    {
                        if !blas
                            .flags
                            .contains(wgt::AccelerationStructureFlags::USE_TRANSFORM)
                        {
                            return Err(BuildAccelerationStructureError::UseTransformMissing(
                                blas.error_ident(),
                            ));
                        }
                        if mesh.transform_buffer_offset.is_none() {
                            return Err(BuildAccelerationStructureError::MissingAssociatedData(
                                transform_buffer.error_ident(),
                            ));
                        }
                        let transform_pending = state.tracker.buffers.set_single(
                            transform_buffer,
                            BufferUses::BOTTOM_LEVEL_ACCELERATION_STRUCTURE_INPUT,
                        );
                        if mesh.transform_buffer_offset.is_none() {
                            return Err(BuildAccelerationStructureError::MissingAssociatedData(
                                transform_buffer.error_ident(),
                            ));
                        }
                        let transform_raw = transform_buffer.try_raw(state.snatch_guard)?;
                        transform_buffer.check_usage(BufferUsages::BLAS_INPUT)?;

                        if let Some(barrier) = transform_pending.map(|pending| {
                            pending.into_hal(transform_buffer.as_ref(), state.snatch_guard)
                        }) {
                            input_barriers.push(barrier);
                        }

                        // `hal::AccelerationStructureTriangleTransform` accepts only `u32` offset
                        let Ok(offset) = u32::try_from(mesh.transform_buffer_offset.unwrap())
                        else {
                            return Err(BuildAccelerationStructureError::OffsetLimitedTo4GB {
                                buffer_ident: transform_buffer.error_ident(),
                                offset: mesh.transform_buffer_offset.unwrap(),
                                count: mesh.transform_buffer_offset.unwrap(),
                                stride: 1,
                            });
                        };

                        if offset % wgt::TRANSFORM_BUFFER_ALIGNMENT as u32 != 0 {
                            return Err(
                                BuildAccelerationStructureError::UnalignedTransformBufferOffset(
                                    transform_buffer.error_ident(),
                                ),
                            );
                        }
                        if transform_buffer.size < 48
                            || transform_buffer.size - 48 < u64::from(offset)
                        {
                            return Err(BuildAccelerationStructureError::InsufficientBufferSize {
                                buffer_ident: transform_buffer.error_ident(),
                                region_size: 48,
                                offset: u64::from(offset),
                                buffer_size: transform_buffer.size,
                            });
                        }
                        state.buffer_memory_init_actions.extend(
                            transform_buffer.initialization_status.read().create_action(
                                transform_buffer,
                                u64::from(offset)..(u64::from(offset) + 48),
                                MemoryInitKind::NeedsInitializedMemory,
                            ),
                        );
                        Some((transform_raw, offset))
                    } else {
                        if blas
                            .flags
                            .contains(wgt::AccelerationStructureFlags::USE_TRANSFORM)
                        {
                            return Err(BuildAccelerationStructureError::TransformMissing(
                                blas.error_ident(),
                            ));
                        }
                        None
                    };

                    let triangles = hal::AccelerationStructureTriangles {
                        vertex_buffer: Some(vertex_buffer),
                        vertex_format: mesh.size.vertex_format,
                        first_vertex: mesh.first_vertex,
                        vertex_count: mesh.size.vertex_count,
                        vertex_stride: mesh.vertex_stride,
                        indices: index_buffer.map(|(index_raw, vertex_offset)| {
                            hal::AccelerationStructureTriangleIndices::<dyn hal::DynBuffer> {
                                format: mesh.size.index_format.unwrap(),
                                buffer: Some(index_raw),
                                offset: vertex_offset,
                                count: mesh.size.index_count.unwrap(),
                            }
                        }),
                        transform: transform_buffer.map(|(transform_raw, offset)| {
                            hal::AccelerationStructureTriangleTransform {
                                buffer: transform_raw,
                                offset,
                            }
                        }),
                        flags: mesh.size.flags,
                    };
                    triangle_entries.push(triangles);
                }

                {
                    let scratch_buffer_offset = *scratch_buffer_blas_size;
                    *scratch_buffer_blas_size = scratch_buffer_blas_size.saturating_add(align_to(
                        blas.size_info.build_scratch_size,
                        u64::from(state.device.alignments.ray_tracing_scratch_buffer_alignment),
                    ));

                    blas_storage.push(BlasStore {
                        blas: blas.clone(),
                        entries: hal::AccelerationStructureEntries::Triangles(triangle_entries),
                        scratch_buffer_offset,
                    });
                }
            }
            ArcBlasGeometries::AabbGeometries(aabb_geometries) => {
                let mut aabb_entries =
                    Vec::<hal::AccelerationStructureAABBs<dyn hal::DynBuffer>>::new();

                for (i, aabb) in aabb_geometries.iter().enumerate() {
                    let size_desc = match &blas.sizes {
                        wgt::BlasGeometrySizeDescriptors::AABBs { descriptors } => descriptors,
                        _ => {
                            return Err(BuildAccelerationStructureError::BlasGeometryKindMismatch(
                                blas.error_ident(),
                            ));
                        }
                    };
                    if i >= size_desc.len() {
                        return Err(BuildAccelerationStructureError::IncompatibleBlasBuildSizes(
                            blas.error_ident(),
                        ));
                    }
                    let size_desc = &size_desc[i];

                    if size_desc.flags != aabb.size.flags {
                        return Err(BuildAccelerationStructureError::IncompatibleBlasFlags(
                            blas.error_ident(),
                            size_desc.flags,
                            aabb.size.flags,
                        ));
                    }

                    if size_desc.primitive_count < aabb.size.primitive_count {
                        return Err(
                            BuildAccelerationStructureError::IncompatibleBlasAabbPrimitiveCount(
                                blas.error_ident(),
                                size_desc.primitive_count,
                                aabb.size.primitive_count,
                            ),
                        );
                    }

                    if aabb.primitive_offset % 8 != 0 {
                        return Err(
                            BuildAccelerationStructureError::UnalignedAabbPrimitiveOffset(
                                blas.error_ident(),
                            ),
                        );
                    }

                    if aabb.stride < wgt::AABB_GEOMETRY_MIN_STRIDE || aabb.stride % 8 != 0 {
                        return Err(BuildAccelerationStructureError::InvalidAabbStride(
                            blas.error_ident(),
                            aabb.stride,
                        ));
                    }

                    let aabb_buffer = aabb.aabb_buffer.clone();
                    let aabb_pending = state.tracker.buffers.set_single(
                        &aabb_buffer,
                        BufferUses::BOTTOM_LEVEL_ACCELERATION_STRUCTURE_INPUT,
                    );
                    let aabb_raw = {
                        let aabb_raw = aabb.aabb_buffer.as_ref().try_raw(state.snatch_guard)?;
                        let aabb_buffer = &aabb.aabb_buffer;
                        aabb_buffer.check_usage(BufferUsages::BLAS_INPUT)?;

                        if let Some(barrier) = aabb_pending.map(|pending| {
                            pending.into_hal(aabb_buffer.as_ref(), state.snatch_guard)
                        }) {
                            input_barriers.push(barrier);
                        }

                        // `hal::AccelerationStructureAABBs` accepts only `u32` offset
                        let Some(aabb_size) = u32::try_from(aabb.stride)
                            .ok()
                            .and_then(|stride| aabb.size.primitive_count.checked_mul(stride))
                        else {
                            return Err(BuildAccelerationStructureError::OffsetLimitedTo4GB {
                                buffer_ident: aabb_buffer.error_ident(),
                                offset: u64::from(aabb.size.primitive_count)
                                    .saturating_mul(aabb.stride),
                                count: u64::from(aabb.size.primitive_count),
                                stride: aabb.stride,
                            });
                        };

                        if aabb_buffer.size < u64::from(aabb.primitive_offset)
                            || aabb_buffer.size - u64::from(aabb.primitive_offset)
                                < u64::from(aabb_size)
                        {
                            return Err(BuildAccelerationStructureError::InsufficientBufferSize {
                                buffer_ident: aabb_buffer.error_ident(),
                                region_size: u64::from(aabb_size),
                                offset: u64::from(aabb.primitive_offset),
                                buffer_size: aabb_buffer.size,
                            });
                        }

                        state.buffer_memory_init_actions.extend(
                            aabb_buffer.initialization_status.read().create_action(
                                aabb_buffer,
                                u64::from(aabb.primitive_offset)
                                    ..u64::from(aabb.primitive_offset) + u64::from(aabb_size),
                                MemoryInitKind::NeedsInitializedMemory,
                            ),
                        );
                        aabb_raw
                    };

                    aabb_entries.push(hal::AccelerationStructureAABBs {
                        buffer: Some(aabb_raw),
                        offset: aabb.primitive_offset,
                        count: aabb.size.primitive_count,
                        stride: aabb.stride,
                        flags: aabb.size.flags,
                    });
                }

                {
                    let scratch_buffer_offset = *scratch_buffer_blas_size;
                    *scratch_buffer_blas_size = scratch_buffer_blas_size.saturating_add(align_to(
                        blas.size_info.build_scratch_size,
                        u64::from(state.device.alignments.ray_tracing_scratch_buffer_alignment),
                    ));

                    blas_storage.push(BlasStore {
                        blas: blas.clone(),
                        entries: hal::AccelerationStructureEntries::AABBs(aabb_entries),
                        scratch_buffer_offset,
                    });
                }
            }
        }
    }
    Ok(())
}

fn map_blas<'a>(
    storage: &'a BlasStore<'_>,
    scratch_buffer: &'a dyn hal::DynBuffer,
    snatch_guard: &'a SnatchGuard,
    blases_compactable: &mut Vec<(
        &'a dyn hal::DynBuffer,
        &'a dyn hal::DynAccelerationStructure,
    )>,
) -> Result<
    hal::BuildAccelerationStructureDescriptor<
        'a,
        dyn hal::DynBuffer,
        dyn hal::DynAccelerationStructure,
    >,
    BuildAccelerationStructureError,
> {
    let BlasStore {
        blas,
        entries,
        scratch_buffer_offset,
    } = storage;
    if blas.update_mode == wgt::AccelerationStructureUpdateMode::PreferUpdate {
        log::debug!("only rebuild implemented")
    }
    let raw = blas.try_raw(snatch_guard)?;

    let state_lock = blas.compacted_state.lock();
    if let BlasCompactState::Compacted = *state_lock {
        return Err(BuildAccelerationStructureError::CompactedBlas(
            blas.error_ident(),
        ));
    }

    if blas
        .flags
        .contains(wgpu_types::AccelerationStructureFlags::ALLOW_COMPACTION)
    {
        blases_compactable.push((blas.compaction_buffer.as_ref().unwrap().as_ref(), raw));
    }
    Ok(hal::BuildAccelerationStructureDescriptor {
        entries,
        mode: hal::AccelerationStructureBuildMode::Build,
        flags: blas.flags,
        source_acceleration_structure: None,
        destination_acceleration_structure: raw,
        scratch_buffer,
        scratch_buffer_offset: *scratch_buffer_offset,
    })
}

fn build_blas<'a>(
    cmd_buf_raw: &mut dyn hal::DynCommandEncoder,
    blas_present: bool,
    tlas_present: bool,
    input_barriers: Vec<hal::BufferBarrier<dyn hal::DynBuffer>>,
    blas_descriptors: &[hal::BuildAccelerationStructureDescriptor<
        'a,
        dyn hal::DynBuffer,
        dyn hal::DynAccelerationStructure,
    >],
    scratch_buffer_barrier: hal::BufferBarrier<dyn hal::DynBuffer>,
    blas_s_for_compaction: Vec<(
        &'a dyn hal::DynBuffer,
        &'a dyn hal::DynAccelerationStructure,
    )>,
) {
    unsafe {
        cmd_buf_raw.transition_buffers(&input_barriers);
    }

    if blas_present {
        unsafe {
            cmd_buf_raw.place_acceleration_structure_barrier(hal::AccelerationStructureBarrier {
                usage: hal::StateTransition {
                    from: hal::AccelerationStructureUses::BUILD_INPUT,
                    to: hal::AccelerationStructureUses::BUILD_OUTPUT,
                },
            });

            cmd_buf_raw.build_acceleration_structures(blas_descriptors);
        }
    }

    if blas_present && tlas_present {
        unsafe {
            cmd_buf_raw.transition_buffers(&[scratch_buffer_barrier]);
        }
    }

    let mut source_usage = hal::AccelerationStructureUses::empty();
    let mut destination_usage = hal::AccelerationStructureUses::empty();
    for &(buf, blas) in blas_s_for_compaction.iter() {
        unsafe {
            cmd_buf_raw.transition_buffers(&[hal::BufferBarrier {
                buffer: buf,
                usage: hal::StateTransition {
                    from: BufferUses::ACCELERATION_STRUCTURE_QUERY,
                    to: BufferUses::ACCELERATION_STRUCTURE_QUERY,
                },
            }])
        }
        unsafe { cmd_buf_raw.read_acceleration_structure_compact_size(blas, buf) }
        destination_usage |= hal::AccelerationStructureUses::COPY_SRC;
    }

    if blas_present {
        source_usage |= hal::AccelerationStructureUses::BUILD_OUTPUT;
        destination_usage |= hal::AccelerationStructureUses::BUILD_INPUT
    }
    if tlas_present {
        source_usage |= hal::AccelerationStructureUses::SHADER_INPUT;
        destination_usage |= hal::AccelerationStructureUses::BUILD_OUTPUT;
    }
    unsafe {
        cmd_buf_raw.place_acceleration_structure_barrier(hal::AccelerationStructureBarrier {
            usage: hal::StateTransition {
                from: source_usage,
                to: destination_usage,
            },
        });
    }
}
