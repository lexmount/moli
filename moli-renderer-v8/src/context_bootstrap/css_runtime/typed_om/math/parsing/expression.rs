use super::*;
use std::collections::BTreeMap;

enum Node {
    Unit(Unit),
    Math(Kind, Vec<Expression>),
}

pub(super) struct Expression {
    node: Node,
    numeric_type: NumericType,
}

impl Expression {
    pub fn unit(mut unit: Unit, canonicalize: bool) -> Self {
        if canonicalize {
            let (name, ratio) = conversion::canonical(&unit.unit);
            unit.value *= ratio;
            unit.unit = name.to_owned();
        }
        Self {
            numeric_type: NumericType::from_unit(&unit.unit).expect("validated CSS numeric unit"),
            node: Node::Unit(unit),
        }
    }

    pub fn is_unit(&self) -> bool {
        matches!(self.node, Node::Unit(_))
    }

    pub fn unsimplified(kind: Kind, children: Vec<Self>) -> Result<Self, Error> {
        let mut numeric_type = children.first().ok_or(Error::Syntax)?.numeric_type;
        if kind == Kind::Invert {
            numeric_type = numeric_type.invert().ok_or(Error::Syntax)?;
        } else {
            for child in &children[1..] {
                numeric_type = if kind == Kind::Product {
                    numeric_type.multiply(child.numeric_type)
                } else {
                    numeric_type.add(child.numeric_type)
                }
                .ok_or(Error::Syntax)?;
            }
        }
        Ok(Self {
            node: Node::Math(kind, children),
            numeric_type,
        })
    }

    // CSS Values calculation-tree simplification is distinct from the public
    // Typed OM arithmetic methods: it canonicalizes and combines like units,
    // flattens nested operators, and distributes scalar multiplication.
    pub fn operation(kind: Kind, children: Vec<Self>, budget: &mut Budget) -> Result<Self, Error> {
        budget.spend(1 + children.len())?;
        // Check the complete expression before simplification can erase a
        // zero-valued term or an incompatible branch of a comparison.
        let expression = Self::unsimplified(kind, children)?;
        let Node::Math(_, mut children) = expression.node else {
            unreachable!()
        };
        match kind {
            Kind::Negate | Kind::Invert => {
                let child = children.pop().unwrap();
                match child.node {
                    Node::Unit(mut unit) if kind == Kind::Negate || unit.unit == "number" => {
                        unit.value = if kind == Kind::Negate {
                            -unit.value
                        } else {
                            1.0 / unit.value
                        };
                        Ok(Self::unit(unit, false))
                    }
                    Node::Math(child_kind, mut values) if child_kind == kind => {
                        Ok(values.pop().unwrap())
                    }
                    Node::Math(Kind::Sum, values) if kind == Kind::Negate => {
                        let values = values
                            .into_iter()
                            .map(|value| Self::operation(Kind::Negate, vec![value], budget))
                            .collect::<Result<_, _>>()?;
                        Self::operation(Kind::Sum, values, budget)
                    }
                    _ => Self::unsimplified(kind, vec![child]),
                }
            }
            Kind::Sum | Kind::Product => {
                let mut flat = Vec::new();
                for child in children {
                    match child.node {
                        Node::Math(child_kind, values) if child_kind == kind => flat.extend(values),
                        _ => flat.push(child),
                    }
                }
                budget.spend(flat.len())?;
                if kind == Kind::Sum {
                    Self::sum(flat)
                } else {
                    Self::product(flat, budget)
                }
            }
            Kind::Min | Kind::Max => Self::comparison(kind, children),
            Kind::Clamp => {
                if let [a, b, c] = children.as_slice()
                    && let (Node::Unit(a), Node::Unit(b), Node::Unit(c)) =
                        (&a.node, &b.node, &c.node)
                    && comparable(a)
                    && a.unit == b.unit
                    && a.unit == c.unit
                {
                    return Ok(Self::unit(
                        Unit {
                            value: operations::maximum(
                                a.value,
                                operations::minimum(b.value, c.value),
                            ),
                            unit: a.unit.clone(),
                        },
                        false,
                    ));
                }
                Self::unsimplified(kind, children)
            }
        }
    }

    fn sum(children: Vec<Self>) -> Result<Self, Error> {
        let mut result: Vec<Self> = Vec::new();
        let mut positions: BTreeMap<String, usize> = BTreeMap::new();
        for child in children {
            if let Node::Unit(unit) = &child.node {
                if let Some(&index) = positions.get(&unit.unit) {
                    let Node::Unit(previous) = &mut result[index].node else {
                        unreachable!()
                    };
                    previous.value += unit.value;
                    continue;
                }
                positions.insert(unit.unit.clone(), result.len());
            }
            result.push(child);
        }
        Self::collapse(Kind::Sum, result)
    }

