use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, Div, Mul, Sub};

use serde::{Serialize, Serializer};

/// Fracción exacta. Todo el tiempo musical de CICADA se mide en ciclos con
/// este tipo, así un paso de 1/3 vale exactamente 1/3 y no se acumula deriva.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rational {
    num: i64,
    den: i64,
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Rational {
    pub const ZERO: Rational = Rational { num: 0, den: 1 };
    pub const ONE: Rational = Rational { num: 1, den: 1 };

    pub fn new(num: i64, den: i64) -> Self {
        Self::reduce(num as i128, den as i128)
    }

    pub fn int(n: i64) -> Self {
        Rational { num: n, den: 1 }
    }

    fn reduce(num: i128, den: i128) -> Self {
        assert!(den != 0, "fracción con denominador cero");
        let g = gcd(num, den).max(1);
        let sign = if den < 0 { -1 } else { 1 };
        Rational {
            num: (sign * num / g) as i64,
            den: (sign * den / g) as i64,
        }
    }

    pub fn num(self) -> i64 {
        self.num
    }

    pub fn den(self) -> i64 {
        self.den
    }

    pub fn floor(self) -> i64 {
        self.num.div_euclid(self.den)
    }

    pub fn ceil(self) -> i64 {
        -(-self.num).div_euclid(self.den)
    }

    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

impl Add for Rational {
    type Output = Rational;
    fn add(self, o: Rational) -> Rational {
        Rational::reduce(
            self.num as i128 * o.den as i128 + o.num as i128 * self.den as i128,
            self.den as i128 * o.den as i128,
        )
    }
}

impl Sub for Rational {
    type Output = Rational;
    fn sub(self, o: Rational) -> Rational {
        Rational::reduce(
            self.num as i128 * o.den as i128 - o.num as i128 * self.den as i128,
            self.den as i128 * o.den as i128,
        )
    }
}

impl Mul for Rational {
    type Output = Rational;
    fn mul(self, o: Rational) -> Rational {
        Rational::reduce(
            self.num as i128 * o.num as i128,
            self.den as i128 * o.den as i128,
        )
    }
}

impl Div for Rational {
    type Output = Rational;
    fn div(self, o: Rational) -> Rational {
        Rational::reduce(
            self.num as i128 * o.den as i128,
            self.den as i128 * o.num as i128,
        )
    }
}

/// En la IR una fracción se escribe `[num, den]`: exacta y fácil de leer desde JS.
impl Serialize for Rational {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        [self.num, self.den].serialize(s)
    }
}

impl Ord for Rational {
    fn cmp(&self, o: &Self) -> Ordering {
        // Los denominadores siempre son positivos, así que basta con multiplicar en cruz.
        (self.num as i128 * o.den as i128).cmp(&(o.num as i128 * self.den as i128))
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_is_exact() {
        let third = Rational::new(1, 3);
        assert_eq!(third + third + third, Rational::ONE);
        assert_eq!(Rational::new(2, 4), Rational::new(1, 2));
        assert_eq!(Rational::new(1, -2), Rational::new(-1, 2));
        assert_eq!(
            Rational::ONE / Rational::int(8) * Rational::int(3),
            Rational::new(3, 8)
        );
    }

    #[test]
    fn floor_and_ceil() {
        assert_eq!(Rational::new(7, 2).floor(), 3);
        assert_eq!(Rational::new(7, 2).ceil(), 4);
        assert_eq!(Rational::int(3).ceil(), 3);
        assert_eq!(Rational::new(-1, 2).floor(), -1);
    }
}
