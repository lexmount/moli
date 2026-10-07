use std::{collections::HashMap, time::Instant};

use moli_svg::{
    SvgAnimationInstanceTimes, SvgAnimationRestart, SvgAnimationTiming, SvgPresentationClock,
    parse_clock_value,
};
use style::values::{
    animated::{Animate, Procedure},
    computed::DProperty,
    specified::svg_path::SVGPathData,
};

use super::JsContextHost;
use crate::{document_runtime::DomHandle, native_bridge::document::SVG_NS};

pub(super) struct SvgSmilState {
    epoch: Instant,
    clocks: HashMap<DomHandle, SvgPresentationClock>,
    instances: HashMap<DomHandle, SvgAnimationInstanceTimes>,
    effects: HashMap<DomHandle, FragmentEffects>,
}

struct FragmentEffects {
    version: u64,
    targets: HashMap<DomHandle, Vec<DomHandle>>,
}

impl Default for SvgSmilState {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            clocks: HashMap::new(),
            instances: HashMap::new(),
            effects: HashMap::new(),
        }
    }
}

impl SvgSmilState {
    fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }
}

impl JsContextHost {
    /// All nested SVG elements in one SVG document fragment share its clock.
    /// An intervening non-SVG element (for example in foreignObject) starts a
    /// new fragment. Merely having an associated synthetic Window is not enough.
    pub(crate) fn svg_animation_fragment(&self, handle: DomHandle) -> Option<DomHandle> {
        let dom = self.dom_host();
        let mut current = handle;
        let mut root = None;
        loop {
            let node = dom.node(current)?;
            if node.namespace() != Some(SVG_NS) {
                break;
            }
            if node.local_name() == Some("svg") {
                root = Some(current);
            }
            let Some(parent) = node.parent_node() else {
                break;
            };
            current = parent;
        }
        root
    }

    pub(crate) fn record_svg_document_begin(&self, document: DomHandle) {
        if self.window_endpoint_for_document(document).is_none() {
            return;
        }
        let mut state = self.svg_smil.borrow_mut();
        let now = state.now();
        let mut nodes = vec![document];
        while let Some(handle) = nodes.pop() {
            if self.dom_host().node(handle).is_some_and(|node| {
                node.namespace() == Some(SVG_NS) && node.local_name() == Some("svg")
            }) && self.svg_animation_fragment(handle) == Some(handle)
            {
                state.clocks.entry(handle).or_default().start(now);
            }
            nodes.extend(self.dom_host().child_nodes(handle).unwrap_or_default());
        }
    }

    fn with_svg_clock<R>(
        &self,
        root: DomHandle,
        update: impl FnOnce(&mut SvgPresentationClock, f64) -> R,
    ) -> R {
        let dom = self.dom_host();
        let active = dom.is_connected_to_document(root)
            && dom.owner_document_handle(root).is_some_and(|document| {
                self.window_endpoint_for_document(document).is_some()
                    && self.document_ready_state_for_handle(document) == "complete"
            });
        let mut state = self.svg_smil.borrow_mut();
        let now = state.now();
        let clock = state.clocks.entry(root).or_default();
        if active {
            clock.start(now);
        }
        update(clock, now)
    }

    pub(crate) fn svg_presentation_time(&self, handle: DomHandle) -> f64 {
        self.svg_animation_fragment(handle)
            .map(|root| self.with_svg_clock(root, |clock, now| clock.current_time(now)))
            .unwrap_or(0.0)
    }

    pub(crate) fn svg_animations_paused(&self, handle: DomHandle) -> bool {
        self.svg_animation_fragment(handle)
            .is_some_and(|root| self.with_svg_clock(root, |clock, _| clock.is_paused()))
    }

    pub(crate) fn pause_svg_animations(&self, handle: DomHandle, pause: bool) {
        if self.svg_animation_fragment(handle) == Some(handle) {
            self.with_svg_clock(handle, |clock, now| {
                if pause {
                    clock.pause(now);
                } else {
                    clock.unpause(now);
                }
            });
        }
    }

    pub(crate) fn seek_svg_animations(&self, handle: DomHandle, seconds: f32) {
        if self.svg_animation_fragment(handle) == Some(handle) {
            self.with_svg_clock(handle, |clock, now| clock.seek(now, f64::from(seconds)));
        }
    }

