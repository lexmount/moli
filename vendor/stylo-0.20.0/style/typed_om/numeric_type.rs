/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Typed OM Numeric Type.

use crate::values::generics::grid::FlexUnit;
use crate::values::generics::Optional;
use crate::values::specified::angle::AngleUnit;
use crate::values::specified::frequency::FrequencyUnit;
use crate::values::specified::length::LengthUnit;
use crate::values::specified::resolution::ResolutionUnit;
use crate::values::specified::time::TimeUnit;

/// https://drafts.css-houdini.org/css-typed-om-1/#cssnumericvalue-base-type
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
pub enum NumericBaseType {
    /// A `<length>` unit.
    Length,

    /// An `<angle>` unit.
    Angle,

    /// A `<time>` unit.
    Time,

    /// A `<frequency>` unit.
    Frequency,

    /// A `<resolution>` unit.
    Resolution,

    /// The `<flex>` unit.
    Flex,

    /// The percentage unit.
    Percent,
}

#[doc(hidden)] // Need to be public so that cbindgen generates it.
pub const NUMERIC_BASE_TYPE_COUNT: usize = 7;

const_assert!(NumericBaseType::Percent as usize + 1 == NUMERIC_BASE_TYPE_COUNT);

/// Every numeric base type in enum-declaration order.
pub const ALL_NUMERIC_BASE_TYPES: [NumericBaseType; NUMERIC_BASE_TYPE_COUNT] = [
    NumericBaseType::Length,
    NumericBaseType::Angle,
    NumericBaseType::Time,
    NumericBaseType::Frequency,
    NumericBaseType::Resolution,
    NumericBaseType::Flex,
    NumericBaseType::Percent,
];

const fn all_numeric_base_types_are_in_order() -> bool {
    let mut i = 0;
    while i < NUMERIC_BASE_TYPE_COUNT - 1 {
        if ALL_NUMERIC_BASE_TYPES_EXCEPT_PERCENT[i] as u8 != i as u8 {
            return false;
        }
        i += 1;
    }
    true
}

const_assert!(all_numeric_base_types_are_in_order());

/// Every numeric base type except `Percent` in enum-declaration order.
const ALL_NUMERIC_BASE_TYPES_EXCEPT_PERCENT: [NumericBaseType; NUMERIC_BASE_TYPE_COUNT - 1] = [
    NumericBaseType::Length,
    NumericBaseType::Angle,
    NumericBaseType::Time,
    NumericBaseType::Frequency,
    NumericBaseType::Resolution,
    NumericBaseType::Flex,
];

const fn all_numeric_base_types_except_percent_are_in_order() -> bool {
    let mut i = 0;
    while i < NUMERIC_BASE_TYPE_COUNT - 1 {
        if ALL_NUMERIC_BASE_TYPES_EXCEPT_PERCENT[i] as u8 != i as u8 {
            return false;
        }
        i += 1;
    }
    true
}

const_assert!(all_numeric_base_types_except_percent_are_in_order());

/// https://drafts.css-houdini.org/css-typed-om-1/#numeric-typing
///
/// The spec models the per-base-type exponents as an ordered map keyed by base
/// type. We use a fixed-size array indexed by `NumericBaseType` instead. A
/// missing entry in the spec's map and a zero entry are observably equivalent
/// for every operation the spec defines (comparisons and iteration only
/// consider non-zero entries), so the array representation is simpler, avoids
/// allocations, and produces the same results.
///
/// `non_zero_count` and `non_zero_except_percent_count` are derived fields
/// maintained in sync with `exponents`, allowing O(1) type compatibility
/// checks. They fit without padding into the 2 bytes following `percent_hint`,
/// so the struct remains 32 bytes.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct NumericType {
    exponents: [i32; NUMERIC_BASE_TYPE_COUNT],
    percent_hint: Optional<NumericBaseType>,
    non_zero_count: u8,
    non_zero_except_percent_count: u8,
}

impl NumericType {
    #[inline]
    fn empty() -> Self {
        Self {
            exponents: [0; NUMERIC_BASE_TYPE_COUNT],
            percent_hint: Optional::None,
            non_zero_count: 0,
            non_zero_except_percent_count: 0,
        }
    }

