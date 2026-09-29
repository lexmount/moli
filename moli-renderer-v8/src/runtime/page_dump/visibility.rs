use std::collections::{HashMap, HashSet};

use moli_html2md::{Dom, NodeKind};
use url::Url;

use crate::dom::native::{NativeDom, NativeNodeId};

/// Read the current page's computed visibility without mutating the live DOM.
/// Explicit disclosure content remains useful in a document dump, even while
/// its tab or section is closed. Other non-rendered UI is not document text.
pub(super) struct MarkdownDom<'a> {
    dom: &'a NativeDom,
    suppressed: HashSet<NativeNodeId>,
    invisible: HashSet<NativeNodeId>,
    blocks: HashSet<NativeNodeId>,
    inline_boundaries: HashSet<NativeNodeId>,
    superscripts: HashSet<NativeNodeId>,
    subscripts: HashSet<NativeNodeId>,
    resolved_urls: HashMap<NativeNodeId, HashMap<String, String>>,
    unchecked_controls: HashSet<NativeNodeId>,
    fragment_links: bool,
}

impl<'a> MarkdownDom<'a> {
    pub(super) fn new(
        dom: &'a NativeDom,
        styles: impl IntoIterator<Item = (NativeNodeId, Vec<String>)>,
        base_url: Option<&Url>,
        final_opacity_animations: HashSet<String>,
    ) -> Self {
        let styles: Vec<_> = styles.into_iter().collect();
        let document_element = dom.document_element_node_id();
        let body = dom.body_node_id();
        let document_visibility_is_hidden = styles.iter().any(|(node, values)| {
            (Some(*node) == document_element || Some(*node) == body)
                && values
                    .get(1)
                    .is_some_and(|value| matches!(value.as_str(), "hidden" | "collapse"))
        });
        let fragment_links = styles.iter().any(|(node, _)| {
            Dom::attribute(dom, *node, "href").is_some_and(|href| href.starts_with('#'))
        });
        // `styles` follows the document's preorder, so every element's parent
        // has already been seen. Cache the painted ancestor background once;
        // walking ancestors for every node is quadratic on deeply nested pages.
        let mut effective_backgrounds = HashMap::new();
        let mut non_solid_backgrounds = HashSet::new();
        if styles
            .iter()
            .any(|(_, values)| values.get(11).is_some_and(|value| !value.is_empty()))
        {
            for (node, values) in &styles {
                let own_color = values.get(11).and_then(|value| css_color(value));
                let own = own_color.filter(|color| color.3 > 0.99);
                let inherited = dom
                    .parent_node(*node)
                    .and_then(|parent| effective_backgrounds.get(&parent).copied());
                effective_backgrounds
                    .insert(*node, own.or(inherited).unwrap_or((255, 255, 255, 1.0)));
                let own_image = values
                    .get(13)
                    .is_some_and(|image| !image.is_empty() && image != "none");
                let inherited_image = own.is_none()
                    && dom
                        .parent_node(*node)
                        .is_some_and(|parent| non_solid_backgrounds.contains(&parent));
                let translucent = own_color.is_some_and(|color| color.3 > 0.0 && color.3 <= 0.99);
                if own_image || inherited_image || translucent {
                    non_solid_backgrounds.insert(*node);
                }
            }
        }
        let mut disclosures = HashSet::new();
        let mut excerpts = HashSet::new();
        let targets: HashMap<_, _> = styles
            .iter()
            .filter_map(|(node, _)| Dom::attribute(dom, *node, "id").map(|id| (id, *node)))
            .collect();
        let hidden_targets: HashMap<_, _> = styles
            .iter()
            .filter_map(|(node, values)| {
                values
                    .first()
                    .is_some_and(|value| value == "none")
                    .then(|| Dom::attribute(dom, *node, "id").map(|id| (id, *node)))
                    .flatten()
            })
            .collect();
        let visible_handler_functions = visible_handler_functions(dom);
        for (node, _) in &styles {
            if (Dom::attribute(dom, *node, "aria-expanded").is_some()
                || Dom::attribute(dom, *node, "role")
                    .is_some_and(|role| role.eq_ignore_ascii_case("tab")))
                && let Some(controlled) = Dom::attribute(dom, *node, "aria-controls")
            {
                disclosures.extend(
                    controlled
                        .split_ascii_whitespace()
                        .filter(|id| target_is_disclosure(dom, &targets, id)),
                );
            }
            for attribute_name in dom.get_attribute_names(*node).unwrap_or_default() {
                if !attribute_name.starts_with("on") {
                    continue;
                }
                let Some(handler) = Dom::attribute(dom, *node, &attribute_name) else {
                    continue;
                };
                for target in inline_disclosure_targets(handler, &visible_handler_functions) {
                    if target_is_disclosure(dom, &targets, target) {
                        disclosures.insert(target);
                    }
                }
            }
            // Some pages pair a shortened paragraph with an explicitly linked
            // hidden full-text copy. Keep the complete copy once; an unrelated
            // dialog or a matching paragraph elsewhere is not such a pair.
            for attribute in ["href", "data-src", "data-target"] {
                let Some(id) =
                    Dom::attribute(dom, *node, attribute).and_then(|value| value.strip_prefix('#'))
                else {
                    continue;
                };
                let Some(&target) = hidden_targets.get(id) else {
                    continue;
                };
                if Dom::attribute(dom, target, "role").is_some_and(|role| {
                    ["dialog", "alertdialog", "menu"]
                        .iter()
                        .any(|excluded| role.eq_ignore_ascii_case(excluded))
                }) {
                    continue;
                }
                let mut ancestor = dom.parent_node(*node);
                while let Some(paragraph) = ancestor {
                    if matches!(Dom::node_kind(dom, paragraph), NodeKind::Element("p")) {
                        if let Some(container) = dom.parent_node(paragraph)
                            && within(dom, target, container)
                            && !within(dom, target, paragraph)
                        {
                            let short = text(dom, paragraph, Some(*node));
                            let prefix = short
                                .strip_suffix("...")
                                .or_else(|| short.strip_suffix('…'));
                            if let Some(prefix) = prefix
                                .map(str::trim_end)
                                .filter(|prefix| !prefix.is_empty())
                            {
                                let full = text(dom, target, None);
                                if full.len() > prefix.len() && full.starts_with(prefix) {
                                    disclosures.insert(id);
                                    excerpts.insert(paragraph);
                                }
                            }
                        }
                        break;
                    }
                    ancestor = dom.parent_node(paragraph);
                }
            }
        }
        let disclosure_roots: HashSet<_> = disclosures
            .iter()
            .filter_map(|id| targets.get(id).copied())
            .collect();
        let hidden_nodes: HashSet<_> = styles
            .iter()
            .filter_map(|(node, values)| {
                (values.first().is_some_and(|value| value == "none")
                    || values
                        .get(1)
                        .is_some_and(|value| matches!(value.as_str(), "hidden" | "collapse")))
                .then_some(*node)
            })
            .collect();
        let mut disclosure_paths = HashSet::new();
        for &target in &disclosure_roots {
            let mut chain = Vec::new();
            let mut current = dom.parent_node(target);
            while let Some(node) = current {
                chain.push(node);
                current = dom.parent_node(node);
            }
            if let Some(last_hidden) = chain.iter().rposition(|node| hidden_nodes.contains(node)) {
                disclosure_paths.extend(chain.into_iter().take(last_hidden + 1));
            }
        }
        let mut suppressed = excerpts;
        let mut invisible = HashSet::new();
        let mut blocks = HashSet::new();
        let mut inline_boundaries = HashSet::new();
        let mut superscripts = HashSet::new();
        let mut subscripts = HashSet::new();
        let mut resolved_urls = HashMap::new();
        let mut unchecked_controls = HashSet::new();
        let mut visibility_restored_regions = HashSet::new();
        for (node, values) in styles {
            let role = Dom::attribute(dom, node, "role").unwrap_or_default();
            if matches!(
                Dom::node_kind(dom, node),
                NodeKind::Element("input" | "textarea")
            ) && let Some(element) = dom.node(node).and_then(|node| node.as_element())
            {
                resolved_urls
                    .entry(node)
                    .or_insert_with(HashMap::new)
                    .insert("value".to_owned(), element.input_value());
                if matches!(Dom::node_kind(dom, node), NodeKind::Element("input"))
                    && element.checked()
                {
                    resolved_urls
                        .entry(node)
                        .or_insert_with(HashMap::new)
                        .insert("checked".to_owned(), String::new());
                } else if matches!(Dom::node_kind(dom, node), NodeKind::Element("input")) {
                    unchecked_controls.insert(node);
                }
            }
            // Lazy media often stays transparent until a scroll/load event.
            // Its declared resource still belongs to the document, unlike
            // transparent numeric placeholders or hidden UI text.
            let lazy_media = matches!(
                Dom::node_kind(dom, node),
                NodeKind::Element("img" | "iframe" | "video" | "audio")
            ) && ["data-original", "data-src", "data-lazy-src", "data-srcset"]
                .iter()
                .any(|attribute| {
                    Dom::attribute(dom, node, attribute)
                        .is_some_and(|value| !value.trim().is_empty())
                });
            let disclosure_root = disclosure_roots.contains(&node)
                || role.eq_ignore_ascii_case("tabpanel")
                || Dom::attribute(dom, node, "hidden") == Some("until-found")
                || (!matches!(role, "dialog" | "alertdialog" | "menu")
                    && Dom::attribute(dom, node, "id").is_some_and(|id| disclosures.contains(id)));
            let disclosure_path = disclosure_paths.contains(&node);
            let parent_is_disclosure_path = dom
                .parent_node(node)
                .is_some_and(|parent| disclosure_paths.contains(&parent));
            if disclosure_path {
                invisible.insert(node);
            }
            // Angular/Vue remove cloak attributes only after the bound view
            // is ready. If one remains at dump time, its template text is not
            // reader content even when a missing stylesheet fails to hide it.
            let uninitialized_template = ["ng-cloak", "data-ng-cloak", "x-ng-cloak", "v-cloak"]
                .iter()
                .any(|name| Dom::attribute(dom, node, name).is_some());
            let tracking_pixel = matches!(Dom::node_kind(dom, node), NodeKind::Element("img"))
                && Dom::attribute(dom, node, "alt").is_none_or(|alt| alt.trim().is_empty())
                && values
                    .get(8)
                    .and_then(|value| px(value))
                    .is_some_and(|width| width <= 1.5)
                && values
                    .get(9)
                    .and_then(|value| px(value))
                    .is_some_and(|height| height <= 1.5);
            let foreground = values.get(10).and_then(|value| css_color(value));
            let zero_contrast_leaf = !has_element_child(dom, node)
                && has_text_child(dom, node)
                && !non_solid_backgrounds.contains(&node)
                && values
                    .get(12)
                    .is_none_or(|shadow| shadow.is_empty() || shadow == "none")
                && foreground
                    .filter(|color| color.3 > 0.99)
                    .zip(effective_backgrounds.get(&node).copied())
                    .is_some_and(|(foreground, background)| {
                        background.3 > 0.99
                            && (foreground.0, foreground.1, foreground.2)
                                == (background.0, background.1, background.2)
                    });
            let document_root = Some(node) == document_element || Some(node) == body;
            let visibility_restored = document_visibility_is_hidden
                && !document_root
                && values.get(1).is_some_and(|value| value == "visible");
            if visibility_restored {
                visibility_restored_regions.insert(node);
            }
            let has_bounded_final_opacity =
                has_bounded_final_opacity(&values, &final_opacity_animations);
            if ((values.first().is_some_and(|value| value == "none")
                || values
                    .get(2)
                    .is_some_and(|value| value.parse::<f32>() == Ok(0.0))
                    && !lazy_media
                    && !has_bounded_final_opacity
                    && !document_root)
                && !disclosure_root
                && !disclosure_path)
                || uninitialized_template
                || tracking_pixel
                || (zero_contrast_leaf && !disclosure_root && !disclosure_path)
                || (parent_is_disclosure_path && !disclosure_root && !disclosure_path)
                || dom
                    .parent_node(node)
                    .is_some_and(|parent| suppressed.contains(&parent))
                    && !disclosure_root
                    && !disclosure_path
            {
                suppressed.insert(node);
            }
            if values.first().is_some_and(|value| {
                matches!(
                    value.as_str(),
                    "block" | "flow-root" | "flex" | "grid" | "list-item" | "table"
                )
            }) {
                blocks.insert(node);
            } else if values.first().is_some_and(|value| {
                matches!(
                    value.as_str(),
                    "inline-block" | "inline-flex" | "inline-grid" | "inline-table"
                )
            }) {
                inline_boundaries.insert(node);
            }
            if matches!(Dom::node_kind(dom, node), NodeKind::Element("span")) {
                let relative = values.get(3).is_some_and(|position| position == "relative");
                let vertical_align = values.get(7).map(String::as_str).unwrap_or_default();
                if vertical_align == "super"
                    || (relative
                        && values
                            .get(5)
                            .and_then(|value| px(value))
                            .is_some_and(|top| top < -1.0))
                {
                    superscripts.insert(node);
                } else if vertical_align == "sub"
                    || (relative
                        && values
                            .get(6)
                            .and_then(|value| px(value))
                            .is_some_and(|bottom| bottom < -1.0))
                {
                    subscripts.insert(node);
                }
            }
            if let Some(base_url) = base_url {
                let names: &[&str] = match Dom::node_kind(dom, node) {
                    NodeKind::Element("a") => &["href"],
                    NodeKind::Element("img") => &[
                        "src",
                        "data-original",
                        "data-src",
                        "data-lazy-src",
                        "srcset",
                        "data-srcset",
                    ],
                    NodeKind::Element("iframe" | "audio" | "source") => {
                        &["src", "data-src", "data-lazy-src"]
                    }
                    NodeKind::Element("video") => &["src", "poster", "data-src", "data-lazy-src"],
                    _ => &[],
                };
                for &name in names {
                    let Some(value) = Dom::attribute(dom, node, name)
                        .map(str::trim)
                        .filter(|value| !value.is_empty() && !value.starts_with('#'))
                    else {
                        continue;
                    };
                    let resolved = if matches!(name, "srcset" | "data-srcset") {
                        resolve_srcset(base_url, value)
                    } else {
                        base_url.join(value).ok().map(|url| url.to_string())
                    };
                    if let Some(url) = resolved {
                        resolved_urls
                            .entry(node)
                            .or_insert_with(HashMap::new)
                            .insert(name.to_owned(), url);
                    }
                }
            }
            if values
                .get(1)
                .is_some_and(|value| matches!(value.as_str(), "hidden" | "collapse"))
                && !disclosure_root
                && !disclosure_path
                && (!document_visibility_is_hidden
                    || dom
                        .parent_node(node)
                        .is_some_and(|parent| visibility_restored_regions.contains(&parent)))
            {
                invisible.insert(node);
            }
        }
        Self {
            dom,
            suppressed,
            invisible,
            blocks,
            inline_boundaries,
            superscripts,
            subscripts,
            resolved_urls,
            unchecked_controls,
            fragment_links,
        }
    }