    pub(crate) fn svg_animation_target(&self, animation: DomHandle) -> Option<DomHandle> {
        let dom = self.dom_host();
        if !dom.is_connected_to_document(animation) {
            return None;
        }
        let href = dom
            .get_attribute_ns(animation, None, "href")
            .or_else(|| {
                dom.get_attribute_ns(animation, Some("http://www.w3.org/1999/xlink"), "href")
            })
            .unwrap_or_default();
        let target = if href.is_empty() {
            dom.node(animation)?.parent_node()?
        } else {
            let fragment = if let Some(fragment) = href.strip_prefix('#') {
                fragment.to_owned()
            } else {
                let document = dom.owner_document_handle(animation)?;
                let mut url = self
                    .document_base_url_for_handle(document)
                    .join(&href)
                    .ok()?;
                let fragment = url.fragment()?.to_owned();
                url.set_fragment(None);
                let mut document_url = self.document_url_for_handle(document);
                document_url.set_fragment(None);
                if url != document_url {
                    return None;
                }
                fragment
            };
            if fragment.is_empty() {
                return None;
            }
            let id = percent_encoding::percent_decode_str(&fragment).decode_utf8_lossy();
            dom.element_handle_by_id_in_subtree(dom.root_node_handle(animation)?, &id)?
        };
        dom.node(target)
            .is_some_and(|node| node.namespace() == Some(SVG_NS))
            .then_some(target)
    }

    pub(crate) fn svg_animation_simple_duration(&self, animation: DomHandle) -> Option<f64> {
        self.dom_host()
            .get_attribute(animation, "dur")
            .and_then(|raw| parse_clock_value(&raw))
            .filter(|v| *v >= 0.0)
    }

    fn svg_animation_timing(&self, animation: DomHandle) -> SvgAnimationTiming {
        let attribute = |name| self.dom_host().get_attribute(animation, name);
        let times = |raw: Option<String>, default| {
            raw.map(|raw| raw.split(';').filter_map(parse_clock_value).collect())
                .unwrap_or(default)
        };
        SvgAnimationTiming {
            begins: times(attribute("begin"), vec![0.0]),
            ends: times(attribute("end"), vec![]),
            simple_duration: self
                .svg_animation_simple_duration(animation)
                .unwrap_or(f64::INFINITY),
            repeat_count: attribute("repeatCount")
                .map(|raw| {
                    if raw == "indefinite" {
                        f64::INFINITY
                    } else {
                        raw.parse::<f64>()
                            .ok()
                            .filter(|v| v.is_finite() && *v > 0.0)
                            .unwrap_or(1.0)
                    }
                })
                .unwrap_or(1.0),
            repeat_duration: attribute("repeatDur")
                .and_then(|raw| parse_clock_value(&raw))
                .filter(|v| *v >= 0.0)
                .unwrap_or(f64::INFINITY),
            freeze: attribute("fill").as_deref() == Some("freeze"),
            restart: match attribute("restart").as_deref() {
                Some("never") => SvgAnimationRestart::Never,
                Some("whenNotActive") => SvgAnimationRestart::WhenNotActive,
                _ => SvgAnimationRestart::Always,
            },
        }
    }

    pub(crate) fn svg_animation_start_time(&self, animation: DomHandle) -> Option<f64> {
        self.svg_animation_target(animation)?;
        let time = self.svg_presentation_time(animation);
        let timing = self.svg_animation_timing(animation);
        let state = self.svg_smil.borrow();
        timing
            .current_interval(
                state
                    .instances
                    .get(&animation)
                    .unwrap_or(&SvgAnimationInstanceTimes::default()),
                time,
            )
            .map(|interval| interval.begin)
    }

    pub(crate) fn add_svg_animation_instance(
        &self,
        animation: DomHandle,
        begin: bool,
        offset: f32,
    ) {
        if self.svg_animation_fragment(animation).is_none() {
            return;
        }
        let time = self.svg_presentation_time(animation) + f64::from(offset);
        let mut state = self.svg_smil.borrow_mut();
        let instances = state.instances.entry(animation).or_default();
        let list = if begin {
            &mut instances.begins
        } else {
            &mut instances.ends
        };
        if !list.contains(&time) {
            list.push(time);
        }
    }

