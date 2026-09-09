/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Generic helper to support a tagged numeric value of 4 bytes and a boxed pointer in the same
//! pointer value in 64-bit builds.
//!
//! The over-all design is a tagged pointer, with the low bit of the pointer being non-zero if it is
//! a non-boxed value. We need to pass the numeric type and tag as separate parameters to make sure
//! that they pack along the tag that we use internally in InlineVariant.

use malloc_size_of::{MallocSizeOf, MallocSizeOfOps};
use std::{fmt, marker, mem};
use to_shmem::{SharedMemoryBuilder, ToShmem};

// NOTE(emilio): cbindgen only understands the #[cfg] on the top level definition.
#[doc(hidden)]
#[repr(C)]
#[cfg(target_pointer_width = "32")]
pub struct BoxedVariant<B> {
    tag: u8,
    ptr: *mut B,
    _phantom: marker::PhantomData<B>,
}

#[doc(hidden)]
#[repr(C)]
#[cfg(target_pointer_width = "64")]
pub struct BoxedVariant<B> {
    ptr: usize, // In little-endian byte order
    _phantom: marker::PhantomData<B>,
}

impl<B> Copy for BoxedVariant<B> {}
impl<B> Clone for BoxedVariant<B> {
    fn clone(&self) -> Self {
        *self
    }
}

unsafe impl<B: Send> Send for BoxedVariant<B> {}
unsafe impl<B: Sync> Sync for BoxedVariant<B> {}

#[doc(hidden)]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct TagVariant {
    tag: u8,
}

#[doc(hidden)]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct InlineVariant<T, N> {
    tag: u8,
    numeric_tag: T,
    value: N,
}

#[doc(hidden)]
#[repr(C)]
pub union NumericUnionImpl<T: Copy, N: Copy, B> {
    inl: InlineVariant<T, N>,
    boxed: BoxedVariant<B>,
    tag: TagVariant,
}

/// cbindgen:derive-eq=false
/// cbindgen:derive-neq=false
#[repr(C)]
pub struct NumericUnion<T: Copy, N: Copy, B>(NumericUnionImpl<T, N, B>);

#[doc(hidden)] // Need to be public so that cbindgen generates it.
pub const NUMERIC_UNION_TAG_INLINE: u8 = 0b1;

impl<T: Copy, N: Copy, B> NumericUnion<T, N, B> {
    /// Whether we hold an inline value.
    pub fn is_inline(&self) -> bool {
        unsafe { (self.0.tag.tag & NUMERIC_UNION_TAG_INLINE) != 0 }
    }

    /// Whether we hold a boxed value.
    pub fn is_boxed(&self) -> bool {
        !self.is_inline()
    }

    #[inline]
    unsafe fn boxed_ptr(&self) -> *mut B {
        debug_assert!(self.is_boxed());
        #[cfg(not(all(target_endian = "big", target_pointer_width = "64")))]
        {
            self.0.boxed.ptr as *mut _
        }
        #[cfg(all(target_endian = "big", target_pointer_width = "64"))]
        {
            self.0.boxed.ptr.swap_bytes() as *mut _
        }
    }

