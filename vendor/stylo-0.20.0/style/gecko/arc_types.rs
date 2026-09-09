/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This file lists all arc FFI types and defines corresponding addref and release functions. This
//! list loosely corresponds to ServoLockedArcTypeList.inc file in Gecko.

#![allow(non_snake_case, missing_docs)]

use crate::media_queries::MediaList;
use crate::properties::animated_properties::AnimationValue;
use crate::properties::{ComputedValues, PropertyDeclarationBlock};
use crate::shared_lock::Locked;
use crate::stylesheets::keyframes_rule::Keyframe;
use crate::stylesheets::{
    AppearanceBaseRule, ContainerRule, CssRules, CustomMediaRule, DocumentRule,
    FontFeatureValuesRule, FontPaletteValuesRule, LayerBlockRule, LayerStatementRule, MarginRule,
    MediaRule, NamespaceRule, PropertyRule, ScopeRule, StartingStyleRule, StylesheetContents,
    SupportsRule, ViewTransitionRule,
};
pub use crate::stylesheets::{
    LockedCounterStyleRule, LockedFontFaceRule, LockedImportRule, LockedKeyframesRule,
    LockedNestedDeclarationsRule, LockedPageRule, LockedPositionTryRule, LockedStyleRule,
};
use crate::url::gecko::CssUrlData;
use servo_arc::Arc;

macro_rules! impl_simple_arc_ffi {
    ($ty:ty, $addref:ident, $release:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $addref(obj: *const $ty) {
            std::mem::forget(Arc::from_raw_addrefed(obj));
        }

        #[no_mangle]
        pub unsafe extern "C" fn $release(obj: *const $ty) {
            let _ = Arc::from_raw(obj);
        }
    };
}

macro_rules! impl_locked_arc_ffi {
    ($servo_type:ty, $alias:ident, $addref:ident, $release:ident) => {
        /// A simple alias for a locked type.
        pub type $alias = Locked<$servo_type>;
        impl_simple_arc_ffi!($alias, $addref, $release);
    };
}

impl_locked_arc_ffi!(
    CssRules,
    LockedCssRules,
    Servo_CssRules_AddRef,
    Servo_CssRules_Release
);
impl_locked_arc_ffi!(
    PropertyDeclarationBlock,
    LockedDeclarationBlock,
    Servo_DeclarationBlock_AddRef,
    Servo_DeclarationBlock_Release
);
impl_simple_arc_ffi!(
    LockedStyleRule,
    Servo_StyleRule_AddRef,
    Servo_StyleRule_Release
);
impl_simple_arc_ffi!(
    LockedImportRule,
    Servo_ImportRule_AddRef,
    Servo_ImportRule_Release
);
impl_locked_arc_ffi!(
    Keyframe,
    LockedKeyframe,
    Servo_Keyframe_AddRef,
    Servo_Keyframe_Release
);
impl_simple_arc_ffi!(
    LockedKeyframesRule,
    Servo_KeyframesRule_AddRef,
    Servo_KeyframesRule_Release
);
impl_simple_arc_ffi!(
    LayerBlockRule,
    Servo_LayerBlockRule_AddRef,
    Servo_LayerBlockRule_Release
);
impl_simple_arc_ffi!(
    LayerStatementRule,
    Servo_LayerStatementRule_AddRef,
    Servo_LayerStatementRule_Release
);
impl_locked_arc_ffi!(
    MediaList,
    LockedMediaList,
    Servo_MediaList_AddRef,
    Servo_MediaList_Release
);
impl_simple_arc_ffi!(MediaRule, Servo_MediaRule_AddRef, Servo_MediaRule_Release);
impl_simple_arc_ffi!(
    CustomMediaRule,
    Servo_CustomMediaRule_AddRef,
    Servo_CustomMediaRule_Release
);
impl_simple_arc_ffi!(
    NamespaceRule,
    Servo_NamespaceRule_AddRef,
    Servo_NamespaceRule_Release
);
impl_simple_arc_ffi!(
    MarginRule,
    Servo_MarginRule_AddRef,
    Servo_MarginRule_Release
);
impl_simple_arc_ffi!(
    LockedPageRule,
    Servo_PageRule_AddRef,
    Servo_PageRule_Release
);
impl_simple_arc_ffi!(
    PropertyRule,
    Servo_PropertyRule_AddRef,
    Servo_PropertyRule_Release
);
impl_simple_arc_ffi!(
    SupportsRule,
    Servo_SupportsRule_AddRef,
    Servo_SupportsRule_Release
);
impl_simple_arc_ffi!(
    ContainerRule,
    Servo_ContainerRule_AddRef,
    Servo_ContainerRule_Release
);
impl_simple_arc_ffi!(
    DocumentRule,
    Servo_DocumentRule_AddRef,
    Servo_DocumentRule_Release
);
impl_simple_arc_ffi!(
    FontFeatureValuesRule,
    Servo_FontFeatureValuesRule_AddRef,
    Servo_FontFeatureValuesRule_Release
);
impl_simple_arc_ffi!(
    FontPaletteValuesRule,
    Servo_FontPaletteValuesRule_AddRef,
    Servo_FontPaletteValuesRule_Release
);
impl_simple_arc_ffi!(
    LockedFontFaceRule,
    Servo_FontFaceRule_AddRef,
    Servo_FontFaceRule_Release
);
impl_simple_arc_ffi!(
    LockedCounterStyleRule,
    Servo_CounterStyleRule_AddRef,
    Servo_CounterStyleRule_Release
);

impl_simple_arc_ffi!(
    StylesheetContents,
    Servo_StyleSheetContents_AddRef,
    Servo_StyleSheetContents_Release
);
impl_simple_arc_ffi!(
    CssUrlData,
    Servo_CssUrlData_AddRef,
    Servo_CssUrlData_Release
);
impl_simple_arc_ffi!(
    ComputedValues,
    Servo_ComputedStyle_AddRef,
    Servo_ComputedStyle_Release
);
impl_simple_arc_ffi!(
    AnimationValue,
    Servo_AnimationValue_AddRef,
    Servo_AnimationValue_Release
);
impl_simple_arc_ffi!(ScopeRule, Servo_ScopeRule_AddRef, Servo_ScopeRule_Release);
impl_simple_arc_ffi!(
    StartingStyleRule,
    Servo_StartingStyleRule_AddRef,
    Servo_StartingStyleRule_Release
);
impl_simple_arc_ffi!(
    AppearanceBaseRule,
    Servo_AppearanceBaseRule_AddRef,
    Servo_AppearanceBaseRule_Release
);

impl_simple_arc_ffi!(
    LockedPositionTryRule,
    Servo_PositionTryRule_AddRef,
    Servo_PositionTryRule_Release
);
impl_simple_arc_ffi!(
    LockedNestedDeclarationsRule,
    Servo_NestedDeclarationsRule_AddRef,
    Servo_NestedDeclarationsRule_Release
);
impl_simple_arc_ffi!(
    ViewTransitionRule,
    Servo_ViewTransitionRule_AddRef,
    Servo_ViewTransitionRule_Release
);