    fn product(children: Vec<Self>, budget: &mut Budget) -> Result<Self, Error> {
        let mut number: Option<usize> = None;
        let mut combined: Vec<Self> = Vec::with_capacity(children.len());
        for child in children {
            if let Node::Unit(unit) = &child.node
                && unit.unit == "number"
            {
                if let Some(previous) = number {
                    let Node::Unit(target) = &mut combined[previous].node else {
                        unreachable!()
                    };
                    target.value *= unit.value;
                    continue;
                }
                number = Some(combined.len());
            }
            combined.push(child);
        }
        let mut children = combined;
        if children.len() == 2
            && let Some(number) = number
        {
            let other = 1 - number;
            if let Node::Math(Kind::Sum, terms) = &children[other].node
                && terms.iter().all(Self::is_unit)
            {
                let Node::Unit(scalar) = &children[number].node else {
                    unreachable!()
                };
                let value = scalar.value;
                let Node::Math(_, terms) = children.remove(other).node else {
                    unreachable!()
                };
                budget.spend(terms.len())?;
                let terms = terms
                    .into_iter()
                    .map(|term| {
                        let Node::Unit(mut unit) = term.node else {
                            unreachable!()
                        };
                        unit.value *= value;
                        Self::unit(unit, false)
                    })
                    .collect();
                return Self::collapse(Kind::Sum, terms);
            }
        }
        // Cancel units only when their conversion is independent of layout.
        // px/em remains an expression even though its dimensional type is a
        // number. Matching relative units can cancel one another.
        let mut units: BTreeMap<&str, i32> = BTreeMap::new();
        let mut value = 1.0;
        let mut all_numeric = true;
        for child in &children {
            let (unit, power) = match &child.node {
                Node::Unit(unit) => (unit, 1),
                Node::Math(Kind::Invert, values) => match &values[0].node {
                    Node::Unit(unit) => (unit, -1),
                    _ => {
                        all_numeric = false;
                        break;
                    }
                },
                _ => {
                    all_numeric = false;
                    break;
                }
            };
            value *= if power == 1 {
                unit.value
            } else {
                1.0 / unit.value
            };
            if unit.unit != "number" {
                *units.entry(&unit.unit).or_default() += power;
            }
        }
        units.retain(|_, power| *power != 0);
        if all_numeric
            && (units.is_empty() || (units.len() == 1 && units.values().next() == Some(&1)))
        {
            let unit = units.keys().next().copied().unwrap_or("number").to_owned();
            return Ok(Self::unit(Unit { value, unit }, false));
        }
        Self::collapse(Kind::Product, children)
    }

    fn comparison(kind: Kind, children: Vec<Self>) -> Result<Self, Error> {
        let mut result: Vec<Self> = Vec::new();
        let mut positions: BTreeMap<String, usize> = BTreeMap::new();
        for child in children {
            if let Node::Unit(unit) = &child.node
                && comparable(unit)
            {
                if let Some(&index) = positions.get(&unit.unit) {
                    let Node::Unit(previous) = &mut result[index].node else {
                        unreachable!()
                    };
                    previous.value = if kind == Kind::Min {
                        operations::minimum(previous.value, unit.value)
                    } else {
                        operations::maximum(previous.value, unit.value)
                    };
                    continue;
                }
                positions.insert(unit.unit.clone(), result.len());
            }
            result.push(child);
        }
        Self::collapse(kind, result)
    }

    fn collapse(kind: Kind, mut children: Vec<Self>) -> Result<Self, Error> {
        if children.len() == 1 {
            Ok(children.pop().unwrap())
        } else {
            Self::unsimplified(kind, children)
        }
    }

    pub fn bind<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
        let realm = scope.get_current_context();
        match self.node {
            Node::Unit(unit) => Some(unit.bind(scope, realm)),
            Node::Math(kind, children) => {
                let mut values = Vec::with_capacity(children.len());
                for (index, mut child) in children.into_iter().enumerate() {
                    // Reify subtraction in a sum as a native negate operand.
                    let negative = kind == Kind::Sum
                        && index != 0
                        && matches!(&child.node, Node::Unit(unit) if unit.value < 0.0);
                    if negative && let Node::Unit(unit) = &mut child.node {
                        unit.value = -unit.value;
                    }
                    let mut value = child.bind(scope)?;
                    if negative {
                        value = math_value(scope, Kind::Negate, &[value], realm)?;
                    }
                    values.push(value);
                }
                math_value(scope, kind, &values, realm)
            }
        }
    }
}

fn comparable(unit: &Unit) -> bool {
    // Percentages can have a negative basis; relative lengths still require
    // contextual information. Neither can be compared at parse time.
    matches!(
        unit.unit.as_str(),
        "number" | "px" | "deg" | "s" | "hz" | "dppx"
    )
}