    fn next_retained(&self, mut node: Option<NativeNodeId>) -> Option<NativeNodeId> {
        while let Some(id) = node {
            if !self.suppressed.contains(&id) {
                return Some(id);
            }
            node = self.dom.next_sibling(id);
        }
        None
    }
}

fn has_bounded_final_opacity(
    values: &[String],
    final_opacity_animations: &HashSet<String>,
) -> bool {
    let names: Vec<_> = values
        .get(14)
        .map(|value| value.split(',').map(str::trim).collect())
        .unwrap_or_default();
    let durations: Vec<_> = values
        .get(15)
        .map(|value| value.split(',').map(str::trim).collect())
        .unwrap_or_default();
    let fill_modes: Vec<_> = values
        .get(16)
        .map(|value| value.split(',').map(str::trim).collect())
        .unwrap_or_default();
    let delays: Vec<_> = values
        .get(17)
        .map(|value| value.split(',').map(str::trim).collect())
        .unwrap_or_default();
    let iteration_counts: Vec<_> = values
        .get(18)
        .map(|value| value.split(',').map(str::trim).collect())
        .unwrap_or_default();
    let directions: Vec<_> = values
        .get(19)
        .map(|value| value.split(',').map(str::trim).collect())
        .unwrap_or_default();
    let play_states: Vec<_> = values
        .get(20)
        .map(|value| value.split(',').map(str::trim).collect())
        .unwrap_or_default();
    names.iter().enumerate().any(|(index, name)| {
        final_opacity_animations.contains(*name)
            && !durations.is_empty()
            && animation_duration_is_positive(durations[index % durations.len()])
            && !fill_modes.is_empty()
            && matches!(fill_modes[index % fill_modes.len()], "forwards" | "both")
            && !delays.is_empty()
            && animation_time_seconds(delays[index % delays.len()])
                .is_some_and(|delay| delay <= 0.0)
            && !iteration_counts.is_empty()
            && iteration_counts[index % iteration_counts.len()].parse::<f32>() == Ok(1.0)
            && !directions.is_empty()
            && directions[index % directions.len()] == "normal"
            && !play_states.is_empty()
            && play_states[index % play_states.len()] == "running"
    })
}

