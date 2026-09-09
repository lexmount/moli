/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Media-query device and expression representation.

use crate::color::AbsoluteColor;
use crate::custom_properties::CssEnvironment;
#[cfg(feature = "servo")]
use crate::derives::*;
use crate::properties::ComputedValues;
use crate::values::computed::font::QueryFontMetricsFlags;
use crate::values::computed::Length;
use parking_lot::RwLock;
use servo_arc::Arc;
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[cfg(feature = "gecko")]
use crate::device::gecko::ExtraDeviceData;
#[cfg(feature = "servo")]
use crate::device::servo::ExtraDeviceData;

#[cfg(feature = "gecko")]
pub mod gecko;
#[cfg(feature = "servo")]
pub mod servo;

/// A device is a structure that represents the current media a given document
/// is displayed in.
///
/// This is the struct against which media queries are evaluated, has a default
/// values computed, and contains all the viewport rule state.
///
/// This structure also contains atomics used for computing root font-relative
/// units. These atomics use relaxed ordering, since when computing the style
/// of the root element, there can't be any other style being computed at the
/// same time (given we need the style of the parent to compute everything else).
///
/// In Gecko, it wraps a pres context.
#[cfg_attr(feature = "servo", derive(Debug, MallocSizeOf))]
pub struct Device {
    /// The default computed values for this Device.
    #[cfg_attr(feature = "servo", ignore_malloc_size_of = "Arc is shared")]
    default_values: Arc<ComputedValues>,
    /// Current computed style of the root element, used for calculations of
    /// root font-relative units.
    #[cfg_attr(feature = "servo", ignore_malloc_size_of = "Arc")]
    root_style: RwLock<Arc<ComputedValues>>,
    /// Font size of the root element, used for rem units in other elements.
    root_font_size: AtomicU32,
    /// Line height of the root element, used for rlh units in other elements.
    root_line_height: AtomicU32,
    /// X-height of the root element, used for rex units in other elements.
    root_font_metrics_ex: AtomicU32,
    /// Cap-height of the root element, used for rcap units in other elements.
    root_font_metrics_cap: AtomicU32,
    /// Advance measure (ch) of the root element, used for rch units in other elements.
    root_font_metrics_ch: AtomicU32,
    /// Ideographic advance measure of the root element, used for ric units in other elements.
    root_font_metrics_ic: AtomicU32,
    /// Whether any styles computed in the document relied on the root font-size
    /// by using rem units.
    used_root_font_size: AtomicBool,
    /// Whether any styles computed in the document relied on the root line-height
    /// by using rlh units.
    used_root_line_height: AtomicBool,
    /// Whether any styles computed in the document relied on the root font metrics
    /// by using rcap, rch, rex, or ric units. This is a lock instead of an atomic
    /// in order to prevent concurrent writes to the root font metric values.
    #[cfg_attr(feature = "servo", ignore_malloc_size_of = "Pure stack type")]
    used_root_font_metrics: RwLock<bool>,
    /// Whether any styles computed in the document relied on font metrics.
    used_font_metrics: AtomicBool,
    /// Whether any styles computed in the document relied on the viewport size
    /// by using vw/vh/vmin/vmax units.
    used_viewport_size: AtomicBool,
    /// Whether any styles computed in the document relied on the viewport size
    /// by using dvw/dvh/dvmin/dvmax units.
    used_dynamic_viewport_size: AtomicBool,
    /// The CssEnvironment object responsible of getting CSS environment
    /// variables.
    environment: CssEnvironment,
    /// The body text color, stored as an `nscolor`, used for the "tables
    /// inherit from body" quirk.
    ///
    /// <https://quirks.spec.whatwg.org/#the-tables-inherit-color-from-body-quirk>
    body_text_color: AtomicU32,

    /// Extra Gecko-specific or Servo-specific data.
    extra: ExtraDeviceData,
}

impl Device {
    /// Get the relevant environment to resolve `env()` functions.
    #[inline]
    pub fn environment(&self) -> &CssEnvironment {
        &self.environment
    }