    /// Native path sampling is applied to an observation's typed style, shared
    /// by CSSOM, SVG geometry and box construction. Its base cascade and the
    /// authored d attribute remain unchanged.
    pub(crate) fn sampled_svg_path(
        &self,
        target: DomHandle,
        base: &DProperty,
    ) -> Option<SVGPathData> {
        let dom = self.dom_host();
        if !dom.node(target).is_some_and(|node| {
            node.namespace() == Some(SVG_NS) && node.local_name() == Some("path")
        }) || !dom.is_connected_to_document(target)
        {
            return None;
        }
        let root = self.svg_animation_fragment(target)?;
        let time = self.svg_presentation_time(root);
        if !self.with_svg_clock(root, |clock, _| clock.has_started()) {
            return None;
        }
        let animations = {
            let mut state = self.svg_smil.borrow_mut();
            let version = dom.dom_version();
            if state
                .effects
                .get(&root)
                .is_none_or(|effects| effects.version != version)
            {
                let mut targets: HashMap<DomHandle, Vec<DomHandle>> = HashMap::new();
                let mut nodes = vec![root];
                while let Some(handle) = nodes.pop() {
                    if dom.node(handle).is_some_and(|node| {
                        node.namespace() == Some(SVG_NS)
                            && matches!(node.local_name(), Some("animate" | "set"))
                    }) && self.svg_animation_fragment(handle) == Some(root)
                        && let Some(target) = self.svg_animation_target(handle)
                    {
                        targets.entry(target).or_default().push(handle);
                    }
                    let children = dom.child_nodes(handle).unwrap_or_default();
                    nodes.extend(children.into_iter().rev());
                }
                state
                    .effects
                    .insert(root, FragmentEffects { version, targets });
            }
            state.effects.get(&root)?.targets.get(&target)?.clone()
        };
        let mut result = None;
        for animation in animations {
            if dom.get_attribute(animation, "attributeName").as_deref() != Some("d") {
                continue;
            }
            let timing = self.svg_animation_timing(animation);
            let progress = {
                let state = self.svg_smil.borrow();
                timing.sample_progress(
                    state
                        .instances
                        .get(&animation)
                        .unwrap_or(&SvgAnimationInstanceTimes::default()),
                    time,
                )
            };
            let Some(progress) = progress else {
                continue;
            };
            if let Some(path) = sample_path(self, animation, base, progress) {
                result = Some(path);
            }
        }
        result
    }
}

fn sample_path(
    runtime: &JsContextHost,
    animation: DomHandle,
    base: &DProperty,
    progress: f64,
) -> Option<SVGPathData> {
    let dom = runtime.dom_host();
    let attribute = |name| dom.get_attribute(animation, name);
    let parse = |raw: &str| {
        let (path, valid) = SVGPathData::parse_bytes(raw.as_bytes());
        valid.then_some(path)
    };
    if dom.node(animation)?.local_name() == Some("set") {
        return parse(&attribute("to")?);
    }
    // Do not silently interpret unsupported additive/spline/paced functions as
    // linear replacement. They need their own composition and distance model.
    if attribute("additive").as_deref() == Some("sum")
        || attribute("accumulate").as_deref() == Some("sum")
    {
        return None;
    }
    let discrete = match attribute("calcMode").as_deref() {
        None | Some("linear") => false,
        Some("discrete") => true,
        _ => return None,
    };
    let paths = if let Some(values) = attribute("values") {
        values
            .split(';')
            .map(|raw| parse(raw.trim()))
            .collect::<Option<Vec<_>>>()?
    } else {
        let from = if let Some(from) = attribute("from") {
            parse(&from)?
        } else {
            match base {
                DProperty::Path(path) => path.clone(),
                DProperty::None => return None,
            }
        };
        vec![from, parse(&attribute("to")?)?]
    };
    if paths.is_empty() {
        return None;
    }
    if paths.len() == 1 {
        return paths.into_iter().next();
    }
    let count = paths.len();
    let times = if let Some(times) = attribute("keyTimes") {
        let times = times
            .split(';')
            .map(|v| v.trim().parse::<f64>().ok())
            .collect::<Option<Vec<_>>>()?;
        if times.len() != count
            || times[0] != 0.0
            || (!discrete && times[count - 1] != 1.0)
            || !times
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            || !times.windows(2).all(|pair| pair[0] <= pair[1])
        {
            return None;
        }
        times
    } else {
        let denominator = if discrete { count } else { count - 1 } as f64;
        (0..count).map(|i| i as f64 / denominator).collect()
    };
    let index = times
        .iter()
        .rposition(|time| *time <= progress)
        .unwrap_or(0);
    if discrete || index == count - 1 {
        return Some(paths[index].clone());
    }
    let span = times[index + 1] - times[index];
    let progress = if span == 0.0 {
        1.0
    } else {
        (progress - times[index]) / span
    };
    paths[index]
        .animate(&paths[index + 1], Procedure::Interpolate { progress })
        .ok()
}
