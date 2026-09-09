/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::animate::AnimationFieldAttrs;
use crate::{cg, to_computed_value};
use proc_macro2::TokenStream;
use syn::DeriveInput;
use synstructure::BindStyle;

pub fn derive(input: DeriveInput) -> TokenStream {
    let trait_impl = |from_body, to_body| {
        quote! {
             #[inline]
             fn from_animated_value(from: Self::AnimatedValue) -> Self {
                 #from_body
             }

             #[inline]
             fn to_animated_value(self, context: &crate::values::animated::Context) -> Self::AnimatedValue {
                 #to_body
             }
        }
    };

    to_computed_value::derive_to_value(
        input,
        parse_quote!(crate::values::animated::ToAnimatedValue),
        parse_quote!(AnimatedValue),
        BindStyle::Move,
        |binding| {
            let attrs = cg::parse_field_attrs::<AnimationFieldAttrs>(&binding.ast());
            to_computed_value::ToValueAttrs {
                field_bound: attrs.field_bound,
                no_field_bound: false,
            }
        },
        |binding| quote!(crate::values::animated::ToAnimatedValue::from_animated_value(#binding)),
        |binding| quote!(crate::values::animated::ToAnimatedValue::to_animated_value(#binding, context)),
        trait_impl,
    )
}
