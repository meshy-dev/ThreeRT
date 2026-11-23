use crate::{api::blas::TlasInstance, dispatch};
use crate::{BindingResource, Label};
use alloc::vec::Vec;
#[cfg(wgpu_core)]
use core::ops::Deref;
use core::ops::{Index, IndexMut, Range};
use core::sync::atomic::{AtomicU32, Ordering};
use wgt::WasmNotSendSync;

/// Descriptor to create top level acceleration structures.
pub type CreateTlasDescriptor<'a> = wgt::CreateTlasDescriptor<Label<'a>>;
static_assertions::assert_impl_all!(CreateTlasDescriptor<'_>: Send, Sync);

#[derive(Debug)]
/// Top Level Acceleration Structure (TLAS).
///
/// A TLAS contains a series of [TLAS instances], which are a reference to
/// a BLAS and a transformation matrix placing the geometry in the world.
///
/// A TLAS also contains an extra set of TLAS instances in a device readable form, you cant interact
/// directly with these, instead you have to build the TLAS with [TLAS instances].
///
/// [TLAS instances]: TlasInstance
pub struct Tlas {
    pub(crate) inner: dispatch::DispatchTlas,
    pub(crate) instances: Vec<Option<TlasInstance>>,
    pub(crate) lowest_unmodified: u32,
    /// ThreeRT patch: the instances changed since the last build, `dirty_lo..dirty_hi` (empty when lo >= hi). A build
    /// hands only those to wgpu-core, which keeps the rest where the last build put them (third_party/wgpu-core/PATCH.md).
    pub(crate) dirty_lo: AtomicU32,
    pub(crate) dirty_hi: AtomicU32,
}

impl Clone for Tlas {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            instances: self.instances.clone(),
            lowest_unmodified: self.lowest_unmodified,
            dirty_lo: AtomicU32::new(self.dirty_lo.load(Ordering::Relaxed)),
            dirty_hi: AtomicU32::new(self.dirty_hi.load(Ordering::Relaxed)),
        }
    }
}
static_assertions::assert_impl_all!(Tlas: WasmNotSendSync);

crate::cmp::impl_eq_ord_hash_proxy!(Tlas => .inner);

impl Tlas {
    /// Get the [`wgpu_hal`] acceleration structure from this `Tlas`.
    ///
    /// Find the Api struct corresponding to the active backend in [`wgpu_hal::api`],
    /// and pass that struct to the to the `A` type parameter.
    ///
    /// Returns a guard that dereferences to the type of the hal backend
    /// which implements [`A::AccelerationStructure`].
    ///
    /// # Types
    ///
    /// The returned type depends on the backend:
    ///
    #[doc = crate::macros::hal_type_vulkan!("AccelerationStructure")]
    #[doc = crate::macros::hal_type_metal!("AccelerationStructure")]
    #[doc = crate::macros::hal_type_dx12!("AccelerationStructure")]
    #[doc = crate::macros::hal_type_gles!("AccelerationStructure")]
    ///
    /// # Deadlocks
    ///
    /// - The returned guard holds a read-lock on a device-local "destruction"
    ///   lock, which will cause all calls to `destroy` to block until the
    ///   guard is released.
    ///
    /// # Errors
    ///
    /// This method will return None if:
    /// - The acceleration structure is not from the backend specified by `A`.
    /// - The acceleration structure is from the `webgpu` or `custom` backend.
    ///
    /// # Safety
    ///
    /// - The returned resource must not be destroyed unless the guard
    ///   is the last reference to it and it is not in use by the GPU.
    ///   The guard and handle may be dropped at any time however.
    /// - All the safety requirements of wgpu-hal must be upheld.
    ///
    /// [`A::AccelerationStructure`]: hal::Api::AccelerationStructure
    #[cfg(wgpu_core)]
    pub unsafe fn as_hal<A: hal::Api>(
        &mut self,
    ) -> Option<impl Deref<Target = A::AccelerationStructure>> {
        let tlas = self.inner.as_core_opt()?;
        unsafe { tlas.context.tlas_as_hal::<A>(tlas) }
    }