fn animation_duration_is_positive(duration: &str) -> bool {
    animation_time_seconds(duration).is_some_and(|duration| duration > 0.0)
}

fn animation_time_seconds(value: &str) -> Option<f32> {
    value
        .strip_suffix("ms")
        .and_then(|value| value.trim().parse::<f32>().ok())
        .map(|value| value / 1000.0)
        .or_else(|| {
            value
                .strip_suffix('s')
                .and_then(|value| value.trim().parse::<f32>().ok())
        })
}

/// Return only animations whose declared final keyframe paints a nonzero
/// opacity. The caller still requires a finite, forward, running animation
/// before using that final authored state for the static document snapshot.
pub(super) fn final_opacity_animation_names<'a>(
    stylesheets: impl IntoIterator<Item = &'a str>,
) -> HashSet<String> {
    let mut result = HashSet::new();
    for source in stylesheets {
        let folded = source.to_ascii_lowercase();
        for offset in code_marker_offsets(&folded, "@keyframes") {
            let Some(relative_open) = source[offset..].find('{') else {
                continue;
            };
            let open = offset + relative_open;
            let Some(close) = matching_delimiter(source, open, '{', '}') else {
                continue;
            };
            let Some(rule) =
                moli_css_parse::parse_keyframes_rule_view_with_stylo(&source[offset..=close])
            else {
                continue;
            };
            let Some(canonical_open) = rule.css_text.find('{') else {
                continue;
            };
            let Some(canonical_close) =
                matching_delimiter(&rule.css_text, canonical_open, '{', '}')
            else {
                continue;
            };
            if final_keyframe_reveals(&rule.css_text[canonical_open + 1..canonical_close]) {
                result.insert(rule.name);
            }
        }
    }
    result
}

