/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Typed OM.
//!
//! https://drafts.css-houdini.org/css-typed-om-1/

use crate::derives::*;
use crate::values::computed::url::ComputedUrl;
use crate::values::generics::transform::GenericMatrix3D;
use crate::values::specified::url::SpecifiedUrl;
use crate::values::CSSFloat;
use crate::{One, Zero};
use app_units::Au;
use servo_arc::Arc;
use style_traits::CssString;
use thin_vec::ThinVec;

pub mod numeric;
pub mod numeric_declaration;
pub mod numeric_type;
pub mod sum_value;

pub use numeric_type::NumericType;

/// A single segment of an unparsed Typed OM value.
///
/// This corresponds to the `CSSUnparsedSegment` union in the Typed OM
/// specification. Unparsed values are represented as a list of string
/// fragments and variable references.
#[derive(Clone, Debug)]
#[repr(C)]
pub enum UnparsedSegment {
    /// A string fragment.
    ///
    /// This corresponds to the string branch of `CSSUnparsedSegment` and is
    /// used for the non-variable parts of a `CSSUnparsedValue`.
    String(CssString),

    /// A `var()` reference segment.
    ///
    /// This corresponds to `CSSVariableReferenceValue` in the Typed OM
    /// specification.
    VariableReference(VariableReferenceValue),
}

/// An unparsed value used by the Typed OM.
///
/// This corresponds to `CSSUnparsedValue` in the Typed OM specification. It
/// is used for values that cannot be reified into a more specific
/// property-agnostic representation and therefore need to preserve their
/// token-like structure as a sequence of string fragments and variable
/// references.
///
/// The underlying list of segments corresponds to the `[[tokens]]` internal
/// slot of `CSSUnparsedValue`.
///
/// This is represented as a type alias over `ThinVec<UnparsedSegment>` rather
/// than a dedicated struct. This avoids the need for additional wrapper types
/// when embedding unparsed values within other structures, while still
/// allowing recursive representations via the segment list.
pub type UnparsedValue = ThinVec<UnparsedSegment>;

/// A variable reference inside an unparsed Typed OM value.
///
/// This corresponds to `CSSVariableReferenceValue` in the Typed OM
/// specification.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct VariableReferenceValue {
    /// The referenced custom property name.
    ///
    /// This corresponds to the `variable` attribute of
    /// `CSSVariableReferenceValue`.
    pub variable: CssString,

    /// The fallback value, if present.
    ///
    /// This corresponds to the `fallback` attribute of
    /// `CSSVariableReferenceValue`. When `has_fallback` is false, this value
    /// must be ignored. When `has_fallback` is true, this contains the
    /// fallback tokens (which may be empty).
    pub fallback: UnparsedValue,

    /// Whether a fallback was explicitly provided.
    ///
    /// This is needed to distinguish between the absence of a fallback
    /// (`var(--a)`) and an explicitly empty fallback (`var(--a,)`), which are
    /// observable via Typed OM.
    pub has_fallback: bool,
}

/// A keyword value used by the Typed OM.
///
/// This corresponds to `CSSKeywordValue` in the Typed OM specification.
/// The keyword is stored as a `CssString` so it can be represented and
/// transferred independently of any specific property (e.g. `"none"`,
/// `"block"`, `"thin"`).
#[derive(Clone, Debug)]
#[repr(C)]
pub struct KeywordValue(pub CssString);

/// A single numeric value with an associated unit.
///
/// This corresponds to `CSSUnitValue` in the Typed OM specification. The
/// numeric component is stored separately from the textual unit identifier.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct UnitValue {
    /// The numeric type associated with this value.
    pub numeric_type: NumericType,

    /// The numeric component of the value.
    pub value: f32,

    /// The textual unit string (e.g. `"px"`, `"em"`, `"%"`, `"deg"`).
    pub unit: CssString,
}

impl UnitValue {
    /// Returns the unit as a string slice.
    #[inline]
    pub fn unit_str(&self) -> &str {
        #[cfg(feature = "gecko")]
        unsafe {
            self.unit.as_str_unchecked()
        }

        #[cfg(feature = "servo")]
        {
            &self.unit
        }
    }
}

/// A sum of numeric values.
///
/// This corresponds to `CSSMathSum` in the Typed OM specification. A sum
/// value represents an expression such as `10px + 2em`. Each entry is itself
/// a `NumericValue`, allowing nested sums if needed.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct MathSum {
    /// The numeric type associated with this sum.
    pub numeric_type: NumericType,

    /// The list of numeric terms that make up the sum.
    pub values: ThinVec<NumericValue>,
}

