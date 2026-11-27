use std::sync::Once;

/// Install the Stylo preferences shared by browser style and CSSOM parsing.
pub fn ensure_stylo_browser_compat_prefs() {
    static ENABLE: Once = Once::new();
    ENABLE.call_once(|| {
        stylo_static_prefs::set_pref!("layout.container-queries.enabled", true);
        stylo_static_prefs::set_pref!("layout.columns.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.at-scope.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.content.alt-text.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.font-palette.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.light-dark.images.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.margin-rules.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.progress-function.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.scroll-driven-animations.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.starting-style-at-rules.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.style-queries.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.tree-counting-functions.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.zoom.enabled", true);
        stylo_static_prefs::set_pref!("layout.grid.enabled", true);
        stylo_static_prefs::set_pref!("layout.variable_fonts.enabled", true);
        // Stylo exposes the experimental CSS Sizing `fit-content(<length>)`
        // form behind a Servo pref. Chromium 147 rejects that function for
        // width/height while still accepting the bare `fit-content` keyword
        // and grid track `fit-content()`. Keep declaration parsing, CSSOM and
        // CSS.supports on the Chromium surface; the grid parser is separate.
        stylo_static_prefs::set_pref!("layout.css.fit-content-function.enabled", false);
        // Blitz d788124a enables Stylo's omnibus Servo gate before creating
        // its Stylist. CSS masking is implemented in this pinned Stylo world
        // but remains grouped behind that gate, so paint cannot receive the
        // computed mask longhands without matching the upstream setup.
        stylo_static_prefs::set_pref!("layout.unimplemented", true);
        stylo_static_prefs::set_pref!("layout.writing-mode.enabled", true);
    });
}
