/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Specified frequency values.

use crate::derives::*;
use cssparser::match_ignore_ascii_case;

/// The unit of a `<frequency>` value.
#[derive(Clone, Copy, Debug, MallocSizeOf, PartialEq, PartialOrd, ToShmem)]
#[repr(u8)]
pub enum FrequencyUnit {
    /// `Hz`
    Hz,
    /// `kHz`
    Khz,
}

impl FrequencyUnit {
    /// Returns the frequency unit for the given string.
    #[inline]
    pub fn from_str(unit: &str) -> Result<Self, ()> {
        Ok(match_ignore_ascii_case! { unit,
            "hz" => Self::Hz,
            "khz" => Self::Khz,
             _ => return Err(())
        })
    }

    /// Returns this unit as a string.
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hz => "hz",
            Self::Khz => "khz",
        }
    }
}