impl MathSum {
    /// Creates a math sum from a sequence of numeric values.
    ///
    /// Returns an error if the values do not have addable numeric types.
    pub fn try_from_numeric_values(values: ThinVec<NumericValue>) -> Result<Self, ()> {
        // Temporarily ignore NumericValue variants that don't expose a
        // numeric type. This filter can be removed once numeric type support
        // is implemented for all NumericValue variants.
        let numeric_type = NumericType::add_types(values.iter().filter_map(|v| v.numeric_type()))?;

        Ok(Self {
            numeric_type,
            values,
        })
    }

    /// Creates a math sum from a previously validated sequence of numeric
    /// values.
    pub fn from_numeric_values_unchecked(values: ThinVec<NumericValue>) -> Self {
        let result = Self::try_from_numeric_values(values);
        debug_assert!(result.is_ok(), "Expected addable values");

        result.unwrap_or_else(|_| Self {
            numeric_type: NumericType::number(),
            values: ThinVec::from([NumericValue::zero()]),
        })
    }
}

/// A product of numeric values.
///
/// This corresponds to `CSSMathProduct` in the Typed OM specification. A
/// product value represents an expression such as `10px * 2`. Each entry is
/// itself a `NumericValue`, allowing nested math expressions if needed.
pub type MathProduct = ThinVec<NumericValue>;

/// A negated numeric value.
///
/// This corresponds to `CSSMathNegate` in the Typed OM specification. A negate
/// expression represents constructs such as `-10px` or `-(10px + 2em)`.
pub type MathNegate = Box<NumericValue>;

/// An inverted numeric value.
///
/// This corresponds to `CSSMathInvert` in the Typed OM specification. An
/// invert expression represents constructs such as `1 / 2`, `1 / 10px`, or
/// more generally the reciprocal of another numeric value.
pub type MathInvert = Box<NumericValue>;

/// A minimum expression over numeric values.
///
/// This corresponds to `CSSMathMin` in the Typed OM specification. A minimum
/// expression represents constructs such as `min(10px, 20%)`. Each entry is
/// itself a `NumericValue`, allowing nested math expressions if needed.
pub type MathMin = ThinVec<NumericValue>;

/// A maximum expression over numeric values.
///
/// This corresponds to `CSSMathMax` in the Typed OM specification. A maximum
/// expression represents constructs such as `max(10px, 20%)`. Each entry is
/// itself a `NumericValue`, allowing nested math expressions if needed.
pub type MathMax = ThinVec<NumericValue>;

/// A clamp expression over numeric values.
///
/// This corresponds to `CSSMathClamp` in the Typed OM specification. A clamp
/// expression represents constructs such as `clamp(10px, 20%, 30px)`.
///
/// The array entries correspond to the lower bound, value, and upper bound,
/// respectively.
pub type MathClamp = crate::OwnedArray<NumericValue, 3>;

/// A math expression used by the Typed OM.
///
/// This corresponds to `CSSMathValue` and its subclasses in the Typed OM
/// specification.
#[derive(Clone, Debug)]
#[repr(C)]
pub enum MathValue {
    /// A sum of numeric values.
    ///
    /// This corresponds to `CSSMathSum`.
    Sum(MathSum),

    /// A product of numeric values.
    ///
    /// This corresponds to `CSSMathProduct`.
    Product(MathProduct),

    /// A negated numeric value.
    ///
    /// This corresponds to `CSSMathNegate`.
    Negate(MathNegate),

    /// An inverted numeric value.
    ///
    /// This corresponds to `CSSMathInvert`.
    Invert(MathInvert),

    /// A minimum expression over numeric values.
    ///
    /// This corresponds to `CSSMathMin`.
    Min(MathMin),

    /// A maximum expression over numeric values.
    ///
    /// This corresponds to `CSSMathMax`.
    Max(MathMax),

    /// A clamp expression over numeric values.
    ///
    /// This corresponds to `CSSMathClamp`.
    Clamp(MathClamp),
}

impl MathValue {
    /// Returns the numeric type associated with this math value, if
    /// available.
    pub fn numeric_type(&self) -> Option<&NumericType> {
        match self {
            Self::Sum(math_sum) => Some(&math_sum.numeric_type),
            _ => None,
        }
    }
}