    /// Returns the default computed values as a reference, in order to match
    /// Servo.
    pub fn default_computed_values(&self) -> &ComputedValues {
        &self.default_values
    }

    /// Returns the default computed values as an `Arc`.
    pub fn default_computed_values_arc(&self) -> &Arc<ComputedValues> {
        &self.default_values
    }

    /// Store a pointer to the root element's computed style, for use in
    /// calculation of root font-relative metrics.
    pub fn set_root_style(&self, style: &Arc<ComputedValues>) {
        *self.root_style.write() = style.clone();
    }

    /// Get the font size of the root element (for rem)
    pub fn root_font_size(&self) -> Length {
        self.used_root_font_size.store(true, Ordering::Relaxed);
        Length::new(f32::from_bits(self.root_font_size.load(Ordering::Relaxed)))
    }

    /// Set the font size of the root element (for rem), in zoom-independent CSS pixels.
    pub fn set_root_font_size(&self, size: f32) {
        self.root_font_size.store(size.to_bits(), Ordering::Relaxed)
    }

    /// Get the line height of the root element (for rlh)
    pub fn root_line_height(&self) -> Length {
        self.used_root_line_height.store(true, Ordering::Relaxed);
        Length::new(f32::from_bits(
            self.root_line_height.load(Ordering::Relaxed),
        ))
    }

    /// Set the line height of the root element (for rlh), in zoom-independent CSS pixels.
    pub fn set_root_line_height(&self, size: f32) {
        self.root_line_height
            .store(size.to_bits(), Ordering::Relaxed);
    }

    /// Get the x-height of the root element (for rex)
    pub fn root_font_metrics_ex(&self) -> Length {
        self.ensure_root_font_metrics_updated();
        Length::new(f32::from_bits(
            self.root_font_metrics_ex.load(Ordering::Relaxed),
        ))
    }

    /// Set the x-height of the root element (for rex), in zoom-independent CSS pixels.
    pub fn set_root_font_metrics_ex(&self, size: f32) -> bool {
        let size = size.to_bits();
        let previous = self.root_font_metrics_ex.swap(size, Ordering::Relaxed);
        previous != size
    }

    /// Get the cap-height of the root element (for rcap)
    pub fn root_font_metrics_cap(&self) -> Length {
        self.ensure_root_font_metrics_updated();
        Length::new(f32::from_bits(
            self.root_font_metrics_cap.load(Ordering::Relaxed),
        ))
    }

    /// Set the cap-height of the root element (for rcap), in zoom-independent CSS pixels.
    pub fn set_root_font_metrics_cap(&self, size: f32) -> bool {
        let size = size.to_bits();
        let previous = self.root_font_metrics_cap.swap(size, Ordering::Relaxed);
        previous != size
    }

    /// Get the advance measure of the root element (for rch)
    pub fn root_font_metrics_ch(&self) -> Length {
        self.ensure_root_font_metrics_updated();
        Length::new(f32::from_bits(
            self.root_font_metrics_ch.load(Ordering::Relaxed),
        ))
    }

    /// Set the advance measure of the root element (for rch), in zoom-independent CSS pixels.
    pub fn set_root_font_metrics_ch(&self, size: f32) -> bool {
        let size = size.to_bits();
        let previous = self.root_font_metrics_ch.swap(size, Ordering::Relaxed);
        previous != size
    }

    /// Get the ideographic advance measure of the root element (for ric)
    pub fn root_font_metrics_ic(&self) -> Length {
        self.ensure_root_font_metrics_updated();
        Length::new(f32::from_bits(
            self.root_font_metrics_ic.load(Ordering::Relaxed),
        ))
    }

    /// Set the ideographic advance measure of the root element (for ric), in zoom-independent CSS pixels.
    pub fn set_root_font_metrics_ic(&self, size: f32) -> bool {
        let size = size.to_bits();
        let previous = self.root_font_metrics_ic.swap(size, Ordering::Relaxed);
        previous != size
    }

