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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stringify_matches_chromium_json_values() {
        // Fractional cases from Blink's DtoaTest.ToFixedPrecisionString.
        // JSONBasicValue uses int32 for whole values in range, double otherwise.
        for (input, expected) in [
            ("0.00000123123123", "0.00000123123"),
            ("0.000000123123123", "1.23123e-7"),
            ("123123.123", "123123"),
            ("1231231.23", "1.23123e+6"),
            ("1e-10", "1.00000e-10"),
            ("0.001953125", "0.00195313"),
            ("2147483647", "2147483647"),
            ("5000000000", "5.00000e+9"),
        ] {
            let number = serde_json::from_str(input).unwrap();
            assert_eq!(stringify(&number), expected, "{input}");
        }
    }

    #[test]
    fn integer_only_accepts_signed_32_bit_whole_numbers() {
        for (input, expected) in [
            ("-2147483648", Some(i32::MIN)),
            ("2147483647", Some(i32::MAX)),
            ("-1", Some(-1)),
            ("0", Some(0)),
            ("1.0", Some(1)),
            ("2147483648", None),
            ("-2147483649", None),
            ("0.5", None),
        ] {
            let number = serde_json::from_str(input).unwrap();
            assert_eq!(integer(&number), expected, "{input}");
        }
    }
}