/// A numeric value used by the Typed OM.
///
/// This corresponds to `CSSNumericValue` and its subclasses in the Typed OM
/// specification. It represents numbers that can appear in CSS values,
/// including both simple unit quantities and composite expressions.
///
/// Unlike the parser-level representation, `NumericValue` is property-agnostic
/// and suitable for conversion to or from the `CSSNumericValue` family of DOM
/// objects.
#[derive(Clone, Debug)]
#[repr(C)]
pub enum NumericValue {
    /// A single numeric value with a concrete unit.
    ///
    /// This corresponds to `CSSUnitValue`.
    Unit(UnitValue),

    /// A math expression.
    ///
    /// This corresponds to `CSSMathValue` and its subclasses.
    Math(MathValue),
}

impl NumericValue {
    /// Returns a zero pixel unit value.
    #[inline]
    pub fn zero_px() -> Self {
        Self::Unit(UnitValue {
            numeric_type: NumericType::length(),
            value: 0.0,
            unit: CssString::from("px"),
        })
    }

    /// Returns the numeric type associated with this numeric value, if
    /// available.
    pub fn numeric_type(&self) -> Option<&NumericType> {
        match self {
            Self::Unit(unit_value) => Some(&unit_value.numeric_type),
            Self::Math(math_value) => math_value.numeric_type(),
        }
    }
}

impl Zero for NumericValue {
    #[inline]
    fn zero() -> Self {
        Self::Unit(UnitValue {
            numeric_type: NumericType::number(),
            value: 0.0,
            unit: CssString::from("number"),
        })
    }

    #[inline]
    fn is_zero(&self) -> bool {
        match *self {
            Self::Unit(ref value) => value.value == 0.0,
            _ => false,
        }
    }
}

impl One for NumericValue {
    #[inline]
    fn one() -> Self {
        Self::Unit(UnitValue {
            numeric_type: NumericType::number(),
            value: 1.0,
            unit: CssString::from("number"),
        })
    }

    #[inline]
    fn is_one(&self) -> bool {
        match *self {
            Self::Unit(ref value) => value.value == 1.0,
            _ => false,
        }
    }
}

/// A translate transform component used by the Typed OM.
///
/// This corresponds to `CSSTranslate` in the Typed OM specification. The `x`,
/// `y`, and `z` components are always present; omitted offsets are represented
/// as `0px`.
///
/// The `is_2d` flag indicates whether the component was reified from a 2D
/// translate function.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct TranslateComponent {
    /// The x-axis translation component.
    pub x: NumericValue,

    /// The y-axis translation component.
    pub y: NumericValue,

    /// The z-axis translation component.
    pub z: NumericValue,

    /// Whether this translate component is two-dimensional.
    pub is_2d: bool,
}

/// A rotate transform component used by the Typed OM.
///
/// This corresponds to `CSSRotate` in the Typed OM specification. The `angle`,
/// `x`, `y`, and `z` components are always present; omitted axis coordinates
/// are represented using the implicit axis for the corresponding rotate
/// function.
///
/// The `is_2d` flag indicates whether the component was reified from a 2D
/// rotate function.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct RotateComponent {
    /// The rotation angle.
    pub angle: NumericValue,

    /// The x-axis rotation coordinate.
    pub x: NumericValue,

    /// The y-axis rotation coordinate.
    pub y: NumericValue,

    /// The z-axis rotation coordinate.
    pub z: NumericValue,

    /// Whether this rotate component is two-dimensional.
    pub is_2d: bool,
}

/// A scale transform component used by the Typed OM.
///
/// This corresponds to `CSSScale` in the Typed OM specification. The `x`, `y`,
/// and `z` components are always present; omitted scale factors are
/// represented as `1`.
///
/// The `is_2d` flag indicates whether the component was reified from a 2D
/// scale function.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct ScaleComponent {
    /// The x-axis scale factor.
    pub x: NumericValue,

    /// The y-axis scale factor.
    pub y: NumericValue,

    /// The z-axis scale factor.
    pub z: NumericValue,

    /// Whether this scale component is two-dimensional.
    pub is_2d: bool,
}

/// A skew transform component used by the Typed OM.
///
/// This corresponds to `CSSSkew` in the Typed OM specification. The `ax` and
/// `ay` components are always present; omitted angles are represented as
/// `0deg`.
///
/// Skew components are always two-dimensional.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct SkewComponent {
    /// The x-axis skew angle.
    pub ax: NumericValue,

    /// The y-axis skew angle.
    pub ay: NumericValue,
}

