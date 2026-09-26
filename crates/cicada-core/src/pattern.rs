//! Motor de patrones. Un patrón se aplana una sola vez en "slots" dentro de
//! un ciclo; consultar cualquier rango de tiempo es luego una operación pura.

use crate::ast::{Atom, Step};
use crate::diag::Pos;
use crate::time::Rational;

/// Un paso ya resuelto: dónde empieza dentro del ciclo y cuánto dura, ambos
/// como fracción de ciclo.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub offset: Rational,
    pub duration: Rational,
    pub atom: Atom,
    /// El carácter o la nota del código que genera este paso.
    pub src: Pos,
}

/// Reparte los pasos a lo largo de un ciclo. Los silencios no generan slots.
pub fn flatten(steps: &[Step]) -> Vec<Slot> {
    let mut out = Vec::new();
    sequence(steps, Rational::ZERO, Rational::ONE, &mut out);
    out
}

fn sequence(steps: &[Step], start: Rational, span: Rational, out: &mut Vec<Slot>) {
    let width = span / Rational::int(steps.len() as i64);
    for (i, s) in steps.iter().enumerate() {
        step(s, start + width * Rational::int(i as i64), width, out);
    }
}

fn step(s: &Step, start: Rational, width: Rational, out: &mut Vec<Slot>) {
    match s {
        Step::Atom(a) if a.value == Atom::Rest => {}
        Step::Atom(a) => out.push(Slot {
            offset: start,
            duration: width,
            atom: a.value,
            src: a.pos,
        }),
        Step::Group(inner) => sequence(inner, start, width, out),
        Step::Repeat(inner, n) => {
            let sub = width / Rational::int(*n as i64);
            for k in 0..*n {
                step(inner, start + sub * Rational::int(k as i64), sub, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Item;
    use crate::parser::parse;

    fn slots(pattern: &str) -> Vec<(Rational, Rational)> {
        let file = parse(&format!("kick {pattern}")).unwrap();
        let Item::Pattern(line) = &file.items[0] else {
            panic!()
        };
        flatten(&line.steps)
            .into_iter()
            .map(|s| (s.offset, s.duration))
            .collect()
    }

    fn r(n: i64, d: i64) -> Rational {
        Rational::new(n, d)
    }

    #[test]
    fn steps_split_the_cycle() {
        assert_eq!(slots("x.x."), vec![(r(0, 1), r(1, 4)), (r(1, 2), r(1, 4))]);
    }

    #[test]
    fn groups_subdivide_one_step() {
        assert_eq!(
            slots("x [x x]"),
            vec![(r(0, 1), r(1, 2)), (r(1, 2), r(1, 4)), (r(3, 4), r(1, 4))]
        );
    }

    #[test]
    fn repeat_inside_step() {
        assert_eq!(
            slots("x*3 ."),
            vec![(r(0, 1), r(1, 6)), (r(1, 6), r(1, 6)), (r(1, 3), r(1, 6))]
        );
    }
}
