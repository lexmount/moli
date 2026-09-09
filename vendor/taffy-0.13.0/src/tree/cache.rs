//! A cache for storing the results of layout computation

#![allow(clippy::unusual_byte_groupings)]

use crate::geometry::{LogicalSize, Size, WritingMode};
use crate::style::AvailableSpace;
use crate::tree::{IntrinsicSizeResult, LayoutInput, LayoutOutput, RunMode, SizingMode, SizingPurpose};
use crate::AutoSizeBehavior;
use crate::RequestedAxis;

/// The number of cache entries for each node in the tree
const CACHE_SIZE: usize = 9;
/// Number of additional entries retained for measurements whose intrinsic
/// inline size depends on a definite block constraint. Grid sizing commonly
/// probes several such constraints in one pass.
const BLOCK_CONSTRAINT_CACHE_SIZE: usize = 8;

// Manually written-out results of float to u32 bit casts because
// `f32::to_bits` is not yet const at our MSRV.

/// `f32::INFINITY` as a u32
const INFINITY_BITS: u32 = 0b_0_11111111_00000000000000000000000_u32;
/// `f32::NEG_INFINITY` as a u32
const NEG_INFINITY_BITS: u32 = 0b_1_11111111_00000000000000000000000_u32;

// The `CacheKey` encodes two f32s as a u64. We know that the f32s will always be
// non-negative, so we pack two extra bits encoding the `RequestedAxis` into the
// sign bits of the f32s. These constants help to encode and decode those bits.

/// The sign bit of the first f32
const SIGN_BIT_1: u64 = 1u64 << 63;
/// The sign bit of the second f32
const SIGN_BIT_2: u64 = 1u64 << 31;
/// Mask of both sign bits (used to compute NON_SIGN_BITS_MASK)
const BOTH_SIGN_BITS_MASK: u64 = SIGN_BIT_1 | SIGN_BIT_2;
/// Mask of excluding the sign bits (used when setting/getting the size excluding the packed bits)
const NON_SIGN_BITS_MASK: u64 = !BOTH_SIGN_BITS_MASK;

/// Bits containing the inline-axis parent size. The requested-axis bits are
/// retained separately when matching measurement entries.
const INLINE_PARENT_SIZE_MASK: u64 = (u32::MAX as u64) << 32;

/// Pack `Option<f32>` into `u32`
#[inline(always)]
fn option_cache_key(input: Option<f32>) -> u32 {
    match input {
        Some(value) => value.to_bits(),
        None => INFINITY_BITS,
    }
}

/// Pack a logical optional size into a cache key with inline-size first.
#[inline(always)]
fn logical_size_option_cache_key(input: LogicalSize<Option<f32>>) -> u64 {
    (option_cache_key(input.inline_size) as u64) << 32 | option_cache_key(input.block_size) as u64
}

/// Pack `AvailableSpace` into `u32`
#[inline(always)]
fn available_space_cache_key(input: AvailableSpace) -> u32 {
    match input {
        AvailableSpace::Definite(value) => (-value).to_bits(),
        AvailableSpace::MinContent => NEG_INFINITY_BITS,
        AvailableSpace::MaxContent => INFINITY_BITS,
    }
}

/// Pack `Size<AvailableSpace>` into `u64`
#[inline(always)]
#[allow(dead_code)]
fn size_available_space_cache_key(input: Size<AvailableSpace>) -> u64 {
    (available_space_cache_key(input.width) as u64) << 32 | available_space_cache_key(input.height) as u64
}

/// Encodes combination of a `known_dimension` (Option<f32>) and `AvailableSpace` in
/// a single dimension into a cache key in a single dimension.
#[inline(always)]
fn mixed_cache_key(kd: Option<f32>, avs: AvailableSpace) -> u32 {
    kd.map(|kd| kd.to_bits()).unwrap_or_else(|| available_space_cache_key(avs))
}

/// Encodes combination of a `known_dimension` (Option<f32>) and `AvailableSpace` in
/// two dimensions into a cache key in a single dimension.
#[inline(always)]
fn size_mixed_cache_key(kd: Size<Option<f32>>, avs: Size<AvailableSpace>) -> u64 {
    (mixed_cache_key(kd.width, avs.width) as u64) << 32 | mixed_cache_key(kd.height, avs.height) as u64
}