    /// Constructs a numeric type from a single base type.
    ///
    /// Keep Gecko's StyleNumericType::WithBaseType() in sync with this
    /// implementation.
    #[inline]
    fn with_base_type(base_type: NumericBaseType) -> Self {
        let mut result = Self::empty();
        result.exponents[base_type as usize] = 1;
        result.non_zero_count = 1;
        if base_type != NumericBaseType::Percent {
            result.non_zero_except_percent_count = 1;
        }
        result
    }

    /// A numeric type whose exponent map is empty.
    pub fn number() -> Self {
        Self::empty()
    }

    /// A numeric type whose percent exponent is 1.
    pub fn percent() -> Self {
        Self::with_base_type(NumericBaseType::Percent)
    }

    /// A numeric type whose length exponent is 1.
    pub fn length() -> Self {
        Self::with_base_type(NumericBaseType::Length)
    }

    /// A numeric type whose angle exponent is 1.
    pub fn angle() -> Self {
        Self::with_base_type(NumericBaseType::Angle)
    }

    /// A numeric type whose time exponent is 1.
    pub fn time() -> Self {
        Self::with_base_type(NumericBaseType::Time)
    }

    /// A numeric type whose frequency exponent is 1.
    pub fn frequency() -> Self {
        Self::with_base_type(NumericBaseType::Frequency)
    }

    /// A numeric type whose resolution exponent is 1.
    pub fn resolution() -> Self {
        Self::with_base_type(NumericBaseType::Resolution)
    }

    /// A numeric type whose flex exponent is 1.
    pub fn flex() -> Self {
        Self::with_base_type(NumericBaseType::Flex)
    }

    /// <https://drafts.css-houdini.org/css-typed-om-1/#create-a-type-from-a-string>
    pub fn try_from_unit(unit: &str) -> Result<Self, ()> {
        if unit.eq_ignore_ascii_case("number") {
            return Ok(Self::number());
        }

        if unit.eq_ignore_ascii_case("percent") {
            return Ok(Self::percent());
        }

        if LengthUnit::from_str(unit).is_ok() {
            return Ok(Self::length());
        }

        if AngleUnit::from_str(unit).is_ok() {
            return Ok(Self::angle());
        }

        if TimeUnit::from_str(unit).is_ok() {
            return Ok(Self::time());
        }

        if FrequencyUnit::from_str(unit).is_ok() {
            return Ok(Self::frequency());
        }

        if ResolutionUnit::from_str(unit).is_ok() {
            return Ok(Self::resolution());
        }

        if FlexUnit::matches(unit) {
            return Ok(Self::flex());
        }

        Err(())
    }

    /// Creates a numeric type from a previously validated unit string.
    pub fn from_unit_unchecked(unit: &str) -> Self {
        let result = Self::try_from_unit(unit);
        debug_assert!(result.is_ok(), "Expected a valid unit, got {unit:?}");

        result.unwrap_or(Self::number())
    }

    fn exponent(&self, base_type: NumericBaseType) -> i32 {
        self.exponents[base_type as usize]
    }

    fn set_exponent(&mut self, base_type: NumericBaseType, new_value: i32) {
        let old_value = self.exponent(base_type);
        self.exponents[base_type as usize] = new_value;
        match (old_value != 0, new_value != 0) {
            (false, true) => {
                self.non_zero_count += 1;
                if base_type != NumericBaseType::Percent {
                    self.non_zero_except_percent_count += 1;
                }
            },
            (true, false) => {
                self.non_zero_count -= 1;
                if base_type != NumericBaseType::Percent {
                    self.non_zero_except_percent_count -= 1;
                }
            },
            _ => {},
        }
    }

    fn add_exponent(&mut self, base_type: NumericBaseType, delta: i32) {
        self.set_exponent(base_type, self.exponent(base_type) + delta);
    }