fn final_keyframe_reveals(body: &str) -> bool {
    let mut offset = 0;
    while let Some(relative_open) = body[offset..].find('{') {
        let open = offset + relative_open;
        let selector = body[offset..open].trim();
        let Some(close) = matching_delimiter(body, open, '{', '}') else {
            return false;
        };
        let is_final = selector
            .split(',')
            .map(str::trim)
            .any(|value| matches!(value, "to" | "100%" | "100.0%"));
        if is_final {
            for declaration in body[open + 1..close].split(';') {
                let Some((name, value)) = declaration.split_once(':') else {
                    continue;
                };
                if name.trim().eq_ignore_ascii_case("opacity")
                    && value
                        .trim()
                        .parse::<f32>()
                        .is_ok_and(|opacity| opacity > 0.0)
                {
                    return true;
                }
            }
        }
        offset = close + 1;
    }
    false
}

fn target_is_disclosure<D: Dom + ?Sized>(
    dom: &D,
    targets: &HashMap<&str, D::NodeId>,
    id: &str,
) -> bool {
    targets.get(id).is_some_and(|target| {
        !Dom::attribute(dom, *target, "role").is_some_and(|role| {
            ["dialog", "alertdialog", "menu"]
                .iter()
                .any(|excluded| role.eq_ignore_ascii_case(excluded))
        })
    })
}

