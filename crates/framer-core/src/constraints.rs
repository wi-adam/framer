use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::units::Length;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConstraintVariable {
    owner: String,
    attribute: String,
}

impl ConstraintVariable {
    pub fn new(owner: impl Into<String>, attribute: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            attribute: attribute.into(),
        }
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn attribute(&self) -> &str {
        &self.attribute
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinearExpression {
    terms: BTreeMap<ConstraintVariable, i64>,
}

impl LinearExpression {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn variable(variable: ConstraintVariable, coefficient: i64) -> Self {
        let mut expression = Self::new();
        expression.add_term(variable, coefficient);
        expression
    }

    pub fn add_term(&mut self, variable: ConstraintVariable, coefficient: i64) {
        if coefficient == 0 {
            return;
        }

        let next = self.terms.get(&variable).copied().unwrap_or_default() + coefficient;
        if next == 0 {
            self.terms.remove(&variable);
        } else {
            self.terms.insert(variable, next);
        }
    }

    pub fn add_expression(&mut self, expression: &Self, multiplier: i64) {
        for (variable, coefficient) in &expression.terms {
            self.add_term(variable.clone(), coefficient * multiplier);
        }
    }

    pub fn variables(&self) -> impl Iterator<Item = &ConstraintVariable> {
        self.terms.keys()
    }

    pub fn evaluate(&self, values: &BTreeMap<ConstraintVariable, Length>) -> Option<Length> {
        let mut result = Length::ZERO;
        for (variable, coefficient) in &self.terms {
            result += *values.get(variable)? * *coefficient;
        }
        Some(result)
    }

    fn row(&self, variables: &[ConstraintVariable]) -> Vec<f64> {
        variables
            .iter()
            .map(|variable| self.terms.get(variable).copied().unwrap_or_default() as f64)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearConstraint {
    id: String,
    expression: LinearExpression,
    target: Length,
}

impl LinearConstraint {
    pub fn new(id: impl Into<String>, expression: LinearExpression, target: Length) -> Self {
        Self {
            id: id.into(),
            expression,
            target,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn expression(&self) -> &LinearExpression {
        &self.expression
    }

    pub fn target(&self) -> Length {
        self.target
    }

    pub fn is_satisfied(&self, values: &BTreeMap<ConstraintVariable, Length>) -> Option<bool> {
        self.expression
            .evaluate(values)
            .map(|actual| actual == self.target)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintSystem {
    variables: BTreeSet<ConstraintVariable>,
    constraints: Vec<LinearConstraint>,
}

impl ConstraintSystem {
    pub fn new(variables: impl IntoIterator<Item = ConstraintVariable>) -> Self {
        Self {
            variables: variables.into_iter().collect(),
            constraints: Vec::new(),
        }
    }

    pub fn from_constraints(
        variables: impl IntoIterator<Item = ConstraintVariable>,
        constraints: impl IntoIterator<Item = LinearConstraint>,
    ) -> Self {
        let mut system = Self::new(variables);
        for constraint in constraints {
            system.add_constraint(constraint);
        }
        system
    }

    pub fn add_constraint(&mut self, constraint: LinearConstraint) {
        self.variables
            .extend(constraint.expression.variables().cloned());
        self.constraints.push(constraint);
    }

    pub fn would_overconstrain(&self, candidate: &LinearConstraint) -> bool {
        let variables = self.variables_with(candidate);
        let mut rows = self.rows(&variables);
        let before_rank = matrix_rank(&rows, variables.len());
        rows.push(candidate.expression.row(&variables));
        let after_rank = matrix_rank(&rows, variables.len());
        after_rank == before_rank
    }

    pub fn solve_with_defaults(
        &self,
        defaults: &BTreeMap<ConstraintVariable, Length>,
    ) -> Option<BTreeMap<ConstraintVariable, Length>> {
        let variables = self.variables.iter().cloned().collect::<Vec<_>>();
        let columns = variables.len();
        let mut matrix = self
            .constraints
            .iter()
            .map(|constraint| {
                let mut row = constraint.expression.row(&variables);
                row.push(constraint.target.ticks() as f64);
                row
            })
            .collect::<Vec<_>>();
        let pivots = row_reduce(&mut matrix, columns);

        for row in &matrix {
            let empty_left_side = row[..columns]
                .iter()
                .all(|coefficient| coefficient.abs() <= EPSILON);
            if empty_left_side && row[columns].abs() > EPSILON {
                return None;
            }
        }

        let mut values = variables
            .iter()
            .map(|variable| {
                defaults
                    .get(variable)
                    .map(|value| (variable.clone(), value.ticks() as f64))
            })
            .collect::<Option<BTreeMap<_, _>>>()?;

        for (row, pivot_column) in pivots.into_iter().rev() {
            let mut value = matrix[row][columns];
            for column in 0..columns {
                if column == pivot_column {
                    continue;
                }
                value -= matrix[row][column] * values.get(&variables[column]).copied()?;
            }
            values.insert(variables[pivot_column].clone(), value);
        }

        Some(
            values
                .into_iter()
                .map(|(variable, ticks)| (variable, Length::from_ticks(ticks.round() as i64)))
                .collect(),
        )
    }

    fn variables_with(&self, candidate: &LinearConstraint) -> Vec<ConstraintVariable> {
        let mut variables = self.variables.clone();
        variables.extend(candidate.expression.variables().cloned());
        variables.into_iter().collect()
    }

    fn rows(&self, variables: &[ConstraintVariable]) -> Vec<Vec<f64>> {
        self.constraints
            .iter()
            .map(|constraint| constraint.expression.row(variables))
            .collect()
    }
}

fn matrix_rank(rows: &[Vec<f64>], columns: usize) -> usize {
    if rows.is_empty() || columns == 0 {
        return 0;
    }

    let mut matrix = rows.to_vec();
    row_reduce(&mut matrix, columns).len()
}

const EPSILON: f64 = 1e-9;

fn row_reduce(matrix: &mut [Vec<f64>], columns: usize) -> Vec<(usize, usize)> {
    let mut pivots = Vec::new();
    let mut rank = 0;
    let row_width = matrix.first().map(Vec::len).unwrap_or_default();

    for column in 0..columns {
        let Some(pivot) = (rank..matrix.len()).max_by(|left, right| {
            matrix[*left][column]
                .abs()
                .partial_cmp(&matrix[*right][column].abs())
                .unwrap_or(Ordering::Equal)
        }) else {
            break;
        };

        if matrix[pivot][column].abs() <= EPSILON {
            continue;
        }

        matrix.swap(rank, pivot);
        let pivot_value = matrix[rank][column];
        for value in &mut matrix[rank][column..row_width] {
            *value /= pivot_value;
        }

        for row in 0..matrix.len() {
            if row == rank {
                continue;
            }
            let factor = matrix[row][column];
            if factor.abs() <= EPSILON {
                continue;
            }
            let pivot_tail = matrix[rank][column..row_width].to_vec();
            for (value, pivot_value) in matrix[row][column..row_width]
                .iter_mut()
                .zip(pivot_tail.iter())
            {
                *value -= factor * pivot_value;
            }
        }

        pivots.push((rank, column));
        rank += 1;
    }

    pivots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variable(name: &str) -> ConstraintVariable {
        ConstraintVariable::new("object", name)
    }

    fn expression(terms: impl IntoIterator<Item = (ConstraintVariable, i64)>) -> LinearExpression {
        let mut expression = LinearExpression::new();
        for (variable, coefficient) in terms {
            expression.add_term(variable, coefficient);
        }
        expression
    }

    #[test]
    fn dependent_constraint_overconstrains_existing_system() {
        let x = variable("x");
        let y = variable("y");
        let system = ConstraintSystem::from_constraints(
            [x.clone(), y.clone()],
            [
                LinearConstraint::new(
                    "x",
                    LinearExpression::variable(x.clone(), 1),
                    Length::from_whole_inches(4),
                ),
                LinearConstraint::new(
                    "y",
                    LinearExpression::variable(y.clone(), 1),
                    Length::from_whole_inches(6),
                ),
            ],
        );
        let mut expression = LinearExpression::new();
        expression.add_term(x, 1);
        expression.add_term(y, -1);

        assert!(system.would_overconstrain(&LinearConstraint::new(
            "x-minus-y",
            expression,
            Length::from_whole_inches(-2),
        )));
    }

    #[test]
    fn scaled_dependent_constraint_overconstrains_existing_system() {
        let x = variable("x");
        let y = variable("y");
        let system = ConstraintSystem::from_constraints(
            [x.clone(), y.clone()],
            [LinearConstraint::new(
                "x-plus-y",
                expression([(x.clone(), 1), (y.clone(), 1)]),
                Length::from_whole_inches(10),
            )],
        );

        assert!(system.would_overconstrain(&LinearConstraint::new(
            "two-x-plus-two-y",
            expression([(x, 2), (y, 2)]),
            Length::from_whole_inches(20),
        )));
    }

    #[test]
    fn duplicate_variable_constraint_overconstrains_existing_system() {
        let x = variable("x");
        let system = ConstraintSystem::from_constraints(
            [x.clone()],
            [LinearConstraint::new(
                "x",
                LinearExpression::variable(x.clone(), 1),
                Length::from_whole_inches(4),
            )],
        );

        assert!(system.would_overconstrain(&LinearConstraint::new(
            "x-again",
            LinearExpression::variable(x, 1),
            Length::from_whole_inches(4),
        )));
    }

    #[test]
    fn constraint_mixing_existing_and_new_variables_is_independent() {
        let x = variable("x");
        let y = variable("y");
        let system = ConstraintSystem::from_constraints(
            [x.clone()],
            [LinearConstraint::new(
                "x",
                LinearExpression::variable(x.clone(), 1),
                Length::from_whole_inches(4),
            )],
        );

        assert!(!system.would_overconstrain(&LinearConstraint::new(
            "x-plus-y",
            expression([(x, 1), (y, 1)]),
            Length::from_whole_inches(10),
        )));
    }

    #[test]
    fn independent_constraint_can_introduce_a_new_variable() {
        let x = variable("x");
        let y = variable("y");
        let system = ConstraintSystem::from_constraints(
            [x.clone()],
            [LinearConstraint::new(
                "x",
                LinearExpression::variable(x, 1),
                Length::from_whole_inches(4),
            )],
        );

        assert!(!system.would_overconstrain(&LinearConstraint::new(
            "y",
            LinearExpression::variable(y, 1),
            Length::from_whole_inches(6),
        )));
    }

    #[test]
    fn linear_constraint_can_evaluate_current_values() {
        let x = variable("x");
        let y = variable("y");
        let mut expression = LinearExpression::new();
        expression.add_term(x.clone(), 2);
        expression.add_term(y.clone(), -1);
        let constraint = LinearConstraint::new("twice-x-minus-y", expression, Length::ZERO);
        let values = BTreeMap::from([
            (x, Length::from_whole_inches(5)),
            (y, Length::from_whole_inches(10)),
        ]);

        assert_eq!(constraint.is_satisfied(&values), Some(true));
    }

    #[test]
    fn linear_constraint_reports_unsatisfied_and_unresolved_values() {
        let x = variable("x");
        let y = variable("y");
        let constraint = LinearConstraint::new(
            "x-minus-y",
            expression([(x.clone(), 1), (y.clone(), -1)]),
            Length::from_whole_inches(2),
        );

        assert_eq!(
            constraint.is_satisfied(&BTreeMap::from([
                (x.clone(), Length::from_whole_inches(5)),
                (y.clone(), Length::from_whole_inches(1)),
            ])),
            Some(false)
        );
        assert_eq!(
            constraint.is_satisfied(&BTreeMap::from([(x, Length::from_whole_inches(5))])),
            None
        );
    }

    #[test]
    fn solver_preserves_free_defaults_while_solving_pivots() {
        let center = variable("center");
        let width = variable("width");
        let system = ConstraintSystem::from_constraints(
            [center.clone(), width.clone()],
            [
                LinearConstraint::new(
                    "left",
                    expression([(center.clone(), 2), (width.clone(), -1)]),
                    Length::from_whole_inches(120),
                ),
                LinearConstraint::new(
                    "right",
                    expression([(center.clone(), 2), (width.clone(), 1)]),
                    Length::from_whole_inches(240),
                ),
            ],
        );
        let solution = system
            .solve_with_defaults(&BTreeMap::from([
                (center.clone(), Length::from_whole_inches(80)),
                (width.clone(), Length::from_whole_inches(36)),
            ]))
            .unwrap();

        assert_eq!(solution[&center], Length::from_whole_inches(90));
        assert_eq!(solution[&width], Length::from_whole_inches(60));
    }

    #[test]
    fn solver_keeps_unconstrained_variables_at_defaults() {
        let x = variable("x");
        let y = variable("y");
        let system = ConstraintSystem::from_constraints(
            [x.clone(), y.clone()],
            [LinearConstraint::new(
                "x",
                LinearExpression::variable(x.clone(), 1),
                Length::from_whole_inches(12),
            )],
        );
        let solution = system
            .solve_with_defaults(&BTreeMap::from([
                (x.clone(), Length::from_whole_inches(4)),
                (y.clone(), Length::from_whole_inches(9)),
            ]))
            .unwrap();

        assert_eq!(solution[&x], Length::from_whole_inches(12));
        assert_eq!(solution[&y], Length::from_whole_inches(9));
    }

    #[test]
    fn solver_rejects_inconsistent_constraints() {
        let x = variable("x");
        let system = ConstraintSystem::from_constraints(
            [x.clone()],
            [
                LinearConstraint::new(
                    "x-four",
                    LinearExpression::variable(x.clone(), 1),
                    Length::from_whole_inches(4),
                ),
                LinearConstraint::new(
                    "x-five",
                    LinearExpression::variable(x.clone(), 1),
                    Length::from_whole_inches(5),
                ),
            ],
        );

        assert_eq!(
            system.solve_with_defaults(&BTreeMap::from([(x, Length::ZERO)])),
            None
        );
    }

    #[test]
    fn solver_requires_defaults_for_variables() {
        let x = variable("x");
        let y = variable("y");
        let system = ConstraintSystem::from_constraints(
            [x.clone(), y.clone()],
            [LinearConstraint::new(
                "x",
                LinearExpression::variable(x.clone(), 1),
                Length::from_whole_inches(12),
            )],
        );

        assert_eq!(
            system.solve_with_defaults(&BTreeMap::from([(x, Length::ZERO)])),
            None
        );
    }

    #[test]
    fn expression_combines_and_cancels_terms() {
        let x = variable("x");
        let y = variable("y");
        let mut first = expression([(x.clone(), 2), (y.clone(), 1)]);
        let second = expression([(x.clone(), 2), (y.clone(), 1)]);

        first.add_expression(&second, -1);

        assert_eq!(
            first.evaluate(&BTreeMap::from([
                (x, Length::from_whole_inches(5)),
                (y, Length::from_whole_inches(7)),
            ])),
            Some(Length::ZERO)
        );
        assert_eq!(first.variables().count(), 0);
    }
}
