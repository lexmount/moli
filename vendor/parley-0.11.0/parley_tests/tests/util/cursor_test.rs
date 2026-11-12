// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use peniko::Color;
use vello_cpu::Pixmap;

use super::renderer::{ColorBrush, RenderingConfig, draw_layout, render_to_pixmap};
use parley::{Affinity, Cursor, FontContext, Layout, LayoutContext};

// Note: This module is only compiled when running tests, which requires std,
// so we don't have to worry about being no_std-compatible.

/// Helper struct for creating cursors and checking their values.
///
/// This type implements multiple assertion methods which, on failure, will
/// print the input text with cursor's expected and actual positions highlighted.
/// This should make test failures more readable than printing the cursor's byte index.
///
/// The following are not supported:
///
/// - RTL text.
/// - Multi-line text.
/// - Any character that doesn't span a single terminal tile.
/// - Multi-bytes characters.
///
/// Some of these limitations are inherent to visually displaying a text layout in the
/// terminal.
///
/// Others will be fixed in the future.
///
/// # Writing tests with Parley
///
/// This API enables users to write tests for cursor values where the intent of
/// the test is obvious from the code of the test alone.
///
/// In general, Parley tries to encourage users to write this style of test.
/// Users should avoid tests where you compare the cursor to a numeric value
/// (mapping a numeric value to a cursor position is not obvious) and
/// screenshot tests (readers shouldn't need to open a screenshot file).
pub(crate) struct CursorTest {
    text: String,
    layout: Layout<ColorBrush>,
}

const CURSOR_WIDTH: f32 = 2.0;

impl CursorTest {
    pub(crate) fn single_line(
        text: &str,
        lcx: &mut LayoutContext<ColorBrush>,
        fcx: &mut FontContext,
    ) -> Self {
        let builder = lcx.ranged_builder(fcx, text, 1.0, true);
        let mut layout = builder.build(text);
        layout.break_all_lines(None);

        // NOTE: If we want to handle more special cases, we may want to use a monospace
        // font and use the glyph advance values to calculate the cursor position.

        Self {
            text: text.to_string(),
            layout,
        }
    }

    #[allow(dead_code, reason = "this will be used for the editor tests later")]
    /// Returns the text that was used to create the layout.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Returns the layout that was created from the text.
    pub(crate) fn layout(&self) -> &Layout<ColorBrush> {
        &self.layout
    }

    #[track_caller]
    fn get_unique_index(&self, method_name: &str, needle: &str) -> usize {
        let Some(index) = self.text.find(needle) else {
            panic!(
                "Error in {method_name}: needle '{needle}' not found in text '{}'",
                self.text
            );
        };
        if self.text[index + needle.len()..].contains(needle) {
            panic!(
                "Error in {method_name}: needle '{needle}' found multiple times in text '{}'",
                self.text
            );
        }
        index
    }

    /// Returns a cursor that points to the first character of the needle, with
    /// [`Affinity::Downstream`].
    ///
    /// The needle must be unique in the text to avoid ambiguity.
    ///
    /// # Panics
    ///
    /// - If the needle is not found in the text.
    /// - If the needle is found multiple times in the text.
    #[track_caller]
    pub(crate) fn cursor_before(&self, needle: &str) -> Cursor {
        let index = self.get_unique_index("cursor_before", needle);
        Cursor::from_byte_index(&self.layout, index, Affinity::Downstream)
    }

    /// Returns a cursor that points to the first character after the needle, with
    /// [`Affinity::Upstream`].
    ///
    /// The needle must be unique in the text to avoid ambiguity.
    ///
    /// # Panics
    ///
    /// - If the needle is not found in the text.
    /// - If the needle is found multiple times in the text.
    #[track_caller]
    pub(crate) fn cursor_after(&self, needle: &str) -> Cursor {
        let index = self.get_unique_index("cursor_after", needle);
        let index = index + needle.len();
        Cursor::from_byte_index(&self.layout, index, Affinity::Upstream)
    }