fn inline_disclosure_targets<'a>(
    value: &'a str,
    visible_functions: &HashSet<String>,
) -> Vec<&'a str> {
    let mut result = Vec::new();
    for (marker, fragment) in [
        ("document.getElementById", false),
        ("document.querySelector", true),
    ] {
        for offset in code_marker_offsets(value, marker) {
            let remaining = &value[offset + marker.len()..];
            let Some((literal, end)) = quoted_call_argument(remaining) else {
                continue;
            };
            let Some(target) = literal
                .strip_prefix('#')
                .or_else(|| (!fragment).then_some(literal))
                .filter(|target| !target.is_empty())
            else {
                continue;
            };
            let direct_mutation = sets_display_visible(&remaining[end..], None);
            let named_mutation = enclosing_function_name(&value[..offset])
                .is_some_and(|name| visible_functions.contains(name));
            if direct_mutation || named_mutation {
                result.push(target);
            }
        }
    }
    result
}

fn visible_handler_functions(dom: &NativeDom) -> HashSet<String> {
    let mut result = HashSet::new();
    let mut pending = vec![dom.document_node_id()];
    while let Some(node) = pending.pop() {
        if !matches!(Dom::node_kind(dom, node), NodeKind::Element("script")) {
            pending.extend(dom.child_ids(node));
            continue;
        }
        let source = descendant_text(dom, node);
        for offset in code_marker_offsets(&source, "function") {
            let signature = &source[offset + "function".len()..];
            let Some((name, parameter, body)) = function_parts(signature) else {
                continue;
            };
            if function_sets_parameter_visible(body, parameter) {
                result.insert(name.to_owned());
            }
        }
    }
    result
}

