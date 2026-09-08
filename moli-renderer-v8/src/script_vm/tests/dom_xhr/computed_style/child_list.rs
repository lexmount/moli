use super::*;

#[test]
fn removed_source_with_moved_old_neighbor_invalidates_remaining_target() {
    let mut vm = new_parsed_test_vm(
        "https://moved-old-neighbor.test/",
        r#"<!doctype html><style>
          .target { color: blue; }
          .source ~ .target { color: red; }
        </style><div id="old"><span id="source" class="source">A</span><span id="neighbor">B</span><span id="target" class="target">C</span></div><div id="new"></div>"#,
    );
    let result = vm
        .eval(
            r#"(() => {
              const oldParent = document.getElementById('old');
              const newParent = document.getElementById('new');
              const held = getComputedStyle(document.getElementById('target'));
              const values = [held.color];
              document.getElementById('source').remove();
              newParent.appendChild(document.getElementById('neighbor'));
              values.push(oldParent.innerText, newParent.innerText, held.color);
              return values.join('|');
            })()"#,
        )
        .expect("moving an old neighbor must not hide the remaining affected siblings");
    assert_eq!(result, "rgb(255, 0, 0)|C|B|rgb(0, 0, 255)");
}

#[test]
fn sibling_move_between_shadow_roots_invalidates_both_style_scopes() {
    let mut vm = new_parsed_test_vm(
        "https://cross-shadow-sibling-move.test/",
        "<!doctype html><div id=old></div><div id=new></div>",
    );
    let result = vm
        .eval(
            r#"(() => {
              const oldRoot = document.getElementById('old').attachShadow({mode:'open'});
              const newRoot = document.getElementById('new').attachShadow({mode:'open'});
              const css = '<style>.target { color: blue; } .source ~ .target { color: red; }</style>';
              oldRoot.innerHTML = css + '<span class=source>A</span><span class=target>B</span>';
              newRoot.innerHTML = css + '<span class=target>C</span>';
              const oldTarget = oldRoot.querySelector('.target');
              const newTarget = newRoot.querySelector('.target');
              const oldStyle = getComputedStyle(oldTarget);
              const newStyle = getComputedStyle(newTarget);
              const values = [oldStyle.color, newStyle.color];
              newRoot.insertBefore(oldRoot.querySelector('.source'), newTarget);
              values.push(oldTarget.innerText, newTarget.innerText, oldStyle.color, newStyle.color);
              return values.join('|');
            })()"#,
        )
        .expect("source-scope fallback must include the old and new shadow trees");
    assert_eq!(
        result,
        "rgb(255, 0, 0)|rgb(0, 0, 255)|B|C|rgb(0, 0, 255)|rgb(255, 0, 0)"
    );
}

#[test]
fn detached_then_reinserted_sibling_does_not_reuse_queued_old_links() {
    let mut vm = new_parsed_test_vm(
        "https://two-step-sibling-move.test/",
        r#"<!doctype html><style>
          .target { color: blue; }
          .source ~ .target { color: red; }
        </style><div id="container"><span id="source" class="source">A</span><span id="target" class="target">B</span></div>"#,
    );
    let result = vm
        .eval(
            r#"(() => {
              const container = document.getElementById('container');
              const source = document.getElementById('source');
              const held = getComputedStyle(document.getElementById('target'));
              const values = [held.color];
              source.remove();
              container.appendChild(source);
              values.push(container.innerText, held.color);
              return values.join('|');
            })()"#,
        )
        .expect("separate remove and insert calls must validate topology together at drain time");
    assert_eq!(result, "rgb(255, 0, 0)|BA|rgb(0, 0, 255)");
}