/// A skewX transform component used by the Typed OM.
///
/// This corresponds to `CSSSkewX` in the Typed OM specification. The value is
/// always present; omitted angles are represented as `0deg`.
///
/// SkewX components are always two-dimensional.
pub type SkewXComponent = NumericValue;

/// A skewY transform component used by the Typed OM.
///
/// This corresponds to `CSSSkewY` in the Typed OM specification. The value is
/// always present; omitted angles are represented as `0deg`.
///
/// SkewY components are always two-dimensional.
pub type SkewYComponent = NumericValue;

/// A perspective value used by a perspective component.
///
/// This corresponds to the `CSSPerspectiveValue` union in the Typed OM
/// specification.
#[derive(Clone, Debug)]
#[repr(C)]
pub enum PerspectiveValue {
    /// A numeric perspective value.
    ///
    /// This corresponds to `CSSNumericValue`.
    Numeric(NumericValue),

    /// A keyword perspective value.
    ///
    /// This corresponds to `CSSKeywordValue`.
    Keyword(KeywordValue),
}

/// A perspective transform component used by the Typed OM.
///
/// This corresponds to `CSSPerspective` in the Typed OM specification. The
/// `length` component is always present.
///
/// Perspective components are always three-dimensional.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct PerspectiveComponent {
    /// The perspective length.
    pub length: PerspectiveValue,
}

/// A matrix transform component used by the Typed OM.
///
/// This corresponds to `CSSMatrixComponent` in the Typed OM specification.
///
/// The `matrix` field always stores a full 4×4 matrix. Two-dimensional
/// matrices are expanded to their equivalent 3D representation during
/// reification.
///
/// The `is_2d` flag indicates whether the component was reified from a 2D
/// matrix function.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct MatrixComponent {
    /// The 4×4 matrix.
    pub matrix: GenericMatrix3D<CSSFloat>,

    /// Whether this matrix component is two-dimensional.
    pub is_2d: bool,
}

/// A single transform component used by the Typed OM.
///
/// This corresponds to `CSSTransformComponent` in the Typed OM specification.
/// Each variant represents one concrete transform component subclass.
#[derive(Clone, Debug)]
#[repr(C)]
pub enum TransformComponent {
    /// A translate transform component.
    ///
    /// This corresponds to `CSSTranslate`.
    Translate(TranslateComponent),

    /// A rotate transform component.
    ///
    /// This corresponds to `CSSRotate`.
    Rotate(RotateComponent),

    /// A scale transform component.
    ///
    /// This corresponds to `CSSScale`.
    Scale(ScaleComponent),

    /// A skew transform component.
    ///
    /// This corresponds to `CSSSkew`.
    Skew(SkewComponent),

    /// A skewX transform component.
    ///
    /// This corresponds to `CSSSkewX`.
    SkewX(SkewXComponent),

    /// A skewY transform component.
    ///
    /// This corresponds to `CSSSkewY`.
    SkewY(SkewYComponent),

    /// A perspective transform component.
    ///
    /// This corresponds to `CSSPerspective`.
    Perspective(PerspectiveComponent),

    /// A matrix transform component.
    ///
    /// This corresponds to `CSSMatrixComponent`.
    Matrix(MatrixComponent),
}

/// A transform value used by the Typed OM.
///
/// This corresponds to `CSSTransformValue` in the Typed OM specification. It
/// represents a `<transform-list>` as an ordered list of transform components.
pub type TransformValue = ThinVec<TransformComponent>;

/// An image value used by the Typed OM.
///
/// This corresponds to `CSSImageValue` in the Typed OM specification.
///
/// `CSSImageValue` objects represent values for properties that take
/// `<image>` values.
#[derive(Clone, Debug, ToCss)]
#[repr(C)]
pub enum ImageValue {
    /// A specified image URL value.
    ///
    /// Relative URLs are preserved and continue to resolve against the
    /// originating stylesheet or document when later used.
    Specified(SpecifiedUrl),

    /// A computed image URL value.
    ///
    /// Computed URLs are already resolved according to normal CSS computed
    /// value processing.
    Computed(ComputedUrl),
}

/// A property-agnostic representation of a value, used by Typed OM.
///
/// `TypedValue` is the internal counterpart of the various `CSSStyleValue`
/// subclasses defined by the Typed OM specification. It captures values that
/// can be represented independently of any particular property.
#[derive(Clone, Debug)]
#[repr(C)]
pub enum TypedValue {
    /// An unparsed value consisting of string fragments and variable
    /// references.
    ///
    /// This corresponds to `CSSUnparsedValue` in the Typed OM specification.
    Unparsed(UnparsedValue),