fn function_parts(source: &str) -> Option<(&str, &str, &str)> {
    let source = source.trim_start();
    let name_end = source.find(|character: char| !is_identifier(character))?;
    let name = &source[..name_end];
    if name.is_empty() {
        return None;
    }
    let parameters = source[name_end..].trim_start().strip_prefix('(')?;
    let close = parameters.find(')')?;
    let parameter = parameters[..close].trim();
    if parameter.is_empty() || parameter.contains(',') || !parameter.chars().all(is_identifier) {
        return None;
    }
    let rest = parameters[close + 1..].trim_start();
    let body_start = rest.find('{')?;
    let body_end = matching_delimiter(rest, body_start, '{', '}')?;
    Some((name, parameter, &rest[body_start + 1..body_end]))
}

fn function_sets_parameter_visible(body: &str, parameter: &str) -> bool {
    for offset in code_marker_offsets(body, ".style.display") {
        let owner = body[..offset]
            .trim_end()
            .rsplit_once(|character: char| !is_identifier(character))
            .map_or(body[..offset].trim_end(), |(_, owner)| owner);
        if owner.is_empty()
            || !sets_display_visible(&body[offset + ".style.display".len()..], Some(""))
        {
            continue;
        }
        if owner == parameter || alias_depends_on_parameter(&body[..offset], owner, parameter) {
            return true;
        }
    }
    false
}

fn alias_depends_on_parameter(prefix: &str, owner: &str, parameter: &str) -> bool {
    let Some(offset) = code_marker_offsets(prefix, owner)
        .into_iter()
        .rev()
        .find(|offset| {
            let before = prefix[..*offset].chars().next_back();
            let after = prefix[*offset + owner.len()..].chars().next();
            before.is_none_or(|character| !is_identifier(character))
                && after.is_none_or(|character| !is_identifier(character))
                && prefix[*offset + owner.len()..]
                    .trim_start()
                    .starts_with('=')
        })
    else {
        return false;
    };
    let assignment = prefix[offset + owner.len()..].trim_start();
    let Some(right) = assignment.strip_prefix('=') else {
        return false;
    };
    let right = right.split_once(';').map_or(right, |(right, _)| right);
    contains_identifier(right, parameter)
}

fn contains_identifier(value: &str, identifier: &str) -> bool {
    code_marker_offsets(value, identifier)
        .into_iter()
        .any(|offset| {
            let before = value[..offset].chars().next_back();
            let after = value[offset + identifier.len()..].chars().next();
            before.is_none_or(|character| !is_identifier(character))
                && after.is_none_or(|character| !is_identifier(character))
        })
}

fn enclosing_function_name(value: &str) -> Option<&str> {
    let value = value.trim_end();
    let value = value.strip_suffix('(')?.trim_end();
    let start = value
        .rfind(|character: char| !is_identifier(character))
        .map_or(0, |offset| offset + 1);
    let name = &value[start..];
    (!name.is_empty()).then_some(name)
}

fn quoted_call_argument(value: &str) -> Option<(&str, usize)> {
    let leading = value.len() - value.trim_start().len();
    let value = value.trim_start().strip_prefix('(')?;
    let after_open = value.len() - value.trim_start().len();
    let value = value.trim_start();
    let quote = value
        .chars()
        .next()
        .filter(|quote| matches!(quote, '\'' | '"'))?;
    let literal = &value[quote.len_utf8()..];
    let end = literal.find(quote)?;
    let after_literal = literal[end + quote.len_utf8()..].trim_start();
    if !after_literal.starts_with(')') {
        return None;
    }
    let consumed = leading
        + 1
        + after_open
        + quote.len_utf8()
        + end
        + quote.len_utf8()
        + (literal[end + quote.len_utf8()..].len() - after_literal.len())
        + 1;
    Some((&literal[..end], consumed))
}