    /// <https://drafts.css-houdini.org/css-typed-om-1/#apply-the-percent-hint>
    fn apply_percent_hint(&mut self, hint: NumericBaseType) {
        // Step 1.
        self.percent_hint = Optional::Some(hint);

        // Step 2.
        // No-op for our array representation, the hint's slot already exists
        // ("missing" and "zero" mean the same thing).

        // Step 3.
        if hint != NumericBaseType::Percent {
            let percent = self.exponent(NumericBaseType::Percent);
            if percent != 0 {
                self.add_exponent(hint, percent);
                self.set_exponent(NumericBaseType::Percent, 0);
            }
        }
    }

    /// <https://drafts.css-houdini.org/css-typed-om-1/#cssnumericvalue-add-two-types>
    ///
    /// The algorithm has some complexity and uses branches rather than
    /// numbered sub-steps, so the implementation below quotes spec text
    /// inline. This is more verbose than usual, but should help map each
    /// branch back to the spec when reviewing or debugging.
    fn add_two_types(type1: &NumericType, type2: &NumericType) -> Result<Self, ()> {
        // Step 1.
        // "Replace type1 with a fresh copy of type1, and type2 with a fresh
        // copy of type2."
        let mut type1 = type1.clone();
        let mut type2 = type2.clone();
        // "Let finalType be a new type with an initially empty ordered map and
        // an initially null percent hint."
        // We don't need a separate finalType with the array representation,
        // when the entries match, type1 already represents the merged result.

        // Step 2.
        match (type1.percent_hint, type2.percent_hint) {
            // Step 2, first branch.
            // "If both type1 and type2 have non-null percent hints with
            // different values"
            (Optional::Some(h1), Optional::Some(h2)) if h1 != h2 => {
                // "The types can't be added. Return failure."
                return Err(());
            },
            // Step 2, second branch.
            // "If type1 has a non-null percent hint hint and type2 doesn't"
            (Optional::Some(hint), Optional::None) => {
                // "Apply the percent hint hint to type2."
                type2.apply_percent_hint(hint)
            },
            // "Vice versa if type2 has a non-null percent hint and type1
            // doesn't."
            (Optional::None, Optional::Some(hint)) => type1.apply_percent_hint(hint),
            // Step 3, third branch.
            // "Otherwise"
            _ => {
                // "Continue to the next step."
            },
        }

        // Step 3, first branch.
        // "If all the entries of type1 with non-zero values are contained in
        // type2 with the same value, and vice-versa"
        // With the array representation, the check reduces to array equality
        // ("missing" and "zero" mean the same thing).
        if type1.exponents == type2.exponents {
            // "Copy all of type1’s entries to finalType, and then copy all of
            // type2’s entries to finalType that finalType doesn’t already
            // contain. Set finalType’s percent hint to type1’s percent hint.
            // Return finalType."
            // As noted in Step1, type1 already represents the merged result,
            // so extra finalType is not needed.
            return Ok(type1);
        }

        // Step 3, second branch.
        // "If type1 and/or type2 contain 'percent' with a non-zero value, and
        // type1 and/or type2 contain a key other than 'percent' with a
        // non-zero value"
        if (type1.exponent(NumericBaseType::Percent) != 0
            || type2.exponent(NumericBaseType::Percent) != 0)
            && (type1.non_zero_except_percent_count != 0
                || type2.non_zero_except_percent_count != 0)
        {
            // "For each base type other than 'percent' hint:"
            for &hint in ALL_NUMERIC_BASE_TYPES_EXCEPT_PERCENT.iter() {
                // Step 3.1.
                // "Provisionally apply the percent hint hint to both type1
                // and type2."
                // Instead of modifying type1 and type2 directly and then
                // eventually reverting them to the original state, we just
                // clone them.
                let mut type1 = type1.clone();
                let mut type2 = type2.clone();
                type1.apply_percent_hint(hint);
                type2.apply_percent_hint(hint);

                // Step 3.2.
                // "If, afterwards, all the entries of type1 with non-zero
                // values are contained in type2 with the same value, and vice
                // versa,"
                // With the array representation, the check reduces to array
                // equality ("missing" and "zero" mean the same thing).
                if type1.exponents == type2.exponents {
                    // "then copy all of type1’s entries to finalType, and
                    // then copy all of type2’s entries to finalType that
                    // finalType doesn’t already contain. Set finalType’s
                    // percent hint to hint. Return finalType."
                    // type1 already represents the merged result, so extra
                    // finalType is not needed.
                    return Ok(type1);
                }

                // Step 3.3.
                // "Otherwise, revert type1 and type2 to their state at the
                // start of this loop."
                // The revert is implicit, t1 and t2 are discarded between
                // iterations.
            }
            // "If the loop finishes without returning finalType, then the
            // types can’t be added. Return failure."
            return Err(());
        }

        // Step 3, third branch.
        // "Otherwise"
        // "The types can't be added. Return failure."
        Err(())
    }

