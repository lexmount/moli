use super::graph::{Graph, MAX_WORK, Node, Unit};
use super::operations::{maximum, minimum};
use super::*;
use std::collections::BTreeMap;

type UnitMap = BTreeMap<String, i32>;

#[derive(Clone)]
struct Term {
    value: f64,
    units: UnitMap,
}

#[derive(Clone, Copy)]
enum Error {
    Incompatible,
    TooLarge,
}

fn throw(scope: &mut v8::PinScope<'_, '_>, error: Error) {
    match error {
        Error::Incompatible => throw_type_error(
            scope,
            "CSS numeric value cannot be converted to these units",
        ),
        Error::TooLarge => {
            crate::util::throw_range_error(scope, "CSS numeric conversion is too large")
        }
    }
}

struct Budget(usize);

impl Budget {
    fn spend(&mut self, amount: usize) -> Result<(), Error> {
        self.0 = self.0.checked_sub(amount).ok_or(Error::TooLarge)?;
        Ok(())
    }
}

// DOM quantities must stay f64. Stylo's NoCalcNumeric/SumValue use f32 and
// currently omit frequency/resolution/flex; only the fixed CSS ratios live
// here. Relative units remain independent of layout and of one another.
pub(super) fn canonical(unit: &str) -> (&str, f64) {
    match unit {
        "in" => ("px", 96.0),
        "cm" => ("px", 96.0 / 2.54),
        "mm" => ("px", 96.0 / 25.4),
        "q" => ("px", 96.0 / 101.6),
        "pt" => ("px", 96.0 / 72.0),
        "pc" => ("px", 16.0),
        "rad" => ("deg", 180.0 / std::f64::consts::PI),
        "grad" => ("deg", 0.9),
        "turn" => ("deg", 360.0),
        "ms" => ("s", 0.001),
        "khz" => ("hz", 1000.0),
        "dpi" => ("dppx", 1.0 / 96.0),
        "dpcm" => ("dppx", 2.54 / 96.0),
        "x" => ("dppx", 1.0),
        _ => (unit, 1.0),
    }
}

impl Unit {
    fn convert(&self, unit: &str) -> Option<f64> {
        if self.unit == unit {
            return Some(self.value);
        }
        let (from, from_ratio) = canonical(&self.unit);
        let (to, to_ratio) = canonical(unit);
        (from == to).then(|| self.value * (from_ratio / to_ratio))
    }
}

impl Term {
    fn from_unit(unit: &Unit) -> Self {
        let (name, ratio) = canonical(&unit.unit);
        Self {
            value: unit.value * ratio,
            units: if name == "number" {
                BTreeMap::new()
            } else {
                [(name.to_owned(), 1)].into()
            },
        }
    }

    fn into_unit(self) -> Result<Unit, Error> {
        let unit = match self.units.len() {
            0 => "number".to_owned(),
            1 => {
                let (unit, power) = self.units.into_iter().next().unwrap();
                if power != 1 {
                    return Err(Error::Incompatible);
                }
                unit
            }
            _ => return Err(Error::Incompatible),
        };
        Ok(Unit {
            value: self.value,
            unit,
        })
    }

    fn numeric_type(&self) -> Result<NumericType, Error> {
        let mut result = NumericType::default();
        for (unit, &power) in &self.units {
            let mut next = NumericType::from_unit(unit).ok_or(Error::Incompatible)?;
            for exponent in &mut next.powers {
                *exponent *= power;
            }
            result = result.multiply(next).ok_or(Error::TooLarge)?;
        }
        Ok(result)
    }

    fn cost(&self) -> usize {
        1 + self.units.len()
    }
}

fn combine(kind: Kind, args: &[&[Term]], budget: &mut Budget) -> Result<Vec<Term>, Error> {
    match kind {
        Kind::Sum => {
            let mut values: Vec<Term> = Vec::new();
            let mut indices: BTreeMap<UnitMap, usize> = BTreeMap::new();
            for term in args.iter().flat_map(|arg| arg.iter()) {
                budget.spend(term.cost())?;
                if let Some(&index) = indices.get(&term.units) {
                    values[index].value += term.value;
                } else {
                    indices.insert(term.units.clone(), values.len());
                    values.push(term.clone());
                }
            }
            let mut numeric_type = values[0].numeric_type()?;
            for term in &values[1..] {
                numeric_type = numeric_type
                    .add(term.numeric_type()?)
                    .ok_or(Error::Incompatible)?;
            }
            Ok(values)
        }
        Kind::Product => {
            let mut values = vec![Term {
                value: 1.0,
                units: BTreeMap::new(),
            }];
            for arg in args {
                let count = values.len().checked_mul(arg.len()).ok_or(Error::TooLarge)?;
                budget.spend(count)?;
                let mut next = Vec::with_capacity(count);
                for a in &values {
                    for b in *arg {
                        budget.spend(a.units.len() + b.units.len())?;
                        let mut units = a.units.clone();
                        for (unit, power) in &b.units {
                            let result = units.entry(unit.clone()).or_insert(0);
                            *result = result.checked_add(*power).ok_or(Error::TooLarge)?;
                        }
                        units.retain(|_, power| *power != 0);
                        next.push(Term {
                            value: a.value * b.value,
                            units,
                        });
                    }
                }
                values = next;
            }
            Ok(values)
        }
        Kind::Negate | Kind::Invert => {
            if kind == Kind::Invert && args[0].len() != 1 {
                return Err(Error::Incompatible);
            }
            for term in args[0] {
                budget.spend(term.cost())?;
            }
            let mut values = args[0].to_vec();
            for term in &mut values {
                if kind == Kind::Negate {
                    term.value = -term.value;
                } else {
                    term.value = 1.0 / term.value;
                    for power in term.units.values_mut() {
                        *power = power.checked_neg().ok_or(Error::TooLarge)?;
                    }
                }
            }
            Ok(values)
        }
        Kind::Min | Kind::Max | Kind::Clamp => {
            let first = &args[0][0];
            if args
                .iter()
                .any(|arg| arg.len() != 1 || arg[0].units != first.units)
            {
                return Err(Error::Incompatible);
            }
            budget.spend(first.cost() + args.len())?;
            let mut result = first.clone();
            result.value = if kind == Kind::Clamp {
                // CSS clamp gives the lower bound priority when lower > upper.
                maximum(
                    args[0][0].value,
                    minimum(args[1][0].value, args[2][0].value),
                )
            } else {
                args[1..].iter().fold(first.value, |value, next| {
                    if kind == Kind::Min {
                        minimum(value, next[0].value)
                    } else {
                        maximum(value, next[0].value)
                    }
                })
            };
            Ok(vec![result])
        }
    }
}