fn sets_display_visible(value: &str, display_already_consumed: Option<&str>) -> bool {
    let value = if display_already_consumed.is_some() {
        value
    } else {
        let Some(value) = value.trim_start().strip_prefix(".style.display") else {
            return false;
        };
        value
    };
    let Some(value) = value.trim_start().strip_prefix('=') else {
        return false;
    };
    let value = value.trim_start();
    let Some(quote) = value
        .chars()
        .next()
        .filter(|quote| matches!(quote, '\'' | '"'))
    else {
        return false;
    };
    let literal = &value[quote.len_utf8()..];
    let Some(end) = literal.find(quote) else {
        return false;
    };
    matches!(
        literal[..end].trim(),
        "" | "block"
            | "inline"
            | "inline-block"
            | "flex"
            | "inline-flex"
            | "grid"
            | "inline-grid"
            | "list-item"
            | "table"
            | "table-row"
            | "table-cell"
    )
}

fn code_marker_offsets(value: &str, marker: &str) -> Vec<usize> {
    let mut result = Vec::new();
    let mut offset = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    while offset < value.len() {
        let tail = &value[offset..];
        if line_comment {
            if tail.starts_with('\n') {
                line_comment = false;
            }
        } else if block_comment {
            if tail.starts_with("*/") {
                block_comment = false;
                offset += 2;
                continue;
            }
        } else if let Some(delimiter) = quote {
            let character = tail.chars().next().expect("offset is within value");
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
        } else if tail.starts_with("//") {
            line_comment = true;
            offset += 2;
            continue;
        } else if tail.starts_with("/*") {
            block_comment = true;
            offset += 2;
            continue;
        } else {
            let character = tail.chars().next().expect("offset is within value");
            if matches!(character, '\'' | '"' | '`') {
                quote = Some(character);
            } else if tail.starts_with(marker) {
                result.push(offset);
                offset += marker.len();
                continue;
            }
        }
        offset += tail
            .chars()
            .next()
            .expect("offset is within value")
            .len_utf8();
    }
    result
}

fn matching_delimiter(value: &str, start: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut offset = start;
    while offset < value.len() {
        let tail = &value[offset..];
        if line_comment {
            if tail.starts_with('\n') {
                line_comment = false;
            }
        } else if block_comment {
            if tail.starts_with("*/") {
                block_comment = false;
                offset += 2;
                continue;
            }
        } else if let Some(delimiter) = quote {
            let character = tail.chars().next().expect("offset is within value");
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
        } else if tail.starts_with("//") {
            line_comment = true;
            offset += 2;
            continue;
        } else if tail.starts_with("/*") {
            block_comment = true;
            offset += 2;
            continue;
        } else {
            let character = tail.chars().next().expect("offset is within value");
            if matches!(character, '\'' | '"' | '`') {
                quote = Some(character);
            } else if character == open {
                depth += 1;
            } else if character == close {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
        }
        offset += tail
            .chars()
            .next()
            .expect("offset is within value")
            .len_utf8();
    }
    None
}

fn is_identifier(character: char) -> bool {
    character == '_' || character == '$' || character.is_ascii_alphanumeric()
}

fn descendant_text(dom: &NativeDom, root: NativeNodeId) -> String {
    let mut pending = vec![root];
    let mut result = String::new();
    while let Some(node) = pending.pop() {
        if let NodeKind::Text(value) = Dom::node_kind(dom, node) {
            result.push_str(value);
        }
        let mut children = Vec::new();
        let mut child = dom.first_child(node);
        while let Some(id) = child {
            children.push(id);
            child = dom.next_sibling(id);
        }
        pending.extend(children.into_iter().rev());
    }
    result
}

fn px(value: &str) -> Option<f32> {
    value.strip_suffix("px")?.trim().parse().ok()
}

fn resolve_srcset(base_url: &Url, value: &str) -> Option<String> {
    let mut resolved = Vec::new();
    let bytes = value.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        while offset < bytes.len() && (bytes[offset].is_ascii_whitespace() || bytes[offset] == b',')
        {
            offset += 1;
        }
        let start = offset;
        while offset < bytes.len() && !bytes[offset].is_ascii_whitespace() {
            offset += 1;
        }
        if start == offset {
            break;
        }
        let mut end = offset;
        while end > start && bytes[end - 1] == b',' {
            end -= 1;
        }
        let ended_with_comma = end < offset;
        let descriptor_start = offset;
        if !ended_with_comma {
            while offset < bytes.len() && bytes[offset] != b',' {
                offset += 1;
            }
        }
        let descriptor = if ended_with_comma {
            ""
        } else {
            value[descriptor_start..offset].trim()
        };
        let url = base_url.join(&value[start..end]).ok()?;
        resolved.push(if descriptor.is_empty() {
            url.to_string()
        } else {
            format!("{url} {descriptor}")
        });
    }
    (!resolved.is_empty()).then(|| resolved.join(", "))
}

