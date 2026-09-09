/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#![allow(unsafe_code)]

//! A replacement for `Box<[T; N]>` that cbindgen can understand.

use malloc_size_of::{MallocShallowSizeOf, MallocSizeOf, MallocSizeOfOps};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::{fmt, mem};
use to_shmem::{SharedMemoryBuilder, ToShmem};

/// A struct that basically replaces a `Box<[T; N]>`, but which cbindgen can
/// understand.
///
/// Every `OwnedArray` is allocation-backed with `N` fully-constructed
/// elements; there is no empty or moved-from state.
///
/// We could rely on the struct layout of `Box<[T; N]>`. Per the `Box`
/// documentation, `Box<T>` has the same ABI as `*mut T`, and since `[T; N]` is
/// `Sized`, that's a thin pointer (unlike `Box<[T]>`, which is fat):
///
///   https://doc.rust-lang.org/std/boxed/index.html#memory-layout
///
/// But handling `Box` with cbindgen both in structs and argument positions
/// more generally is a bit tricky.
///
/// This is useful when generated cbindgen code has circular references that
/// can only be broken by indirection through boxed objects. When a type needs
/// two, three, or a similar small number of such objects of the same kind,
/// `OwnedArray<T, N>` is more ergonomic and efficient than holding `N`
/// separate boxed values: it uses a single allocation and a single thin
/// pointer instead of `N` of each.
///
/// Compared to `OwnedSlice<T>`, `OwnedArray<T, N>` does not store the length
/// at runtime, `N` is part of the type, so the struct is a single
/// pointer-word instead of two. For types used as enum variant payloads, this
/// reduces the enclosing enum's footprint. The length can also be statically
/// checked, wrong-length values are unrepresentable, so consumers don't need
/// need runtime length assertions.
///
/// cbindgen:derive-eq=false
/// cbindgen:derive-neq=false
#[repr(C)]
pub struct OwnedArray<T, const N: usize> {
    ptr: NonNull<T>,
    _phantom: PhantomData<T>,
}

impl<T, const N: usize> Drop for OwnedArray<T, N> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.ptr.as_ptr() as *mut [T; N]));
        }
    }
}

unsafe impl<T: Send, const N: usize> Send for OwnedArray<T, N> {}
unsafe impl<T: Sync, const N: usize> Sync for OwnedArray<T, N> {}

impl<T: Clone, const N: usize> Clone for OwnedArray<T, N> {
    #[inline]
    fn clone(&self) -> Self {
        std::array::from_fn(|i| self[i].clone()).into()
    }
}

impl<T: fmt::Debug, const N: usize> fmt::Debug for OwnedArray<T, N> {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        self.deref().fmt(formatter)
    }
}

impl<T: PartialEq, const N: usize> PartialEq for OwnedArray<T, N> {
    fn eq(&self, other: &Self) -> bool {
        self.deref().eq(other.deref())
    }
}

impl<T: Eq, const N: usize> Eq for OwnedArray<T, N> {}

impl<T, const N: usize> OwnedArray<T, N> {
    /// Convert the OwnedArray into a boxed array.
    #[inline]
    pub fn into_box(self) -> Box<[T; N]> {
        let b = unsafe { Box::from_raw(self.ptr.as_ptr() as *mut [T; N]) };
        mem::forget(self);
        b
    }

    /// Convert the OwnedArray into an array.
    #[inline]
    pub fn into_array(self) -> [T; N] {
        *self.into_box()
    }
}

impl<T, const N: usize> Deref for OwnedArray<T, N> {
    type Target = [T; N];

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.ptr.as_ptr() as *const [T; N]) }
    }
}

impl<T, const N: usize> DerefMut for OwnedArray<T, N> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *(self.ptr.as_ptr() as *mut [T; N]) }
    }
}

impl<T, const N: usize> From<[T; N]> for OwnedArray<T, N> {
    #[inline]
    fn from(values: [T; N]) -> Self {
        Box::new(values).into()
    }
}

impl<T, const N: usize> From<Box<[T; N]>> for OwnedArray<T, N> {
    #[inline]
    fn from(b: Box<[T; N]>) -> Self {
        let ptr = unsafe { NonNull::new_unchecked(Box::into_raw(b) as *mut T) };
        Self {
            ptr,
            _phantom: PhantomData,
        }
    }
}

impl<T: Sized, const N: usize> MallocShallowSizeOf for OwnedArray<T, N> {
    fn shallow_size_of(&self, ops: &mut MallocSizeOfOps) -> usize {
        unsafe { ops.malloc_size_of(self.ptr.as_ptr()) }
    }
}

impl<T: MallocSizeOf + Sized, const N: usize> MallocSizeOf for OwnedArray<T, N> {
    fn size_of(&self, ops: &mut MallocSizeOfOps) -> usize {
        self.shallow_size_of(ops) + (**self).size_of(ops)
    }
}

impl<T: ToShmem + Sized, const N: usize> ToShmem for OwnedArray<T, N> {
    fn to_shmem(&self, builder: &mut SharedMemoryBuilder) -> to_shmem::Result<Self> {
        unsafe {
            let dest = to_shmem::to_shmem_slice(self.iter(), builder)?;
            Ok(mem::ManuallyDrop::new(Self::from(Box::from_raw(
                dest as *mut [T; N],
            ))))
        }
    }
}
