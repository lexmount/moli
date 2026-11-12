// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `PlainEditor` tests.

use crate::test_name;
use crate::util::TestEnv;
use parley::Affinity;

// TODO - Use CursorTest API for these tests

#[test]
fn editor_simple_move() {
    let mut env = TestEnv::new(test_name!(), None);
    let mut editor = env.editor("Hi, all!\nNext");
    env.check_editor_snapshot(&mut editor);
    let mut drv = env.driver(&mut editor);
    drv.move_right();
    drv.move_right();
    drv.move_right();

    env.check_editor_snapshot(&mut editor);
    env.driver(&mut editor).move_down();
    env.check_editor_snapshot(&mut editor);
    env.driver(&mut editor).move_left();
    env.check_editor_snapshot(&mut editor);
    env.driver(&mut editor).move_up();
    env.check_editor_snapshot(&mut editor);
}

#[test]
fn editor_select_all() {
    let mut env = TestEnv::new(test_name!(), None);
    let mut editor = env.editor("Hi, all!\nNext");
    env.driver(&mut editor).select_all();
    env.check_editor_snapshot(&mut editor);
}

#[test]
fn editor_select_hard_line() {
    let mut env = TestEnv::new(test_name!(), None);
    let mut editor = env.editor("First\nNew Hard Line with soft break!\nLast");
    editor.set_width(Some(40.));
    env.driver(&mut editor).move_right();
    // We can select the first line.
    env.driver(&mut editor).select_to_hard_line_end();
    env.check_editor_snapshot(&mut editor);
    env.driver(&mut editor).move_to_hard_line_start();
    env.check_editor_snapshot(&mut editor);
    env.driver(&mut editor).move_down();
    env.driver(&mut editor).move_to_hard_line_end();
    env.check_editor_snapshot(&mut editor);
    env.driver(&mut editor).select_to_hard_line_start();
    env.check_editor_snapshot(&mut editor);
    env.driver(&mut editor).move_right();
    // Cursor is logically after the newline; there's not really any great answer here.
    env.driver(&mut editor).select_to_hard_line_start();
    env.check_editor_snapshot(&mut editor);

    // We can select the last line.
    env.driver(&mut editor).move_right();
    env.driver(&mut editor).move_right();
    env.driver(&mut editor).move_to_hard_line_end();
    env.check_editor_snapshot(&mut editor);
    env.driver(&mut editor).select_to_hard_line_start();
    env.check_editor_snapshot(&mut editor);
}

#[test]
fn editor_double_newline() {
    let mut env = TestEnv::new(test_name!(), None);
    let mut editor = env.editor("Hi, all!\n\nNext");
    env.driver(&mut editor).select_all();
    env.check_editor_snapshot(&mut editor);
}

#[test]
fn editor_insert_line_endings_set_downstream_affinity() {
    let mut env = TestEnv::new(test_name!(), None);

    for (insert, expected_text) in [
        ("\n", "A\nB"),
        ("\r", "A\rB"),
        ("\u{2028}", "A\u{2028}B"),
        ("\u{2029}", "A\u{2029}B"),
        ("X\n", "AX\nB"),
        ("X\r", "AX\rB"),
        ("X\u{2028}", "AX\u{2028}B"),
        ("X\u{2029}", "AX\u{2029}B"),
    ] {
        let mut editor = env.editor("AB");
        env.driver(&mut editor).move_right(); // between A and B
        env.driver(&mut editor).insert_or_replace_selection(insert);

        assert_eq!(editor.raw_text(), expected_text);

        let sel = editor.raw_selection();
        assert!(sel.is_collapsed());
        assert_eq!(sel.focus().index(), expected_text.len() - 1);
        assert_eq!(sel.focus().affinity(), Affinity::Downstream);
    }
}

#[test]
fn editor_insert_regular_text_set_upstream_affinity() {
    let mut env = TestEnv::new(test_name!(), None);

    let mut editor = env.editor("AB");
    env.driver(&mut editor).move_right(); // between A and B
    env.driver(&mut editor).insert_or_replace_selection("X");

    assert_eq!(editor.raw_text(), "AXB");

    let sel = editor.raw_selection();
    assert!(sel.is_collapsed());
    assert_eq!(sel.focus().index(), 2);
    assert_eq!(sel.focus().affinity(), Affinity::Upstream);
}

#[test]
fn editor_set_text_clamps_selection_to_shorter_text() {
    let mut env = TestEnv::new(test_name!(), None);
    let mut editor = env.editor("hello world");
    env.driver(&mut editor).move_to_text_end();
    editor.set_text("");
    env.driver(&mut editor).insert_or_replace_selection("x");
    assert_eq!(editor.text(), "x");
}

#[test]
fn editor_set_text_clamps_selection_to_char_boundary() {
    let mut env = TestEnv::new(test_name!(), None);
    let mut editor = env.editor("ab");
    env.driver(&mut editor).move_right();
    // The cursor at byte 1 is inside the two-byte "é".
    editor.set_text("é");
    env.driver(&mut editor).insert_or_replace_selection("x");
    assert_eq!(editor.text(), "xé");
}
