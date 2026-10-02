//! Blink JSONValue's int32 and six-significant-digit double conversions.

use num_bigint::BigUint;
use serde_json::Number;

pub(crate) fn integer(number: &Number) -> Option<i32> {
    let value = number.as_f64()?;
    (value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX) && value.fract() == 0.0)
        .then_some(value as i32)
}

pub(crate) fn stringify(number: &Number) -> String {
    if let Some(integer) = integer(number) {
        return integer.to_string();
    }
    let value = number.as_f64().expect("finite JSON number");
    let bits = value.abs().to_bits();
    let encoded_exponent = ((bits >> 52) & 0x7ff) as i32;
    let significand = bits & ((1_u64 << 52) - 1);
    let (significand, binary_exponent) = if encoded_exponent == 0 {
        (significand, -1074)
    } else {
        (significand | (1_u64 << 52), encoded_exponent - 1023 - 52)
    };
    let mut numerator = BigUint::from(significand);
    let mut denominator = BigUint::from(1_u8);
    if binary_exponent >= 0 {
        numerator <<= binary_exponent as usize;
    } else {
        denominator <<= binary_exponent.unsigned_abs() as usize;
    }
    let mut exponent = value.abs().log10().floor() as i32;
    let scale = 5 - exponent;
    let power = BigUint::from(10_u8).pow(scale.unsigned_abs());
    if scale >= 0 {
        numerator *= power;
    } else {
        denominator *= power;
    }
    // Round the exact binary value, including halfway cases, without a
    // second floating-point rounding when multiplying by a decimal scale.
    let mut coefficient = &numerator / &denominator;
    if ((&numerator % &denominator) << 1_usize) >= denominator {
        coefficient += BigUint::from(1_u8);
    }
    let mut digits = coefficient.to_string();
    if digits.len() == 7 {
        digits.pop();
        exponent += 1;
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if !(-6..6).contains(&exponent) {
        // Blink retains trailing mantissa zeros in exponential notation.
        return format!("{sign}{}.{}e{exponent:+}", &digits[..1], &digits[1..]);
    }
    let fixed = if exponent < 0 {
        format!("0.{}{digits}", "0".repeat((-exponent - 1) as usize))
    } else {
        let point = (exponent + 1) as usize;
        if point < digits.len() {
            digits.insert(point, '.');
        }
        digits
    };
    let fixed = if fixed.contains('.') {
        fixed.trim_end_matches('0').trim_end_matches('.')
    } else {
        &fixed
    };
    format!("{sign}{fixed}")
}