    /// A keyword value such as `"block"`, `"none"`, or `"thin"`.
    ///
    /// This corresponds to `CSSKeywordValue` in the Typed OM specification.
    /// Keywords are represented as a standalone `KeywordValue` so they can
    /// be carried and compared independently of any particular property.
    Keyword(KeywordValue),

    /// A numeric value such as a length, angle, time, or a sum thereof.
    ///
    /// This corresponds to the `CSSNumericValue` hierarchy in the Typed OM
    /// specification, including `CSSUnitValue` and `CSSMathSum`.
    Numeric(NumericValue),

    /// A transform value such as `translate(10px, 20px)`.
    ///
    /// This corresponds to `CSSTransformValue` in the Typed OM specification.
    Transform(TransformValue),

    /// An image value.
    ///
    /// This corresponds to `CSSImageValue` in the Typed OM specification.
    Image(ImageValue),
}

/// A list of property-agnostic values used by the Typed OM.
///
/// `TypedValueList` is the internal counterpart of CSS value lists exposed by
/// Typed OM. It stores one or more [`TypedValue`] items in source order and
/// is used when a value reifies to multiple property-agnostic components.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct TypedValueList {
    /// The list of reified values.
    pub values: ThinVec<TypedValue>,
}

/// Reifies a value into its Typed OM representation.
///
/// This trait is the Typed OM analogue of [`ToCss`]. Instead of serializing
/// values into CSS syntax, it converts them into [`TypedValue`]s that can be
/// exposed to the DOM as `CSSStyleValue` subclasses.
///
/// Most consumers should use [`ToTyped::to_typed_value`] or
/// [`ToTyped::to_typed_value_list`], depending on whether they need a single
/// reified value or the full list of reified values.
///
/// This trait is derivable with `#[derive(ToTyped)]`. The derived
/// implementation currently supports:
///
/// * Keyword enums: Enums whose variants are all unit variants are
///   automatically reified as [`TypedValue::Keyword`], using the same
///   serialization logic as [`ToCss`].
///
/// * Bitflags structs: Structs annotated with
///   `#[css(bitflags(single = "...", mixed = "...", overlapping_bits))]`
///   are automatically reified as [`TypedValue::Keyword`] values when they
///   can be represented as a single CSS keyword. Values that would serialize
///   to multiple CSS keywords are treated as unsupported.
///
/// * Structs and data-carrying variants: Unless treated specially (such as
///   bitflags structs), the derive attempts to call `.to_typed()` recursively
///   on supported fields or variant payloads, producing [`TypedValue`]s when
///   possible.
///
/// * Other cases: If no automatic mapping is defined, or recursion is
///   explicitly disabled, the derived implementation falls back to the
///   default method (which returns `Err(())`, and thus `to_typed_value()`
///   returns `None`).
///
/// Over time, the derive may be extended to handle additional CSS value
/// categories such as numeric, color, and transform types.
///
/// Summary of derive attributes recognized by `#[derive(ToTyped)]`:
///
/// * `#[css(bitflags(single = "...", mixed = "...", overlapping_bits))]` on a
///   struct generates keyword reification for CSS bitflags types. Values that
///   can be represented as a single CSS keyword are reified as
///   [`TypedValue::Keyword`]; values that would serialize to multiple CSS
///   keywords are treated as unsupported and return `Err(())`.
///
///   `overlapping_bits` is supported for bitflags where one keyword subsumes
///   other internal bits, such as `contain: size`.
///
/// * `#[typed(skip_derive_fields)]` on the type disables recursion for
///   structs and data-carrying enum variants.
///
/// * `#[css(skip)]`, `#[typed(skip)]`, or `#[typed(todo)]` on a variant cause
///   that variant to be treated as unsupported (the derived implementation
///   returns `Err(())`).
///
/// * `#[css(skip)]` on a field causes that field to be ignored during
///   reification.
///
/// * `#[css(skip_if = "...")]` / `#[typed(skip_if = "...")]` on a field
///   conditionally disables reification for that field. If the provided
///   function returns `true` for the field value, the field is ignored.
///
/// * `#[css(contextual_skip_if = "...")]` /
///   `#[typed(contextual_skip_if = "...")]` on a field conditionally disables
///   reification for that field. The provided function is called with all
///   fields in the current struct or variant. If it returns `true`, the field
///   is ignored.
///
///   Typed skip annotations override CSS skip annotations when both are
///   present.
///
/// * `#[css(keyword = "...")]` on a unit variant overrides the keyword that
///   would otherwise be derived from the Rust identifier.
///
/// * `#[css(comma)]` on the variant indicates that supported fields may reify
///   to multiple separate values. When this attribute is present, multiple
///   [`TypedValue`] items may be produced, unless
///   `#[typed(no_multiple_values)]` is also present. If multiple values are
///   not allowed and the derived implementation would produce more than one
///   item, it returns `Err(())`.
///
/// * `#[typed(no_multiple_values)]` on a variant prevents it from reifying to
///   multiple [`TypedValue`] items, even if `#[css(comma)]` is present.
///
/// * `#[css(iterable)]` on a field indicates that the field represents a list
///   of values. Each item in the iterable is reified individually by calling
///   `ToTyped::to_typed` on the element type.
///
/// * `#[css(if_empty = "...")]` on an iterable field specifies a keyword
///   value that should be produced when the iterable is empty.
///
/// * `#[css(represents_keyword)]` on a bool field causes the field name to be
///   reified as a keyword when the field is true.
pub trait ToTyped {
    /// Attempt to convert `self` into one or more [`TypedValue`] items.
    ///
    /// Implementations append any resulting values to `dest`. This is the
    /// low-level entry point used by the Typed OM reification infrastructure.
    /// Most callers should prefer [`ToTyped::to_typed_value`] or
    /// [`ToTyped::to_typed_value_list`].
    ///
    /// Returning `Err(())` indicates that the value cannot be represented as
    /// a property-agnostic Typed OM value.
    fn to_typed(&self, _dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        Err(())
    }