    fn cursor_to_monospace(&self, cursor: Cursor, is_correct: bool) -> String {
        fn check_no_color() -> bool {
            let Some(env_var) = std::env::var_os("NO_COLOR") else {
                return false;
            };
            let env_var = env_var.to_str().unwrap_or_default().trim();

            if env_var == "0" {
                return false;
            }
            if env_var.eq_ignore_ascii_case("false") {
                return false;
            }
            true
        }

        // NOTE: The background color doesn't carry important information,
        // so we do a simple implementation, without worrying about
        // color-blindness and platform issues.
        let ansi_bg_color = if cfg!(not(unix)) || check_no_color() {
            ""
        } else if is_correct {
            // Green background
            "\x1b[48;5;70m"
        } else {
            // Red background
            "\x1b[48;5;160m"
        };
        let ansi_reset = if cfg!(not(unix)) { "" } else { "\x1b[0m" };
        let index = cursor.index();
        let affinity = cursor.affinity();

        let cursor_str = if affinity == Affinity::Upstream {
            // - ANSI code for 'Set background color'
            // - Unicode sequence for '▕' character
            // - ANSI code for 'Reset all attributes'
            format!("{ansi_bg_color}\u{2595}{ansi_reset}")
        } else {
            // - 1 space
            // - ANSI code for 'Set background color'
            // - Unicode sequence for '▏' character
            // - ANSI code for 'Reset all attributes'
            format!(" {ansi_bg_color}\u{258F}{ansi_reset}")
        };

        // FIXME - This assumes that the byte index of a string matches how many
        // terminal tiles that string occupies. This is wrong for even trivial
        // cases (eg unicode characters spanning multiple code points).
        " ".repeat(index) + &cursor_str
    }

    #[track_caller]
    fn cursor_assertion(&self, expected: Cursor, actual: Cursor) {
        if expected == actual {
            return;
        }

        // TODO - Check that the tested string doesn't include difficult
        // characters (newlines, tabs, RTL text, etc.)
        // If it does, we should still print the text on a best effort basis, but
        // without visual cursors and with a warning that the text may not be accurate.

        let bg_color_expected = Color::from_rgba8(255, 255, 255, 255);
        let padding_color_expected = Color::from_rgba8(166, 200, 255, 255);
        let cursor_color_expected = Color::from_rgba8(0, 0, 255, 255);
        let selection_colors_expected = [
            Color::from_rgba8(0, 255, 0, 255),
            Color::from_rgba8(255, 0, 0, 255),
        ];
        let bg_color_actual = Color::from_rgba8(230, 230, 230, 255);
        let padding_color_actual = Color::from_rgba8(166, 255, 240, 255);
        let cursor_color_actual = Color::from_rgba8(0, 0, 128, 255);
        let selection_colors_actual = [
            Color::from_rgba8(0, 128, 0, 255),
            Color::from_rgba8(128, 0, 0, 255),
        ];

        let rendering_config_expected = RenderingConfig {
            background_color: bg_color_expected,
            padding_color: padding_color_expected,
            inline_box_color: bg_color_expected,
            cursor_color: cursor_color_expected,
            selection_colors: selection_colors_expected,
            size: None,
            scale: 1.0,
            hint: false,
            glyph_transform: None,
        };
        let rendering_config_actual = RenderingConfig {
            background_color: bg_color_actual,
            padding_color: padding_color_actual,
            inline_box_color: bg_color_actual,
            cursor_color: cursor_color_actual,
            selection_colors: selection_colors_actual,
            size: None,
            scale: 1.0,
            hint: false,
            glyph_transform: None,
        };

        let rect_expected = expected.geometry(&self.layout, CURSOR_WIDTH);
        let rect_actual = actual.geometry(&self.layout, CURSOR_WIDTH);

        let img_expected = render_to_pixmap(draw_layout(
            &rendering_config_expected,
            &self.layout,
            Some(rect_expected),
            &[],
        ));
        let img_actual = render_to_pixmap(draw_layout(
            &rendering_config_actual,
            &self.layout,
            Some(rect_actual),
            &[],
        ));

        assert_eq!(
            img_expected.width(),
            img_actual.width(),
            "expected and actual snapshot dimensions match"
        );
        assert_eq!(
            img_expected.height(),
            img_actual.height(),
            "expected and actual snapshot dimensions match"
        );

        let mut full_img = Pixmap::new(img_actual.width(), img_actual.height() * 2);
        // vello_cpu currently doesn't have a simple "blit pixmap with no scaling" API, so just copy the data to get the
        // expected and actual images in a vertical stack
        let num_pixels = img_actual.width() as usize * img_actual.height() as usize;
        full_img.data_mut()[..num_pixels].copy_from_slice(img_expected.data());
        full_img.data_mut()[num_pixels..].copy_from_slice(img_actual.data());

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let screenshot_path = std::env::temp_dir().join(format!("parley-{timestamp:016}.png"));

        // TODO - If possible, display the image in the terminal using Kitty Image Protocol
        // https://sw.kovidgoyal.net/kitty/graphics-protocol/
        // (probably with kitty_image crate)
        std::fs::write(&screenshot_path, full_img.into_png().unwrap()).unwrap();

        panic!(
            concat!(
                "-----------\n",
                "cursor assertion failed\n",
                "  expected: '{text}' - ({expected_index}, {expected_affinity:?})\n",
                "            {expected_cursor}\n",
                "       got: '{text}' - ({actual_index}, {actual_affinity:?})\n",
                "            {actual_cursor}\n",
                "screenshot saved in '{screenshot_path}'\n",
                "-----------\n",
            ),
            text = self.text,
            expected_index = expected.index(),
            expected_affinity = expected.affinity(),
            actual_index = actual.index(),
            actual_affinity = actual.affinity(),
            expected_cursor = self.cursor_to_monospace(expected, true),
            actual_cursor = self.cursor_to_monospace(actual, false),
            screenshot_path = screenshot_path.display(),
        );
    }