/// Space-optimised cache key that packs bits into as small a size as possible
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
struct CacheKey {
    /// The initial cached size of the node itself
    kd_available_space: u64,
    /// The containing block size in its own logical axes.
    logical_parent_size: u64,
    /// Writing mode that owns the containing block size.
    parent_writing_mode: WritingMode,
    /// Whether inherent size styles participate in this computation.
    sizing_mode: SizingMode,
    /// Whether this result is final layout or an intrinsic contribution.
    sizing_purpose: SizingPurpose,
    /// How an authored logical block-size auto resolves in this space.
    block_auto_behavior: AutoSizeBehavior,
}

impl CacheKey {
    #[inline(always)]
    /// Return the inline parent size together with the requested-axis bits.
    fn inline_parent_size_and_axis(&self) -> u64 {
        self.logical_parent_size & (INLINE_PARENT_SIZE_MASK | BOTH_SIGN_BITS_MASK)
    }
}

impl From<&LayoutInput> for CacheKey {
    fn from(input: &LayoutInput) -> Self {
        // Pack axis enum into spare bits in the known_dimensions and available_space values
        let extra_bits = match input.axis {
            RequestedAxis::Horizontal => SIGN_BIT_1,
            RequestedAxis::Vertical => SIGN_BIT_2,
            RequestedAxis::Both => SIGN_BIT_1 | SIGN_BIT_2,
        };

        Self {
            kd_available_space: size_mixed_cache_key(input.known_dimensions, input.available_space),
            logical_parent_size: (logical_size_option_cache_key(
                input.parent_writing_mode.to_logical(input.parent_size),
            ) & NON_SIGN_BITS_MASK)
                | extra_bits,
            parent_writing_mode: input.parent_writing_mode,
            sizing_mode: input.sizing_mode,
            sizing_purpose: input.sizing_purpose,
            block_auto_behavior: input.block_auto_behavior,
        }
    }
}

/// Cached intermediate layout results
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub(crate) struct CacheEntry<T> {
    /// The key for the cache entry
    key: CacheKey,
    /// The cached size and baselines of the item
    content: T,
}

/// The subset of a measurement result retained by the size cache.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
struct CachedMeasurement {
    /// Measured outer size.
    size: Size<f32>,
    /// Whether the result must be keyed by parent block-size.
    depends_on_block_constraints: bool,
    /// Whether this probe obtained its inline contribution by applying the
    /// preferred aspect ratio.
    applied_aspect_ratio: bool,
}

