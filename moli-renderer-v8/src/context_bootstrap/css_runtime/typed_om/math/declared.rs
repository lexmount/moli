use super::graph::{Graph, MAX_WORK, Node};
use super::*;
use moli_css_parse::{CssDeclaredMathNode, CssDeclaredNumericValue, CssDeclaredUnitValue};

pub(in crate::context_bootstrap::css_runtime::typed_om) fn snapshot_declared_math<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Object>,
) -> Option<CssDeclaredNumericValue> {
    let graph = Graph::read(scope, &[value])?;
    debug_assert_eq!(graph.roots, [0]);
    let nodes = graph.nodes.into_iter().map(|node| match node {
        Node::Unit(unit) => {
            // Use the property's canonical units without combining its operands.
            let (name, ratio) = conversion::canonical(&unit.unit);
            CssDeclaredMathNode::Unit(CssDeclaredUnitValue {
                value: unit.value * ratio,
                unit: name.to_owned(),
            })
        }
        Node::Math(operator, children) => CssDeclaredMathNode::Math(operator, children),
    });
    Some(CssDeclaredNumericValue::Math(nodes.collect()))
}

enum Work {
    Node(usize),
    Bind(Kind, usize),
}

pub(in crate::context_bootstrap::css_runtime::typed_om) fn reify_declared_numeric<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &CssDeclaredNumericValue,
) -> Option<v8::Local<'s, v8::Object>> {
    let nodes = match value {
        CssDeclaredNumericValue::Unit(unit) => {
            return Some(values::unit_value(scope, unit.value, unit.unit.clone()));
        }
        CssDeclaredNumericValue::Math(nodes) => nodes,
    };
    let realm = scope.get_current_context();
    let mut pending = vec![Work::Node(0)];
    let mut objects = Vec::new();
    let mut remaining = MAX_WORK;
    while let Some(work) = pending.pop() {
        remaining = remaining.checked_sub(1).or_else(|| {
            crate::util::throw_range_error(scope, "CSS numeric value is too large");
            None
        })?;
        match work {
            Work::Node(index) => match nodes.get(index)? {
                CssDeclaredMathNode::Unit(unit) => {
                    objects.push(values::unit_value(scope, unit.value, unit.unit.clone()));
                }
                CssDeclaredMathNode::Math(kind, children) => {
                    pending.push(Work::Bind(*kind, children.len()));
                    pending.extend(children.iter().rev().map(|&index| Work::Node(index)));
                }
            },
            Work::Bind(kind, count) => {
                let start = objects.len().checked_sub(count)?;
                let object = math_value(scope, kind, &objects[start..], realm)?;
                objects.truncate(start);
                objects.push(object);
            }
        }
    }
    objects.pop()
}