    fn ensure_root_font_metrics_updated(&self) {
        let mut guard = self.used_root_font_metrics.write();
        let previously_computed = mem::replace(&mut *guard, true);
        if !previously_computed {
            self.update_root_font_metrics();
        }
    }

    /// Compute the root element's font metrics, and returns a bool indicating whether
    /// the font metrics have changed since the previous restyle.
    pub fn update_root_font_metrics(&self) -> bool {
        let root_style = self.root_style.read();
        let root_effective_zoom = (*root_style).effective_zoom;
        let root_font_size = (*root_style).get_font().clone_font_size().computed_size();

        let root_font_metrics = self.query_font_metrics(
            (*root_style).writing_mode.is_upright(),
            &(*root_style).get_font(),
            root_font_size,
            QueryFontMetricsFlags::USE_USER_FONT_SET
                | QueryFontMetricsFlags::NEEDS_CH
                | QueryFontMetricsFlags::NEEDS_IC,
            /* track_usage = */ false,
        );

        let mut root_font_metrics_changed = false;
        root_font_metrics_changed |= self.set_root_font_metrics_ex(
            root_effective_zoom.unzoom(root_font_metrics.x_height_or_default(root_font_size).px()),
        );
        root_font_metrics_changed |= self.set_root_font_metrics_ch(
            root_effective_zoom.unzoom(
                root_font_metrics
                    .zero_advance_measure_or_default(
                        root_font_size,
                        (*root_style).writing_mode.is_upright(),
                    )
                    .px(),
            ),
        );
        root_font_metrics_changed |= self.set_root_font_metrics_cap(
            root_effective_zoom.unzoom(root_font_metrics.cap_height_or_default().px()),
        );
        root_font_metrics_changed |= self.set_root_font_metrics_ic(
            root_effective_zoom.unzoom(root_font_metrics.ic_width_or_default(root_font_size).px()),
        );

        root_font_metrics_changed
    }

    /// Returns whether we ever looked up the root font size of the Device.
    pub fn used_root_font_size(&self) -> bool {
        self.used_root_font_size.load(Ordering::Relaxed)
    }

    /// Returns whether we ever looked up the root line-height of the device.
    pub fn used_root_line_height(&self) -> bool {
        self.used_root_line_height.load(Ordering::Relaxed)
    }

    /// Returns whether we ever looked up the root font metrics of the device.
    pub fn used_root_font_metrics(&self) -> bool {
        *self.used_root_font_metrics.read()
    }

    /// Returns whether we ever looked up the viewport size of the Device.
    pub fn used_viewport_size(&self) -> bool {
        self.used_viewport_size.load(Ordering::Relaxed)
    }

    /// Returns whether we ever looked up the dynamic viewport size of the Device.
    pub fn used_dynamic_viewport_size(&self) -> bool {
        self.used_dynamic_viewport_size.load(Ordering::Relaxed)
    }

    /// Returns whether font metrics have been queried.
    pub fn used_font_metrics(&self) -> bool {
        self.used_font_metrics.load(Ordering::Relaxed)
    }

    /// Returns the body text color.
    pub fn body_text_color(&self) -> AbsoluteColor {
        AbsoluteColor::from_nscolor(self.body_text_color.load(Ordering::Relaxed))
    }

    /// Sets the body text color for the "inherit color from body" quirk.
    ///
    /// <https://quirks.spec.whatwg.org/#the-tables-inherit-color-from-body-quirk>
    pub fn set_body_text_color(&self, color: AbsoluteColor) {
        self.body_text_color
            .store(color.to_nscolor(), Ordering::Relaxed)
    }

    /// Applies text zoom to a font-size or line-height value (see nsStyleFont::ZoomText).
    #[inline]
    pub fn zoom_text(&self, size: Length) -> Length {
        size.scale_by(self.text_zoom())
    }

    /// Un-apply text zoom.
    #[inline]
    pub fn unzoom_text(&self, size: Length) -> Length {
        size.scale_by(1. / self.text_zoom())
    }
}