#[test]
fn inner_text_and_held_style_update_after_sibling_reorder() {
    let mut vm = new_parsed_test_vm(
        "https://sibling-reorder.test/",
        r#"<!doctype html><style>
          .target { color: blue; }
          .source ~ .target { color: red; }
        </style><div id="container"><span id="source" class="source">A</span><span id="target" class="target">B</span></div>"#,
    );
    let result = vm
        .eval(
            r#"(() => {
              const container = document.getElementById('container');
              const source = document.getElementById('source');
              const held = getComputedStyle(document.getElementById('target'));
              const values = [held.color];
              container.appendChild(source);
              values.push(container.innerText, held.color);
              container.prepend(source);
              values.push(container.innerText, held.color);
              return values.join('|');
            })()"#,
        )
        .expect("innerText must finish after a sibling move and preserve held style liveness");
    assert_eq!(result, "rgb(255, 0, 0)|BA|rgb(0, 0, 255)|AB|rgb(255, 0, 0)");
}

#[test]
fn relative_selector_updates_after_backward_sibling_reorder() {
    let mut vm = new_parsed_test_vm(
        "https://backward-sibling-reorder.test/",
        r#"<!doctype html><style>
          .source { color: blue; }
          .source:has(~ .target) { color: red; }
        </style><div id="container"><span id="source" class="source">A</span><span id="target" class="target">B</span></div>"#,
    );
    let result = vm
        .eval(
            r#"(() => {
              const container = document.getElementById('container');
              const target = document.getElementById('target');
              const held = getComputedStyle(document.getElementById('source'));
              const values = [held.color];
              container.prepend(target);
              values.push(container.innerText, held.color);
              container.appendChild(target);
              values.push(container.innerText, held.color);
              return values.join('|');
            })()"#,
        )
        .expect("relative invalidation must finish after reversing a sibling relation");
    assert_eq!(result, "rgb(255, 0, 0)|BA|rgb(0, 0, 255)|AB|rgb(255, 0, 0)");
}

#[test]
fn sibling_move_between_parents_invalidates_old_and_new_targets() {
    let mut vm = new_parsed_test_vm(
        "https://cross-parent-sibling-move.test/",
        r#"<!doctype html><style>
          .target { color: blue; }
          .source ~ .target { color: red; }
        </style>
        <div id="old"><span id="source" class="source">A</span><span id="old-target" class="target">B</span></div>
        <div id="new"><span id="new-target" class="target">C</span></div>"#,
    );
    let result = vm
        .eval(
            r#"(() => {
              const oldParent = document.getElementById('old');
              const newParent = document.getElementById('new');
              const source = document.getElementById('source');
              const oldStyle = getComputedStyle(document.getElementById('old-target'));
              const newStyle = getComputedStyle(document.getElementById('new-target'));
              const values = [oldStyle.color, newStyle.color];
              newParent.prepend(source);
              values.push(oldParent.innerText, newParent.innerText, oldStyle.color, newStyle.color);
              return values.join('|');
            })()"#,
        )
        .expect("moving a source must invalidate both sibling regions");
    assert_eq!(
        result,
        "rgb(255, 0, 0)|rgb(0, 0, 255)|B|AC|rgb(0, 0, 255)|rgb(255, 0, 0)"
    );
}

#[test]
fn batched_sibling_reorders_update_adjacent_and_general_sibling_styles() {
    let mut vm = new_parsed_test_vm(
        "https://batched-sibling-reorder.test/",
        r#"<!doctype html><style>
          .target { color: blue; background-color: white; }
          .source ~ .target { color: red; }
          .source + .target { background-color: black; }
        </style><div id="container"><span id="source" class="source">A</span><span id="target" class="target">B</span><span id="other">C</span></div>"#,
    );
    let result = vm
        .eval(
            r#"(() => {
              const container = document.getElementById('container');
              const source = document.getElementById('source');
              const other = document.getElementById('other');
              const held = getComputedStyle(document.getElementById('target'));
              const values = [held.color, held.backgroundColor];
              container.appendChild(source);
              container.prepend(other);
              values.push(container.innerText, held.color, held.backgroundColor);
              container.prepend(source);
              values.push(container.innerText, held.color, held.backgroundColor);
              return values.join('|');
            })()"#,
        )
        .expect("batched moves must not retain obsolete sibling edges or computed styles");
    assert_eq!(
        result,
        "rgb(255, 0, 0)|rgb(0, 0, 0)|CBA|rgb(0, 0, 255)|rgb(255, 255, 255)|ACB|rgb(255, 0, 0)|rgb(255, 255, 255)"
    );
}