    /// Attempt to convert `self` into a [`TypedValue`].
    ///
    /// Returns the first reified value as `Some(TypedValue)` if the value can
    /// be reified into a property-agnostic CSSStyleValue subclass. Returns
    /// `None` if the value is unrepresentable, in which case consumers
    /// produce a property-tied CSSStyleValue instead.
    fn to_typed_value(&self) -> Option<TypedValue> {
        let mut dest = ThinVec::new();
        self.to_typed(&mut dest).ok()?;
        dest.into_iter().next()
    }

    /// Attempt to convert `self` into a [`NumericValue`].
    ///
    /// Returns `Some(NumericValue)` if the value reifies to a single
    /// `TypedValue::Numeric` item. Returns `None` otherwise.
    fn to_numeric_value(&self) -> Option<NumericValue> {
        match self.to_typed_value()? {
            TypedValue::Numeric(value) => Some(value),
            _ => None,
        }
    }

    /// Attempt to convert `self` into a [`TypedValueList`].
    ///
    /// Returns `Some(TypedValueList)` if the value can be reified into one or
    /// more property-agnostic Typed OM values. Returns `None` if the value is
    /// unrepresentable, in which case consumers produce a property-tied
    /// `CSSStyleValue` instead.
    fn to_typed_value_list(&self) -> Option<TypedValueList> {
        let mut dest = ThinVec::new();
        self.to_typed(&mut dest).ok()?;
        Some(TypedValueList { values: dest })
    }
}

impl<'a, T> ToTyped for &'a T
where
    T: ToTyped + ?Sized,
{
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        (*self).to_typed(dest)
    }
}

impl<T> ToTyped for Box<T>
where
    T: ?Sized + ToTyped,
{
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        (**self).to_typed(dest)
    }
}

impl<T> ToTyped for Arc<T>
where
    T: ?Sized + ToTyped,
{
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        (**self).to_typed(dest)
    }
}

impl ToTyped for Au {
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        let numeric_type = NumericType::length();
        let value = self.to_f32_px();
        let unit = CssString::from("px");
        dest.push(TypedValue::Numeric(NumericValue::Unit(UnitValue {
            numeric_type,
            value,
            unit,
        })));
        Ok(())
    }
}

macro_rules! impl_to_typed_for_predefined_type {
    ($name: ty) => {
        impl<'a> ToTyped for $name {
            fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
                dest.push(TypedValue::Numeric(NumericValue::Unit(UnitValue {
                    numeric_type: NumericType::number(),
                    value: *self as f32,
                    unit: CssString::from("number"),
                })));
                Ok(())
            }
        }
    };
}

impl_to_typed_for_predefined_type!(f32);
impl_to_typed_for_predefined_type!(i8);
impl_to_typed_for_predefined_type!(i32);