    /// Asserts that the cursor is before the needle.
    ///
    /// The needle must be unique in the text to avoid ambiguity.
    ///
    /// # Panics
    ///
    /// - If the needle is not found in the text.
    /// - If the needle is found multiple times in the text.
    /// - If the cursor has the wrong position.
    /// - If the cursor doesn't have [`Affinity::Downstream`].
    #[track_caller]
    pub(crate) fn assert_cursor_is_before(&self, needle: &str, cursor: Cursor) {
        let index = self.get_unique_index("assert_cursor_is_before", needle);

        let expected_cursor = Cursor::from_byte_index(&self.layout, index, Affinity::Downstream);
        self.cursor_assertion(expected_cursor, cursor);
    }

    /// Asserts that the cursor is after the needle.
    ///
    /// The needle must be unique in the text to avoid ambiguity.
    ///
    /// # Panics
    ///
    /// - If the needle is not found in the text.
    /// - If the needle is found multiple times in the text.
    /// - If the cursor has the wrong position.
    /// - If the cursor doesn't have [`Affinity::Upstream`].
    #[track_caller]
    pub(crate) fn assert_cursor_is_after(&self, needle: &str, cursor: Cursor) {
        let index = self.get_unique_index("assert_cursor_is_after", needle);
        let index = index + needle.len();

        let expected_cursor = Cursor::from_byte_index(&self.layout, index, Affinity::Upstream);
        self.cursor_assertion(expected_cursor, cursor);
    }

    /// Compares two cursors and asserts that they are the same.
    ///
    /// # Panics
    ///
    /// - If the cursors don't have the same index.
    /// - If the cursors don't have the same affinity.
    #[allow(dead_code, reason = "this will be used for the editor tests later")]
    #[track_caller]
    pub(crate) fn assert_cursor_is(&self, expected: Cursor, cursor: Cursor) {
        self.cursor_assertion(expected, cursor);
    }

    /// Prints the text this object was created with, with the cursor highlighted.
    ///
    /// Uses the same format as assertion failures.
    #[track_caller]
    pub(crate) fn print_cursor(&self, cursor: Cursor) {
        eprintln!(
            concat!(
                "dumping test layout value\n",
                "      text: '{text}' - ({actual_index}, {actual_affinity:?})\n",
                "            {actual_cursor}\n",
            ),
            text = self.text,
            actual_index = cursor.index(),
            actual_affinity = cursor.affinity(),
            actual_cursor = self.cursor_to_monospace(cursor, true),
        );
    }

    /// Renders the text this object was created with, with the cursor highlighted, and saves it to a temp file.
    ///
    /// Uses the same visual format as assertion failures.
    #[track_caller]
    #[allow(dead_code, reason = "this will be used for the editor tests later")]
    pub(crate) fn render_cursor(&self, cursor: Cursor) {
        let background_color = Color::from_rgba8(255, 255, 255, 255);
        let padding_color = Color::from_rgba8(166, 200, 255, 255);
        let inline_box_color = Color::from_rgba8(0, 0, 0, 255);
        let cursor_color = Color::from_rgba8(0, 0, 255, 255);
        let selection_colors = [
            Color::from_rgba8(0, 255, 0, 255),
            Color::from_rgba8(255, 0, 0, 255),
        ];

        let rendering_config_cursor = RenderingConfig {
            background_color,
            padding_color,
            inline_box_color,
            cursor_color,
            selection_colors,
            size: None,
            scale: 1.0,
            hint: false,
            glyph_transform: None,
        };

        let rect_cursor = cursor.geometry(&self.layout, CURSOR_WIDTH);

        let img_cursor = render_to_pixmap(draw_layout(
            &rendering_config_cursor,
            &self.layout,
            Some(rect_cursor),
            &[],
        ));

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let screenshot_path = std::env::temp_dir().join(format!("parley-{timestamp:016}.png"));

        // TODO - If possible, display the image in the terminal,
        // see comment in cursor_assertion
        std::fs::write(&screenshot_path, img_cursor.into_png().unwrap()).unwrap();

        eprintln!(
            "screenshot saved in '{screenshot_path}'\n",
            screenshot_path = screenshot_path.display(),
        );
    }
}