    /// Returns the unpacked value, mutably.
    pub fn unpack_mut(&mut self) -> UnpackedMut<'_, T, N, B> {
        unsafe {
            if self.is_boxed() {
                UnpackedMut::Boxed(&mut *self.boxed_ptr())
            } else {
                UnpackedMut::Inline(&mut self.0.inl.numeric_tag, &mut self.0.inl.value)
            }
        }
    }

    /// Returns the unpacked value.
    pub fn unpack(&self) -> Unpacked<'_, T, N, B> {
        unsafe {
            if self.is_boxed() {
                Unpacked::Boxed(&*self.boxed_ptr())
            } else {
                Unpacked::Inline(self.0.inl.numeric_tag, self.0.inl.value)
            }
        }
    }

    /// Returns the extracted value.
    pub fn extract(self) -> Extracted<T, N, B> {
        let extracted = unsafe {
            if self.is_boxed() {
                Extracted::Boxed(Box::from_raw(self.boxed_ptr()))
            } else {
                Extracted::Inline(self.0.inl.numeric_tag, self.0.inl.value)
            }
        };
        mem::forget(self);
        extracted
    }

    /// Constructs an inline value.
    pub fn inline(numeric_tag: T, value: N) -> Self {
        Self(NumericUnionImpl {
            inl: InlineVariant {
                tag: NUMERIC_UNION_TAG_INLINE,
                numeric_tag,
                value,
            },
        })
    }

    /// Constructs a boxed value.
    pub fn boxed(v: Box<B>) -> Self {
        let ptr = Box::into_raw(v);

        #[cfg(target_pointer_width = "32")]
        let boxed = BoxedVariant {
            tag: 0,
            ptr,
            _phantom: marker::PhantomData,
        };

        #[cfg(target_pointer_width = "64")]
        let boxed = BoxedVariant {
            #[cfg(target_endian = "little")]
            ptr: ptr as usize,
            #[cfg(target_endian = "big")]
            ptr: (ptr as usize).swap_bytes(),
            _phantom: marker::PhantomData,
        };

        let union = Self(NumericUnionImpl { boxed });
        debug_assert!(union.is_boxed());
        union
    }
}

impl<T: Copy, N: Copy, B> Drop for NumericUnion<T, N, B> {
    fn drop(&mut self) {
        if self.is_boxed() {
            let _ = unsafe { Box::from_raw(self.boxed_ptr()) };
        }
    }
}

impl<T: Copy, N: Copy, B: Clone> Clone for NumericUnion<T, N, B> {
    fn clone(&self) -> Self {
        match self.unpack() {
            Unpacked::Inline(t, n) => Self::inline(t, n),
            Unpacked::Boxed(b) => Self::boxed(Box::new(b.clone())),
        }
    }
}

impl<T: Copy, N: Copy, B> MallocSizeOf for NumericUnion<T, N, B> {
    fn size_of(&self, ops: &mut MallocSizeOfOps) -> usize {
        match self.unpack() {
            Unpacked::Boxed(c) => unsafe { ops.malloc_size_of(c) },
            Unpacked::Inline(..) => 0,
        }
    }
}

impl<T: Copy + ToShmem, N: Copy + ToShmem, B: ToShmem> ToShmem for NumericUnion<T, N, B> {
    fn to_shmem(&self, builder: &mut SharedMemoryBuilder) -> to_shmem::Result<Self> {
        unsafe {
            Ok(mem::ManuallyDrop::new(if self.is_inline() {
                let inl = self.0.inl;
                Self(NumericUnionImpl { inl })
            } else {
                let b = mem::ManuallyDrop::new(Box::from_raw(self.boxed_ptr()));
                let b = (*b).to_shmem(builder)?;
                Self::boxed(mem::ManuallyDrop::into_inner(b))
            }))
        }
    }
}

impl<T: Copy + fmt::Debug, N: Copy + fmt::Debug, B: fmt::Debug> fmt::Debug
    for NumericUnion<T, N, B>
{
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        self.unpack().fmt(formatter)
    }
}

impl<T: Copy + PartialEq, N: Copy + PartialEq, B: PartialEq> PartialEq for NumericUnion<T, N, B> {
    fn eq(&self, other: &Self) -> bool {
        self.unpack() == other.unpack()
    }
}

/// Returns the data in a safe way.
#[derive(Clone, Debug, PartialEq)]
pub enum Unpacked<'a, T, N, B> {
    /// A boxed value.
    Boxed(&'a B),
    /// An inline value
    Inline(T, N),
}

/// Returns the extracted data in a safe way.
#[derive(Clone, Debug, PartialEq)]
pub enum Extracted<T, N, B> {
    /// A boxed value.
    Boxed(Box<B>),
    /// An inline value
    Inline(T, N),
}

/// As above, but mutable.
pub enum UnpackedMut<'a, T, N, B> {
    /// A boxed value
    Boxed(&'a mut B),
    /// An inline value
    Inline(&'a mut T, &'a mut N),
}