/// A cache for caching the results of a sizing a Grid Item or Flexbox Item
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Cache {
    /// The cache entry for the node's final layout
    final_layout_entry: Option<CacheEntry<LayoutOutput>>,
    /// The cache entries for the node's preliminary size measurements
    measure_entries: [Option<CacheEntry<CachedMeasurement>>; CACHE_SIZE],
    /// Bounded cache for measurements that must be keyed by parent block-size.
    block_constraint_entries: [Option<CacheEntry<CachedMeasurement>>; BLOCK_CONSTRAINT_CACHE_SIZE],
    /// Next entry replaced when the block-constraint cache is full.
    next_block_constraint_entry: u8,
    /// Tracks if all cache entries are empty
    is_empty: bool,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    /// Create a new empty cache
    pub const fn new() -> Self {
        Self {
            final_layout_entry: None,
            measure_entries: [None; CACHE_SIZE],
            block_constraint_entries: [None; BLOCK_CONSTRAINT_CACHE_SIZE],
            next_block_constraint_entry: 0,
            is_empty: true,
        }
    }

    /// Return the cache slot to cache the current computed result in
    ///
    /// ## Caching Strategy
    ///
    /// We need multiple cache slots, because a node's size is often queried by it's parent multiple times in the course of the layout
    /// process, and we don't want later results to clobber earlier ones.
    ///
    /// The two variables that we care about when determining cache slot are:
    ///
    ///   - How many "known_dimensions" are set. In the worst case, a node may be called first with neither dimension known, then with one
    ///     dimension known (either width of height - which doesn't matter for our purposes here), and then with both dimensions known.
    ///   - Whether unknown dimensions are being sized under a min-content or a max-content available space constraint (definite available space
    ///     shares a cache slot with max-content because a node will generally be sized under one or the other but not both).
    ///
    /// ## Cache slots:
    ///
    /// - Slot 0: Both known_dimensions were set
    /// - Slots 1-4: 1 of 2 known_dimensions were set and:
    ///   - Slot 1: width but not height known_dimension was set and the other dimension was either a MaxContent or Definite available space constraintraint
    ///   - Slot 2: width but not height known_dimension was set and the other dimension was a MinContent constraint
    ///   - Slot 3: height but not width known_dimension was set and the other dimension was either a MaxContent or Definite available space constraintable space constraint
    ///   - Slot 4: height but not width known_dimension was set and the other dimension was a MinContent constraint
    /// - Slots 5-8: Neither known_dimensions were set and:
    ///   - Slot 5: x-axis available space is MaxContent or Definite and y-axis available space is MaxContent or Definite
    ///   - Slot 6: x-axis available space is MaxContent or Definite and y-axis available space is MinContent
    ///   - Slot 7: x-axis available space is MinContent and y-axis available space is MaxContent or Definite
    ///   - Slot 8: x-axis available space is MinContent and y-axis available space is MinContent
    ///
    /// Results that report a block-constraint dependency bypass these nine
    /// shape slots and use a separate bounded cache keyed by the full parent
    /// size. Independent results remain reusable across parent block-sizes.
    #[inline]
    fn compute_cache_slot(known_dimensions: Size<Option<f32>>, available_space: Size<AvailableSpace>) -> usize {
        use AvailableSpace::{Definite, MaxContent, MinContent};

        let has_known_width = known_dimensions.width.is_some();
        let has_known_height = known_dimensions.height.is_some();

        // Slot 0: Both known_dimensions were set
        if has_known_width && has_known_height {
            return 0;
        }

        // Slot 1: width but not height known_dimension was set and the other dimension was either a MaxContent or Definite available space constraint
        // Slot 2: width but not height known_dimension was set and the other dimension was a MinContent constraint
        if has_known_width && !has_known_height {
            return 1 + (available_space.height == MinContent) as usize;
        }

        // Slot 3: height but not width known_dimension was set and the other dimension was either a MaxContent or Definite available space constraint
        // Slot 4: height but not width known_dimension was set and the other dimension was a MinContent constraint
        if has_known_height && !has_known_width {
            return 3 + (available_space.width == MinContent) as usize;
        }

        // Slots 5-8: Neither known_dimensions were set and:
        match (available_space.width, available_space.height) {
            // Slot 5: x-axis available space is MaxContent or Definite and y-axis available space is MaxContent or Definite
            (MaxContent | Definite(_), MaxContent | Definite(_)) => 5,
            // Slot 6: x-axis available space is MaxContent or Definite and y-axis available space is MinContent
            (MaxContent | Definite(_), MinContent) => 6,
            // Slot 7: x-axis available space is MinContent and y-axis available space is MaxContent or Definite
            (MinContent, MaxContent | Definite(_)) => 7,
            // Slot 8: x-axis available space is MinContent and y-axis available space is MinContent
            (MinContent, MinContent) => 8,
        }
    }

    /// Try to retrieve a cached result from the cache
    #[inline]
    pub fn get(&self, input: &LayoutInput) -> Option<LayoutOutput> {
        let key = CacheKey::from(input);
        match input.run_mode {
            RunMode::PerformLayout => self.final_layout_entry.filter(|entry| entry.key == key).map(|e| e.content),
            RunMode::ComputeSize => self.get_size(input).map(LayoutOutput::from_intrinsic_size_result),
            RunMode::PerformHiddenLayout => None,
        }
    }

    /// Try to retrieve a dedicated intrinsic size result from the measurement
    /// caches.
    #[inline]
    pub fn get_size(&self, input: &LayoutInput) -> Option<IntrinsicSizeResult> {
        debug_assert_eq!(input.run_mode, RunMode::ComputeSize);
        let key = CacheKey::from(input);
        for entry in self.measure_entries.iter().flatten() {
            if entry.key.kd_available_space == key.kd_available_space
                && entry.key.inline_parent_size_and_axis() == key.inline_parent_size_and_axis()
                && entry.key.parent_writing_mode == key.parent_writing_mode
                && entry.key.sizing_mode == key.sizing_mode
                && entry.key.sizing_purpose == key.sizing_purpose
                && entry.key.block_auto_behavior == key.block_auto_behavior
            {
                return Some(IntrinsicSizeResult {
                    size: entry.content.size,
                    depends_on_block_constraints: entry.content.depends_on_block_constraints,
                    applied_aspect_ratio: entry.content.applied_aspect_ratio,
                });
            }
        }

        for entry in self.block_constraint_entries.iter().flatten() {
            if entry.key == key {
                return Some(IntrinsicSizeResult {
                    size: entry.content.size,
                    depends_on_block_constraints: entry.content.depends_on_block_constraints,
                    applied_aspect_ratio: entry.content.applied_aspect_ratio,
                });
            }
        }

        None
    }

    /// Store a computed size in the cache
    pub fn store(&mut self, input: &LayoutInput, layout_output: LayoutOutput) {
        let key = CacheKey::from(input);
        match input.run_mode {
            RunMode::PerformLayout => {
                self.is_empty = false;
                self.final_layout_entry = Some(CacheEntry { key, content: layout_output })
            }
            RunMode::ComputeSize => {
                self.store_size(input, layout_output.into_intrinsic_size_result());
            }
            RunMode::PerformHiddenLayout => {}
        }
    }

    /// Store a dedicated intrinsic size result in the appropriate measurement
    /// cache.
    pub fn store_size(&mut self, input: &LayoutInput, result: IntrinsicSizeResult) {
        debug_assert_eq!(input.run_mode, RunMode::ComputeSize);
        self.is_empty = false;
        let key = CacheKey::from(input);
        let entry = CacheEntry {
            key,
            content: CachedMeasurement {
                size: result.size,
                depends_on_block_constraints: result.depends_on_block_constraints,
                applied_aspect_ratio: result.applied_aspect_ratio,
            },
        };
        if result.depends_on_block_constraints {
            if let Some(existing_index) = self.block_constraint_entries.iter().position(|existing| match existing {
                Some(existing) => existing.key == key,
                None => false,
            }) {
                self.block_constraint_entries[existing_index] = Some(entry);
                return;
            }
            let cache_slot = self
                .block_constraint_entries
                .iter()
                .position(Option::is_none)
                .unwrap_or(self.next_block_constraint_entry as usize);
            self.block_constraint_entries[cache_slot] = Some(entry);
            self.next_block_constraint_entry = ((cache_slot + 1) % BLOCK_CONSTRAINT_CACHE_SIZE) as u8;
        } else {
            let cache_slot = Self::compute_cache_slot(input.known_dimensions, input.available_space);
            self.measure_entries[cache_slot] = Some(entry);
        }
    }

    /// Clear all cache entries and reports clear operation outcome ([`ClearState`])
    pub fn clear(&mut self) -> ClearState {
        if self.is_empty {
            return ClearState::AlreadyEmpty;
        }
        self.is_empty = true;
        self.final_layout_entry = None;
        self.measure_entries = [None; CACHE_SIZE];
        self.block_constraint_entries = [None; BLOCK_CONSTRAINT_CACHE_SIZE];
        self.next_block_constraint_entry = 0;
        ClearState::Cleared
    }

    /// Returns true if all cache entries are None, else false
    pub fn is_empty(&self) -> bool {
        self.final_layout_entry.is_none()
            && !self.measure_entries.iter().any(|entry| entry.is_some())
            && !self.block_constraint_entries.iter().any(|entry| entry.is_some())
    }
}

