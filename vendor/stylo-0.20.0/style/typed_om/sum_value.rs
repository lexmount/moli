/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Typed OM Sum Value.

use crate::typed_om::numeric::NoCalcNumeric;
use crate::typed_om::numeric_type::NumericType;
use crate::typed_om::{MathSum, MathValue, NumericValue, UnitValue};
use itertools::Itertools;
use std::collections::HashMap;
use style_traits::CssString;
use thin_vec::ThinVec;

type UnitMap = HashMap<String, i32>;

// <https://drafts.css-houdini.org/css-typed-om-1/#product-of-two-unit-maps>
fn product_of_two_unit_maps(s: &UnitMap, other: &UnitMap) -> UnitMap {
    // Step 1.
    let mut result = s.clone();

    // Step 2.
    for (unit, power) in other {
        // Step 2.1 & 2.2.
        *result.entry(unit.clone()).or_insert(0) += power;
    }

    // Step 3.
    result
}

/// <https://drafts.css-houdini.org/css-typed-om-1/#cssnumericvalue-sum-value>
#[derive(Clone, Debug)]
struct SumValueItem {
    value: f32,
    unit_map: UnitMap,
}

impl SumValueItem {
    /// <https://drafts.css-houdini.org/css-typed-om-1/#create-a-cssunitvalue-from-a-sum-value-item>
    fn to_unit_value(&self) -> Result<UnitValue, ()> {
        // Step 1.
        if self.unit_map.len() > 1 {
            return Err(());
        }

        // Step 2.
        if self.unit_map.is_empty() {
            return Ok(UnitValue {
                numeric_type: NumericType::number(),
                value: self.value,
                unit: CssString::from("number"),
            });
        }

        // Step 3.
        let (unit, power) = self.unit_map.iter().next().unwrap();
        if *power != 1 {
            return Err(());
        }

        // Step 4.
        Ok(UnitValue {
            numeric_type: NumericType::from_unit_unchecked(unit),
            value: self.value,
            unit: CssString::from(unit),
        })
    }
}

/// <https://drafts.css-houdini.org/css-typed-om-1/#cssnumericvalue-sum-value>
#[derive(Clone, Debug)]
pub struct SumValue(Vec<SumValueItem>);