    /// Returns custom implementation of Tlas (if custom backend and is internally T)
    #[cfg(custom)]
    pub fn as_custom<T: crate::custom::TlasInterface>(&self) -> Option<&T> {
        self.inner.as_custom()
    }
    /// Returns the index of the first instance that has not been modified since the last build.
    /// Custom backends use this to perform partial TLAS updates.
    #[cfg(custom)]
    pub fn lowest_unmodified(&self) -> u32 {
        self.lowest_unmodified
    }

    /// Get a reference to all instances.
    pub fn get(&self) -> &[Option<TlasInstance>] {
        &self.instances
    }

    /// ThreeRT patch: marks `range` changed (the next build hands it to wgpu-core).
    fn touch(&self, range: Range<usize>) {
        self.dirty_lo.fetch_min(range.start as u32, Ordering::Relaxed);
        self.dirty_hi.fetch_max(range.end as u32, Ordering::Relaxed);
    }

    /// ThreeRT patch: the changed range, taken (a build's).
    #[cfg(wgpu_core)]
    pub(crate) fn take_dirty(&self) -> Range<u32> {
        let lo = self.dirty_lo.swap(u32::MAX, Ordering::Relaxed);
        let hi = self.dirty_hi.swap(0, Ordering::Relaxed).min(self.instances.len() as u32);
        if lo >= hi { 0..0 } else { lo..hi }
    }

    /// Get a mutable slice to a range of instances.
    /// Returns None if the range is out of bounds.
    /// All elements from the lowest accessed index up are marked as modified.
    // this recommendation is not useful yet, but is likely to be when ability to update arrives or possible optimisations for building get implemented.
    /// For best performance it is recommended to prefer access to low elements and modify higher elements as little as possible.
    /// This can be done by ordering instances from the most to the least used. It is recommended
    /// to use [`Self::index_mut`] unless the option if out of bounds is required
    pub fn get_mut_slice(&mut self, range: Range<usize>) -> Option<&mut [Option<TlasInstance>]> {
        if range.end > self.instances.len() {
            return None;
        }
        if range.end as u32 > self.lowest_unmodified {
            self.lowest_unmodified = range.end as u32;
        }
        self.touch(range.clone());
        Some(&mut self.instances[range])
    }

    /// Get a single mutable reference to an instance.
    /// Returns None if the range is out of bounds.
    /// All elements from the lowest accessed index up are marked as modified.
    // this recommendation is not useful yet, but is likely to be when ability to update arrives or possible optimisations for building get implemented.
    /// For best performance it is recommended to prefer access to low elements and modify higher elements as little as possible.
    /// This can be done by ordering instances from the most to the least used. It is recommended
    /// to use [`Self::index_mut`] unless the option if out of bounds is required
    pub fn get_mut_single(&mut self, index: usize) -> Option<&mut Option<TlasInstance>> {
        if index >= self.instances.len() {
            return None;
        }
        if index as u32 + 1 > self.lowest_unmodified {
            self.lowest_unmodified = index as u32 + 1;
        }
        self.touch(index..index + 1);
        Some(&mut self.instances[index])
    }

    /// Get the binding resource for the underling acceleration structure, to be used when creating a [`BindGroup`]
    ///
    /// [`BindGroup`]: super::BindGroup
    pub fn as_binding(&self) -> BindingResource<'_> {
        BindingResource::AccelerationStructure(self)
    }
}

impl Index<usize> for Tlas {
    type Output = Option<TlasInstance>;

    fn index(&self, index: usize) -> &Self::Output {
        self.instances.index(index)
    }
}

impl Index<Range<usize>> for Tlas {
    type Output = [Option<TlasInstance>];

    fn index(&self, index: Range<usize>) -> &Self::Output {
        self.instances.index(index)
    }
}

impl IndexMut<usize> for Tlas {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.touch(index..index + 1);
        let idx = self.instances.index_mut(index);
        if index as u32 + 1 > self.lowest_unmodified {
            self.lowest_unmodified = index as u32 + 1;
        }
        idx
    }
}

impl IndexMut<Range<usize>> for Tlas {
    fn index_mut(&mut self, index: Range<usize>) -> &mut Self::Output {
        self.touch(index.clone());
        let idx = self.instances.index_mut(index.clone());
        if index.end > self.lowest_unmodified as usize {
            self.lowest_unmodified = index.end as u32;
        }
        idx
    }
}
