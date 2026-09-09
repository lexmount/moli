/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Gamut mapping.
//! <https://drafts.csswg.org/css-color-4/#gamut-mapping>

use super::{AbsoluteColor, ColorSpace};

/// Gamut-mapping min precision
const MIN_PRECISION: f32 = 1.0 / (i32::MAX as f32);

pub mod raytrace;

impl AbsoluteColor {
    /// 13.2.1. Binary Search Gamut Mapping with Local MINDE
    /// <https://drafts.csswg.org/css-color-4/#GMA-Binary-local-MINDE>
    pub fn gamut_map_binary_search(&self, dest_color_space: ColorSpace) -> Self {
        const MIN_L: f32 = MIN_PRECISION;
        const MAX_L: f32 = 1.0;

        // 1. if destination has no gamut limits (XYZ-D65, XYZ-D50, Lab, LCH,
        //    Oklab, OkLCh) convert origin to destination and return it as the
        //    gamut mapped color
        if matches!(
            dest_color_space,
            ColorSpace::Lab
                | ColorSpace::Lch
                | ColorSpace::Oklab
                | ColorSpace::Oklch
                | ColorSpace::XyzD50
                | ColorSpace::XyzD65
        ) {
            return self.to_color_space(dest_color_space);
        }

        // 2. let origin_OkLCh be origin converted from origin color space to
        //    the OkLCh color space
        let origin_oklch = self.to_color_space(ColorSpace::Oklch);

        // 3. if the Lightness of origin_OkLCh is greater than or equal to
        //    100%, convert `oklab(1 0 0 / origin.alpha)` to destination and
        //    return it as the gamut mapped color
        if origin_oklch.components.0 >= MAX_L {
            return AbsoluteColor::new(ColorSpace::Oklab, 1.0, 0.0, 0.0, self.alpha)
                .to_color_space(dest_color_space);
        }

        // 4. if the Lightness of origin_OkLCh is less than than or equal to
        //    0%, convert `oklab(0 0 0 / origin.alpha)` to destination and
        //    return it as the gamut mapped color
        if origin_oklch.components.0 <= MIN_L {
            return AbsoluteColor::new(ColorSpace::Oklab, 0.0, 0.0, 0.0, self.alpha)
                .to_color_space(dest_color_space);
        }

        // 5. let inGamut(color) be a function which returns true if, when
        //    passed a color, that color is inside the gamut of destination.
        //    For HSL and HWB, it returns true if the color is inside the
        //    gamut of sRGB.
        //    See [`Self::in_gamut`] and [`Self::in_gamut_for_dest_space`].

        // 6. if inGamut(origin_OkLCh) is true, convert origin_OkLCh to
        //    destination and return it as the gamut mapped color
        if origin_oklch.in_gamut_for_dest_space(dest_color_space) {
            return origin_oklch.to_color_space(dest_color_space);
        }

        // 7. otherwise, let delta(one, two) be a function which returns the
        //    deltaEOK of color one compared to color two
        //    See the [`delta_eok`] function.

        // 8. let JND be 0.02
        const JND: f32 = 0.02;

        // 9. let epsilon be 0.0001
        const EPSILON: f32 = 0.0001;

        // 10. let clip(color) be a function which converts color to
        //     destination, clamps each component to the bounds of the
        //     reference range for that component, and returns the result.
        //     See [`Self::clip`] and [`Self::clip_to_dest_space`].

        // 11. set current to origin_OkLCh
        let mut current_oklch = origin_oklch.clone();

        // 12. set clipped to clip(current)
        let mut clipped = current_oklch.clip_to_dest_space(dest_color_space);

        // 13. set E to delta(clipped, current)
        let mut e = delta_eok(&clipped, &current_oklch);

        // 14. if E < JND, return clipped as the gamut mapped color
        if e < JND {
            return clipped;
        }

        // 15. set min to zero
        let mut min = 0.0;

        // 16. set max to the Oklch chroma of origin_Oklch.
        let mut max = origin_oklch.components.1;

        // 17. let min_inGamut be a boolean that represents when min is still
        //     in gamut, and set it to true
        let mut min_in_gamut = true;

        // 18. while (max - min is greater than epsilon) repeat the following
        //     steps.
        while max - min > EPSILON {
            // 18.1. set chroma to (min + max) / 2
            let chroma = (min + max) / 2.0;

            // 18.2. set the chroma component of current to chroma
            current_oklch.components.1 = chroma;

            // 18.3. if min_inGamut is true and also if inGamut(current) is
            //       true, set min to chroma and continue to repeat these steps.
            if min_in_gamut && current_oklch.in_gamut_for_dest_space(dest_color_space) {
                min = chroma;
                continue;
            }

            // 18.4. otherwise, if inGamut(current) is false carry out these
            //       steps:
            // 18.4.1. set clipped to clip(current)
            clipped = current_oklch.clip_to_dest_space(dest_color_space);

            // 18.4.2. set E to delta(clipped, current)
            e = delta_eok(&clipped, &current_oklch);

            // 18.4.3. if E < JND
            if e < JND {
                // 18.4.3.1. if (JND - E < epsilon), return clipped as the
                //           gamut mapped color
                if JND - e < EPSILON {
                    return clipped;
                }

                // 18.4.3.2. otherwise:
                // 18.4.3.2.1. set min_inGamut to false
                min_in_gamut = false;

                // 18.4.3.2.2. set min to chroma
                min = chroma;
            } else {
                // 18.4.4. otherwise, set max to chroma and continue to repeat
                //         these steps
                max = chroma;
            }
        }

        // 19. return clipped as the gamut mapped color
        clipped
    }