fn css_color(value: &str) -> Option<(u8, u8, u8, f32)> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "transparent" => return Some((0, 0, 0, 0.0)),
        "white" => return Some((255, 255, 255, 1.0)),
        "black" => return Some((0, 0, 0, 1.0)),
        _ => {}
    }
    let (body, alpha) = match (
        value
            .strip_prefix("rgba(")
            .and_then(|body| body.strip_suffix(')')),
        value
            .strip_prefix("rgb(")
            .and_then(|body| body.strip_suffix(')')),
    ) {
        (Some(body), _) => (body, true),
        (_, Some(body)) => (body, false),
        _ => return None,
    };
    let mut parts = body.split(',').map(str::trim);
    let red = parts.next()?.parse().ok()?;
    let green = parts.next()?.parse().ok()?;
    let blue = parts.next()?.parse().ok()?;
    let opacity = if alpha {
        parts.next()?.parse().ok()?
    } else {
        1.0
    };
    parts
        .next()
        .is_none()
        .then_some((red, green, blue, opacity))
}

fn has_element_child(dom: &NativeDom, node: NativeNodeId) -> bool {
    let mut child = dom.first_child(node);
    while let Some(id) = child {
        if matches!(Dom::node_kind(dom, id), NodeKind::Element(_)) {
            return true;
        }
        child = dom.next_sibling(id);
    }
    false
}

fn has_text_child(dom: &NativeDom, node: NativeNodeId) -> bool {
    let mut child = dom.first_child(node);
    while let Some(id) = child {
        if matches!(Dom::node_kind(dom, id), NodeKind::Text(text) if !text.trim().is_empty()) {
            return true;
        }
        child = dom.next_sibling(id);
    }
    false
}

fn within(dom: &NativeDom, mut node: NativeNodeId, ancestor: NativeNodeId) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        let Some(parent) = dom.parent_node(node) else {
            return false;
        };
        node = parent;
    }
}

fn text(dom: &NativeDom, root: NativeNodeId, skip: Option<NativeNodeId>) -> String {
    let mut pending = vec![root];
    let mut result = String::new();
    while let Some(node) = pending.pop() {
        if Some(node) == skip {
            continue;
        }
        match Dom::node_kind(dom, node) {
            NodeKind::Text(value) => result.push_str(value),
            NodeKind::Element("script" | "style" | "template") => continue,
            _ => {}
        }
        let mut children = Vec::new();
        let mut child = dom.first_child(node);
        while let Some(id) = child {
            children.push(id);
            child = dom.next_sibling(id);
        }
        pending.extend(children.into_iter().rev());
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Dom for MarkdownDom<'_> {
    type NodeId = NativeNodeId;

    fn node_kind(&self, node: Self::NodeId) -> NodeKind<'_> {
        if self.suppressed.contains(&node) {
            return NodeKind::Other;
        }
        let kind = Dom::node_kind(self.dom, node);
        if self.invisible.contains(&node) {
            // Visibility can be overridden by a descendant. Keep traversing
            // without emitting this element's own image, link or formatting.
            return NodeKind::Document;
        }
        if matches!(kind, NodeKind::Text(_))
            && self
                .dom
                .parent_node(node)
                .is_some_and(|parent| self.invisible.contains(&parent))
        {
            return NodeKind::Other;
        }
        if self.superscripts.contains(&node) {
            return NodeKind::Element("sup");
        }
        if self.subscripts.contains(&node) {
            return NodeKind::Element("sub");
        }
        kind
    }

    fn first_child(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.next_retained(self.dom.first_child(node))
    }

    fn next_sibling(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.next_retained(self.dom.next_sibling(node))
    }

    fn attribute(&self, node: Self::NodeId, name: &str) -> Option<&str> {
        if name == "checked" && self.unchecked_controls.contains(&node) {
            return None;
        }
        self.resolved_urls
            .get(&node)
            .and_then(|attributes| attributes.get(name))
            .map(String::as_str)
            .or_else(|| Dom::attribute(self.dom, node, name))
    }

    fn has_block_layout(&self, node: Self::NodeId) -> bool {
        self.blocks.contains(&node)
    }

    fn has_text_boundary(&self, node: Self::NodeId) -> bool {
        self.inline_boundaries.contains(&node)
    }

    fn may_have_fragment_links(&self) -> bool {
        self.fragment_links
    }
}
