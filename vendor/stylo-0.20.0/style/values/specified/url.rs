/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Common handling for the specified value CSS url() values.

use crate::values::generics::url::GenericUrlOrNone;

pub use crate::url::SpecifiedUrl;

/// Specified <url> | <none>
pub type UrlOrNone = GenericUrlOrNone<SpecifiedUrl>;
