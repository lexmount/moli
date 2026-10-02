use super::*;
use std::collections::HashSet;

pub(super) const MAX_WORK: usize = 1_000_000;

#[derive(Clone)]
pub(super) struct Unit {
    pub value: f64,
    pub unit: String,
}

impl Unit {
    pub fn read<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        Some(Self {
            unit: values::css_unit_value_unit(scope, object)?,
            value: values::css_unit_value_number(scope, object)?,
        })
    }

    pub fn bind<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        realm: v8::Local<'s, v8::Context>,
    ) -> v8::Local<'s, v8::Object> {
        let scope = &mut v8::ContextScope::new(scope, realm);
        values::unit_value(scope, self.value, self.unit)
    }
}

pub(super) enum Node {
    Unit(Unit),
    Math(Kind, Vec<usize>),
}

pub(super) struct Graph {
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
}

fn intern<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    identities: v8::Local<'s, v8::Map>,
    objects: &mut Vec<v8::Local<'s, v8::Object>>,
) -> Option<usize> {
    let known = identities.get(scope, object.into())?;
    if !known.is_undefined() {
        return Some(known.uint32_value(scope)? as usize);
    }
    if objects.len() >= MAX_WORK {
        crate::util::throw_range_error(scope, "CSS numeric graph is too large");
        return None;
    }
    let index = objects.len();
    let value = v8::Integer::new_from_unsigned(scope, index as u32);
    identities.set(scope, object.into(), value.into())?;
    objects.push(object);
    Some(index)
}

impl Graph {
    /// Snapshot after all argument conversions. Values are mutable, but child
    /// identities are not. A flat graph avoids native recursion (including Drop)
    /// and visits a shared operand only once, without invoking author getters.
    pub fn read<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        roots: &[v8::Local<'s, v8::Object>],
    ) -> Option<Self> {
        let identities = v8::Map::new(scope);
        let mut objects = Vec::new();
        let roots = roots
            .iter()
            .map(|&object| intern(scope, object, identities, &mut objects))
            .collect::<Option<Vec<_>>>()?;
        let mut nodes = Vec::new();
        let mut edges = 0usize;
        while nodes.len() < objects.len() {
            let object = objects[nodes.len()];
            if let Some(unit) = Unit::read(scope, object) {
                nodes.push(Node::Unit(unit));
                continue;
            }
            let kind = kind(scope, object)?;
            let children = operands(scope, object)?;
            edges += children.len();
            if edges > MAX_WORK {
                crate::util::throw_range_error(scope, "CSS numeric graph is too large");
                return None;
            }
            let children = children
                .into_iter()
                .map(|child| intern(scope, child, identities, &mut objects))
                .collect::<Option<Vec<_>>>()?;
            nodes.push(Node::Math(kind, children));
        }
        Some(Self { nodes, roots })
    }
}

/// Compare lazily so an early mismatch does not traverse unrelated subgraphs.
/// Argument conversion has already finished; only private native slots are read.
pub(in crate::context_bootstrap::css_runtime::typed_om) fn equals<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    items: &[v8::Local<'s, v8::Object>],
) -> Option<bool> {
    let identities = v8::Map::new(scope);
    let mut objects = Vec::new();
    let left = intern(scope, receiver, identities, &mut objects)?;
    let mut visited = HashSet::new();
    for &item in items {
        let right = intern(scope, item, identities, &mut objects)?;
        let mut pending = vec![(left, right)];
        while let Some((a, b)) = pending.pop() {
            if !visited.insert((a, b)) {
                continue;
            }
            if visited.len() > MAX_WORK {
                crate::util::throw_range_error(scope, "CSS numeric comparison is too large");
                return None;
            }
            let (a, b) = (objects[a], objects[b]);
            match (Unit::read(scope, a), Unit::read(scope, b)) {
                (Some(a), Some(b)) => {
                    // Even identical objects can hold NaN after arithmetic.
                    if a.unit != b.unit || a.value != b.value {
                        return Some(false);
                    }
                }
                (None, None) => {
                    if kind(scope, a)? != kind(scope, b)? {
                        return Some(false);
                    }
                    let (a, b) = (operands(scope, a)?, operands(scope, b)?);
                    if a.len() != b.len() {
                        return Some(false);
                    }
                    if pending.len() + a.len() > MAX_WORK {
                        crate::util::throw_range_error(
                            scope,
                            "CSS numeric comparison is too large",
                        );
                        return None;
                    }
                    for (a, b) in a.into_iter().zip(b).rev() {
                        pending.push((
                            intern(scope, a, identities, &mut objects)?,
                            intern(scope, b, identities, &mut objects)?,
                        ));
                    }
                }
                _ => return Some(false),
            }
        }
    }
    Some(true)
}