impl SumValue {
    /// <https://drafts.css-houdini.org/css-typed-om-1/#create-a-sum-value>
    pub fn try_from_numeric_value(value: &NumericValue) -> Result<Self, ()> {
        match value {
            // CSSUnitValue
            NumericValue::Unit(unit_value) => {
                // Step 1.
                let mut value = unit_value.value;
                let mut unit = unit_value.unit_str();

                // Step2.
                let numeric = NoCalcNumeric::parse_unit_value(value, unit)?;
                if let Some(canonical_unit) = numeric.canonical_unit() {
                    let canonical = numeric.to(canonical_unit)?;
                    value = canonical.unitless_value();
                    unit = canonical.unit();
                }

                // Step 3.
                if unit.eq_ignore_ascii_case("number") {
                    return Ok(Self(vec![SumValueItem {
                        value,
                        unit_map: UnitMap::new(),
                    }]));
                }

                // Step 4.
                Ok(Self(vec![SumValueItem {
                    value,
                    unit_map: [(unit.to_owned(), 1)].into_iter().collect::<UnitMap>(),
                }]))
            },

            // CSSMathSum
            NumericValue::Math(MathValue::Sum(math_sum)) => {
                // Step 1.
                let mut values: Vec<SumValueItem> = Vec::new();

                // Step 2.
                for item in &math_sum.values {
                    // Step 2.1.
                    let value = SumValue::try_from_numeric_value(item)?;

                    // Step 2.2.
                    for sub_value in value.0 {
                        // Step 2.2.1.
                        if let Some(item) = values
                            .iter_mut()
                            .find(|item| item.unit_map == sub_value.unit_map)
                        {
                            item.value += sub_value.value;
                            continue;
                        }

                        // Step 2.2.2.
                        values.push(sub_value);
                    }
                }

                // Step 3.

                // TODO: Create a type [1] from the unit map of each item of
                // values, and add [2] all the types together. If the result is
                // failure, return failure.
                //
                // [1] https://drafts.css-houdini.org/css-typed-om-1/#create-a-type-from-a-unit-map
                // [2] https://drafts.css-houdini.org/css-typed-om-1/#cssnumericvalue-add-two-types

                // Step 4.
                Ok(Self(values))
            },

            // CSSMathProduct
            NumericValue::Math(MathValue::Product(math_product)) => {
                // Step 1.
                let mut values = vec![SumValueItem {
                    value: 1.0,
                    unit_map: Default::default(),
                }];

                // Step 2.
                for item in math_product {
                    // Step 2.1 & 2.2.
                    let new_values = SumValue::try_from_numeric_value(item)?;

                    let mut temp = Vec::new();

                    // Step 2.3.
                    for item1 in &values {
                        // Step 2.3.1.
                        for item2 in &new_values.0 {
                            // Step 2.3.1.1.
                            let mut unit_map =
                                product_of_two_unit_maps(&item1.unit_map, &item2.unit_map);
                            unit_map.retain(|_, power| *power != 0);
                            let item = SumValueItem {
                                value: item1.value * item2.value,
                                unit_map,
                            };

                            // Step 2.3.1.2.
                            temp.push(item);
                        }
                    }

                    // Step 2.4.
                    values = temp;
                }

                // Step 3.
                Ok(Self(values))
            },

            // CSSMathNegate
            NumericValue::Math(MathValue::Negate(math_negate)) => {
                // Step 1 & 2.
                let mut values = SumValue::try_from_numeric_value(math_negate)?.0;

                // Step 3.
                for item in &mut values {
                    item.value = -item.value;
                }

                // Step 4.
                Ok(Self(values))
            },

            // CSSMathInvert
            NumericValue::Math(MathValue::Invert(math_invert)) => {
                // Step 1 & 2.
                let mut values = SumValue::try_from_numeric_value(math_invert)?.0;

                // Step 3.
                if values.len() != 1 {
                    return Err(());
                }

                let item = &mut values[0];

                // Step 4.
                item.value = 1.0 / item.value;
                for power in item.unit_map.values_mut() {
                    *power = -*power;
                }

                // Step 5.
                Ok(Self(values))
            },

            // CSSMathMin
            NumericValue::Math(MathValue::Min(math_min)) => {
                // Step 1 & 2.
                let mut args = Vec::new();

                for item in math_min {
                    let values = SumValue::try_from_numeric_value(item)?;

                    if values.0.len() > 1 {
                        return Err(());
                    }

                    args.push(values);
                }

                debug_assert!(!args.is_empty());

                // Step 3.
                if !args.iter().map(|arg| &arg.0[0].unit_map).all_equal() {
                    return Err(());
                }

                // Step 4.
                let min = args
                    .into_iter()
                    .map(|arg| arg.0.into_iter().next().unwrap())
                    .min_by(|a, b| a.value.total_cmp(&b.value))
                    .ok_or(())?;

                Ok(Self(vec![min]))
            },

            // CSSMathMax
            NumericValue::Math(MathValue::Max(math_max)) => {
                // Step 1 & 2.
                let mut args = Vec::new();

                for item in math_max {
                    let values = SumValue::try_from_numeric_value(item)?;

                    if values.0.len() > 1 {
                        return Err(());
                    }

                    args.push(values);
                }
                debug_assert!(!args.is_empty());

                // Step 3.
                if !args.iter().map(|arg| &arg.0[0].unit_map).all_equal() {
                    return Err(());
                }

                // Step 4.
                let max = args
                    .into_iter()
                    .map(|arg| arg.0.into_iter().next().unwrap())
                    .max_by(|a, b| a.value.total_cmp(&b.value))
                    .ok_or(())?;

                Ok(Self(vec![max]))
            },

            // CSSMathClamp
            //
            // TODO: The spec currently does not define "create a sum value"
            // for CSSMathClamp. The implementation below follows WPT and
            // existing browser implementations. Proposed spec steps:
            //
            // 1. Let args be lower, value, and upper, each replaced by the
            //    result of creating a sum value from the corresponding
            //    internal slot.
            //
            // 2. If any item of args is failure, or has a length greater than
            //    one, return failure.
            //
            // 3. If not all of the unit maps among the items of args are
            //    identical, return failure.
            //
            // 4. Clamp value's sole item's value between lower's and upper's
            //    sole item's values, and return value.
            //
            // See https://github.com/w3c/csswg-drafts/issues/14038
            NumericValue::Math(MathValue::Clamp(math_clamp)) => {
                // Step 1 & 2.
                let lower = SumValue::try_from_numeric_value(&math_clamp[0])?;
                let value = SumValue::try_from_numeric_value(&math_clamp[1])?;
                let upper = SumValue::try_from_numeric_value(&math_clamp[2])?;

                if lower.0.len() > 1 || value.0.len() > 1 || upper.0.len() > 1 {
                    return Err(());
                }

                // Step 3.
                if lower.0[0].unit_map != value.0[0].unit_map
                    || lower.0[0].unit_map != upper.0[0].unit_map
                {
                    return Err(());
                }

                // Step 4.
                let mut value = value.0.into_iter().next().unwrap();
                value.value = value.value.max(lower.0[0].value).min(upper.0[0].value);

                Ok(Self(vec![value]))
            },
        }
    }