fn sum_value(graph: &Graph) -> Result<Vec<Term>, Error> {
    let mut budget = Budget(MAX_WORK);
    let mut results: Vec<Option<Vec<Term>>> = vec![None; graph.nodes.len()];
    let root = graph.roots[0];
    let mut pending = vec![(root, false)];
    while let Some((index, ready)) = pending.pop() {
        if results[index].is_some() {
            continue;
        }
        budget.spend(1)?;
        match &graph.nodes[index] {
            Node::Unit(unit) => results[index] = Some(vec![Term::from_unit(unit)]),
            Node::Math(kind, children) => {
                if !ready {
                    pending.push((index, true));
                    pending.extend(children.iter().rev().map(|&child| (child, false)));
                } else {
                    let args = children
                        .iter()
                        .map(|&child| results[child].as_deref().unwrap())
                        .collect::<Vec<_>>();
                    results[index] = Some(combine(*kind, &args, &mut budget)?);
                }
            }
        }
    }
    Ok(results[root].take().unwrap())
}

/// Convert internal numeric state without invoking an overridable JS method.
pub(in crate::context_bootstrap::css_runtime::typed_om) fn to_unit_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    unit: &str,
) -> Option<f64> {
    let graph = Graph::read(scope, &[object])?;
    let result = sum_value(&graph).and_then(|mut terms| {
        if terms.len() != 1 {
            return Err(Error::Incompatible);
        }
        terms
            .pop()
            .unwrap()
            .into_unit()?
            .convert(unit)
            .ok_or(Error::Incompatible)
    });
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            throw(scope, error);
            None
        }
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSNumericValue.to")]
struct ToArgs {
    #[webidl(required, converter = "usv_string")]
    unit: String,
}

fn convert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    names: Vec<String>,
    single: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let Some(names) = names
        .iter()
        .map(|name| values::normalize_unit_name(name))
        .collect::<Option<Vec<_>>>()
    else {
        webidl::throw_dom_exception(scope, "SyntaxError", "Invalid CSS numeric unit");
        return None;
    };
    let realm = object.get_creation_context(scope)?;
    let graph = Graph::read(scope, &[object])?;
    let values = sum_value(&graph).and_then(|terms| {
        terms
            .into_iter()
            .map(Term::into_unit)
            .collect::<Result<Vec<_>, _>>()
    });
    let mut values = match values {
        Ok(values) => values,
        Err(error) => {
            throw(scope, error);
            return None;
        }
    };
    if single {
        if values.len() == 1
            && let Some(value) = values[0].convert(&names[0])
        {
            return Some(
                Unit {
                    value,
                    unit: names[0].clone(),
                }
                .bind(scope, realm),
            );
        }
        throw(scope, Error::Incompatible);
        return None;
    }
    let mut result = Vec::new();
    if names.is_empty() {
        values.sort_by(|a, b| a.unit.cmp(&b.unit));
        result.extend(values.into_iter().map(|value| value.bind(scope, realm)));
    } else {
        // Greedily consume compatible terms in the requested order; retain
        // explicit zero-valued entries, including repeated requested units.
        for name in names {
            let mut value = 0.0;
            values.retain(|unit| {
                if let Some(converted) = unit.convert(&name) {
                    value += converted;
                    false
                } else {
                    true
                }
            });
            result.push(Unit { value, unit: name }.bind(scope, realm));
        }
        if !values.is_empty() {
            throw(scope, Error::Incompatible);
            return None;
        }
    }
    math_value(scope, Kind::Sum, &result, realm)
}

pub(super) fn to_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ToArgs>(scope, &args) else {
        return;
    };
    if let Some(value) = convert(scope, args.this(), vec![parsed.unit], true) {
        rv.set(value.into());
    }
}

pub(super) fn to_sum_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let mut names = Vec::new();
    for index in 0..args.length() {
        match webidl::convert::<webidl::UsvString>(
            scope,
            args.get(index),
            webidl::Context::argument("CSSNumericValue.toSum", index as usize + 1),
        ) {
            Ok(unit) => names.push(unit.0),
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        }
    }
    if let Some(value) = convert(scope, args.this(), names, false) {
        rv.set(value.into());
    }
}