    /// Clamp this color to within the [0..1] range.
    /// NOTE this assumes RGB ranges and will not work for Lab, Oklab, or
    ///      other color spaces with different ranges, or limitless ranges
    fn clip(&self) -> Self {
        let mut result = self.clone();
        result.components = result.components.map(|c| c.clamp(0.0, 1.0));
        result
    }

    /// ^10. let clip(color) be a function which converts color to destination,
    ///      clamps each component to the bounds of the reference range for
    ///      that component, and returns the result
    /// Clip/clamp this color to the supplied destination color space
    fn clip_to_dest_space(&self, dest_color_space: ColorSpace) -> Self {
        self.to_color_space(dest_color_space).clip()
    }

    /// Returns true if this color is within its gamut limits.
    fn in_gamut(&self) -> bool {
        macro_rules! in_range {
            ($c:expr) => {{
                $c >= MIN_PRECISION && $c <= 1.0
            }};
        }

        match self.color_space {
            ColorSpace::Hsl | ColorSpace::Hwb => self.to_color_space(ColorSpace::Srgb).in_gamut(),
            ColorSpace::Srgb
            | ColorSpace::SrgbLinear
            | ColorSpace::DisplayP3
            | ColorSpace::DisplayP3Linear
            | ColorSpace::A98Rgb
            | ColorSpace::ProphotoRgb
            | ColorSpace::Rec2020 => {
                in_range!(self.components.0)
                    && in_range!(self.components.1)
                    && in_range!(self.components.2)
            },
            ColorSpace::Lab
            | ColorSpace::Lch
            | ColorSpace::Oklab
            | ColorSpace::Oklch
            | ColorSpace::XyzD50
            | ColorSpace::XyzD65 => true,
        }
    }

    /// ^5. let inGamut(color) be a function which returns true if, when passed
    ///     a color, that color is inside the gamut of destination. For HSL and
    ///     HWB, it returns true if the color is inside the gamut of sRGB.
    /// Check if this color is in-gamut for the destination color space
    fn in_gamut_for_dest_space(&self, dest_color_space: ColorSpace) -> bool {
        if self.color_space == ColorSpace::Hsl || self.color_space == ColorSpace::Hwb {
            self.to_color_space(ColorSpace::Srgb).in_gamut()
        } else {
            self.to_color_space(dest_color_space).in_gamut()
        }
    }
}

/// Calculate deltaE OK (simple root sum of squares).
/// <https://drafts.csswg.org/css-color-4/#color-difference-OK>
fn delta_eok(reference: &AbsoluteColor, sample: &AbsoluteColor) -> f32 {
    // Delta is calculated in the oklab color space.
    let reference = reference.to_color_space(ColorSpace::Oklab);
    let sample = sample.to_color_space(ColorSpace::Oklab);

    let diff = reference.components - sample.components;
    let diff = diff * diff;
    (diff.0 + diff.1 + diff.2).sqrt()
}