    /// <https://drafts.css-houdini.org/css-typed-om-1/#cssnumericvalue-multiply-two-types>
    ///
    /// Spec text is quoted inline to make each step easy to map back to the
    /// algorithm during review.
    fn multiply_two_types(type1: &NumericType, type2: &NumericType) -> Result<Self, ()> {
        // Step 1.
        // "Replace type1 with a fresh copy of type1, and type2 with a fresh
        // copy of type2."
        let mut type1 = type1.clone();
        let mut type2 = type2.clone();
        // "Let finalType be a new type with an initially empty ordered map
        // and an initially null percent hint."
        // We don't need a separate finalType with the array representation,
        // since multiplying types is equivalent to adding the exponents,
        // type1 can be used directly.

        match (type1.percent_hint, type2.percent_hint) {
            // Step 2.
            // "If both type1 and type2 have non-null percent hints with
            // different values, the types can't be multiplied."
            (Optional::Some(h1), Optional::Some(h2)) if h1 != h2 => {
                // "Return failure."
                return Err(());
            },
            // Step 3.
            // "If type1 has a non-null percent hint hint and type2 doesn't, "
            (Optional::Some(hint), Optional::None) => {
                // "apply the percent hint hint to type2."
                type2.apply_percent_hint(hint)
            },
            // "Vice versa if type2 has a non-null percent hint and type1
            // doesn't."
            (Optional::None, Optional::Some(hint)) => type1.apply_percent_hint(hint),
            _ => {},
        }

        // Step 4.
        // "Copy all of type1's entries to finalType,"
        // As noted in Step 1, type1 can be used directly, so a separate
        // finalType is not needed.
        // "then for each baseType -> power of type2:"
        for &base_type in ALL_NUMERIC_BASE_TYPES.iter() {
            let power = type2.exponent(base_type);

            // The spec iterates only the baseType → power entries present in
            // type2. With the array representation we iterate all base types,
            // so skip entries whose exponent is zero.
            if power == 0 {
                continue;
            }

            // Step 4.1.
            // "If finalType[baseType] exists, increment its value by power."
            // Step 4.2.
            // "Otherwise, set finalType[baseType] to power."
            // With the array representation, both cases are handled by adding
            // the exponent, because missing entries are represented as zero.
            type1.add_exponent(base_type, power);
        }
        // "Set finalType's percent hint to type1's percent hint."
        // After Step 3, type1's percent hint equals type2's in all surviving
        // cases (both null, both equal, or the null side was filled in), so
        // type1 already has the final hint.

        // Step 5.
        // "Return finalType."
        Ok(type1)
    }

    fn combine_types<'a, I>(
        mut types: I,
        combine: fn(&NumericType, &NumericType) -> Result<NumericType, ()>,
    ) -> Result<Self, ()>
    where
        I: Iterator<Item = &'a NumericType>,
    {
        let mut result = types.next().ok_or(())?.clone();

        for next in types {
            result = combine(&result, next)?;
        }

        Ok(result)
    }

    /// Applies the add two types algorithm repeatedly across a sequence of
    /// numeric types, returning the combined type.
    pub fn add_types<'a, I>(types: I) -> Result<Self, ()>
    where
        I: Iterator<Item = &'a NumericType>,
    {
        Self::combine_types(types, Self::add_two_types)
    }

    /// Applies the multiply two types algorithm repeatedly across a sequence of
    /// numeric types, returning the combined type.
    pub fn multiply_types<'a, I>(types: I) -> Result<Self, ()>
    where
        I: Iterator<Item = &'a NumericType>,
    {
        Self::combine_types(types, Self::multiply_two_types)
    }
}