    /// Step 3 of:
    /// https://drafts.css-houdini.org/css-typed-om-1/#dom-cssnumericvalue-to
    pub fn to_unit(&self, unit: &str) -> Result<UnitValue, ()> {
        debug_assert!(NumericType::try_from_unit(unit).is_ok());

        if self.0.len() != 1 {
            return Err(());
        }

        let sole_item = &self.0[0];

        let item = sole_item.to_unit_value()?;

        let item = {
            let numeric = NoCalcNumeric::parse_unit_value(item.value, item.unit_str())?;
            let converted = numeric.to(unit)?;

            UnitValue {
                numeric_type: NumericType::from_unit_unchecked(converted.unit()),
                value: converted.unitless_value(),
                unit: CssString::from(converted.unit()),
            }
        };

        Ok(item)
    }

    /// Step 3-6 of:
    /// https://drafts.css-houdini.org/css-typed-om-1/#dom-cssnumericvalue-tosum
    pub fn to_units(&self, units: &[&str]) -> Result<MathSum, ()> {
        debug_assert!(units
            .iter()
            .all(|unit| NumericType::try_from_unit(unit).is_ok()));

        // Step 3.
        let mut values = self
            .0
            .iter()
            .map(|item| item.to_unit_value())
            .collect::<Result<Vec<_>, _>>()?;

        // Step 4.
        if units.is_empty() {
            values.sort_by(|a, b| a.unit.cmp(&b.unit));

            return Ok(MathSum::from_numeric_values_unchecked(
                values.into_iter().map(NumericValue::Unit).collect(),
            ));
        }

        // Step 5.
        let mut result = ThinVec::new();

        for unit in units {
            // Step 5.1.
            let mut temp = UnitValue {
                numeric_type: NumericType::from_unit_unchecked(*unit),
                value: 0.0,
                unit: CssString::from(*unit),
            };

            // Step 5.2.
            let mut i = 0;
            while i < values.len() {
                let value = &values[i];

                // Step 5.2.1.
                let value_unit = value.unit_str();

                // Step 5.2.2 & 5.2.2.1.
                let numeric = NoCalcNumeric::parse_unit_value(value.value, &value_unit)?;
                if let Ok(converted) = numeric.to(unit) {
                    // Step 5.2.2.2.
                    temp.value += converted.unitless_value();

                    // Step 5.2.2.3.
                    values.remove(i);
                } else {
                    i += 1;
                }
            }

            // Step 5.3.
            result.push(NumericValue::Unit(temp));
        }

        // Step 6.
        if !values.is_empty() {
            return Err(());
        }

        Ok(MathSum::from_numeric_values_unchecked(result))
    }
}