/// Clear operation outcome. See [`Cache::clear`]
pub enum ClearState {
    /// Cleared some values
    Cleared,
    /// Everything was already cleared
    AlreadyEmpty,
}

#[cfg(test)]
mod tests {
    use super::Cache;
    use crate::geometry::{Line, Size, WritingMode};
    use crate::style::AvailableSpace;
    use crate::tree::{
        AutoSizeBehavior, IntrinsicSizeResult, LayoutInput, LayoutOutput, RequestedAxis, RunMode, SizingMode,
        SizingPurpose,
    };

    fn input(sizing_purpose: SizingPurpose) -> LayoutInput {
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose,
            axis: RequestedAxis::Horizontal,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
            parent_size: Size::NONE,
            parent_writing_mode: WritingMode::HorizontalTb,
            available_space: Size { width: AvailableSpace::Definite(100.0), height: AvailableSpace::MaxContent },
            vertical_margins_are_collapsible: Line::FALSE,
        }
    }

    #[test]
    fn intrinsic_contributions_do_not_alias_layout_measurements() {
        let mut cache = Cache::new();
        let contribution = input(SizingPurpose::IntrinsicContribution);
        let layout = input(SizingPurpose::Layout);
        cache.store(&contribution, LayoutOutput::from_outer_size(Size { width: 60.0, height: 20.0 }));

        assert!(cache.get(&layout).is_none());
        assert_eq!(cache.get(&contribution).unwrap().size.width, 60.0);
    }

    #[test]
    fn intrinsic_cache_preserves_operation_provenance() {
        let mut cache = Cache::new();
        let input = input(SizingPurpose::IntrinsicContribution);
        let expected = IntrinsicSizeResult {
            size: Size { width: 60.0, height: 30.0 },
            depends_on_block_constraints: false,
            applied_aspect_ratio: true,
        };

        cache.store_size(&input, expected);

        assert_eq!(cache.get_size(&input), Some(expected));
    }

    #[test]
    fn intrinsic_measurements_distinguish_parent_block_constraints() {
        let mut cache = Cache::new();
        let mut initial = input(SizingPurpose::IntrinsicContribution);
        initial.parent_size = Size { width: Some(200.0), height: Some(100.0) };
        cache.store(
            &initial,
            LayoutOutput::from_outer_size(Size { width: 100.0, height: 100.0 }).with_block_constraint_dependency(true),
        );

        let mut changed_block_constraint = initial;
        changed_block_constraint.parent_size.height = Some(50.0);

        assert!(cache.get(&changed_block_constraint).is_none());
    }

    #[test]
    fn independent_measurements_ignore_parent_block_constraints() {
        let mut cache = Cache::new();
        let mut initial = input(SizingPurpose::IntrinsicContribution);
        initial.parent_size = Size { width: Some(200.0), height: Some(100.0) };
        cache.store(&initial, LayoutOutput::from_outer_size(Size { width: 80.0, height: 20.0 }));

        let mut changed_block_constraint = initial;
        changed_block_constraint.parent_size.height = Some(50.0);

        assert_eq!(cache.get(&changed_block_constraint).unwrap().size.width, 80.0);
    }

    #[test]
    fn dependent_measurements_retain_multiple_block_constraints() {
        let mut cache = Cache::new();
        let mut input = input(SizingPurpose::IntrinsicContribution);
        input.parent_size.width = Some(200.0);

        for height in [100.0, 50.0] {
            input.parent_size.height = Some(height);
            cache.store(
                &input,
                LayoutOutput::from_outer_size(Size { width: height, height }).with_block_constraint_dependency(true),
            );
        }

        input.parent_size.height = Some(100.0);
        assert_eq!(cache.get(&input).unwrap().size.width, 100.0);
        input.parent_size.height = Some(50.0);
        assert_eq!(cache.get(&input).unwrap().size.width, 50.0);
    }

    #[test]
    fn vertical_measurements_key_the_parent_inline_axis_in_logical_space() {
        let mut cache = Cache::new();
        let mut initial = input(SizingPurpose::IntrinsicContribution);
        initial.parent_writing_mode = WritingMode::VerticalRl;
        initial.parent_size = Size { width: Some(100.0), height: Some(200.0) };
        cache.store(&initial, LayoutOutput::from_outer_size(Size { width: 80.0, height: 20.0 }));

        let mut changed_block_constraint = initial;
        changed_block_constraint.parent_size.width = Some(50.0);
        assert_eq!(cache.get(&changed_block_constraint).unwrap().size.width, 80.0);

        let mut changed_inline_constraint = initial;
        changed_inline_constraint.parent_size.height = Some(150.0);
        assert!(cache.get(&changed_inline_constraint).is_none());
    }

    #[test]
    fn measurements_from_different_parent_writing_modes_do_not_alias() {
        let mut cache = Cache::new();
        let mut horizontal = input(SizingPurpose::IntrinsicContribution);
        horizontal.parent_size = Size { width: Some(100.0), height: Some(100.0) };
        cache.store(&horizontal, LayoutOutput::from_outer_size(Size { width: 80.0, height: 20.0 }));

        let mut vertical = horizontal;
        vertical.parent_writing_mode = WritingMode::VerticalRl;

        assert!(cache.get(&vertical).is_none());
    }
}
