use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Sub, SubAssign};

use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(deny_unknown_fields)]
pub struct Length {
    ticks: i64,
}

impl Length {
    pub const TICKS_PER_INCH: i64 = 16;
    pub const ZERO: Self = Self { ticks: 0 };

    pub const fn from_ticks(ticks: i64) -> Self {
        Self { ticks }
    }

    pub fn from_inches(inches: f64) -> Self {
        Self {
            ticks: (inches * Self::TICKS_PER_INCH as f64).round() as i64,
        }
    }

    pub fn from_feet(feet: f64) -> Self {
        Self::from_inches(feet * 12.0)
    }

    pub const fn from_whole_inches(inches: i64) -> Self {
        Self {
            ticks: inches * Self::TICKS_PER_INCH,
        }
    }

    pub const fn ticks(self) -> i64 {
        self.ticks
    }

    pub fn inches(self) -> f64 {
        self.ticks as f64 / Self::TICKS_PER_INCH as f64
    }

    pub fn feet(self) -> f64 {
        self.inches() / 12.0
    }

    pub fn max(self, other: Self) -> Self {
        if self >= other { self } else { other }
    }

    pub fn min(self, other: Self) -> Self {
        if self <= other { self } else { other }
    }

    pub fn abs(self) -> Self {
        Self {
            ticks: self.ticks.abs(),
        }
    }
}

impl Add for Length {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            ticks: self.ticks + rhs.ticks,
        }
    }
}

impl AddAssign for Length {
    fn add_assign(&mut self, rhs: Self) {
        self.ticks += rhs.ticks;
    }
}

impl Sub for Length {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            ticks: self.ticks - rhs.ticks,
        }
    }
}

impl SubAssign for Length {
    fn sub_assign(&mut self, rhs: Self) {
        self.ticks -= rhs.ticks;
    }
}

impl Mul<i64> for Length {
    type Output = Self;

    fn mul(self, rhs: i64) -> Self::Output {
        Self {
            ticks: self.ticks * rhs,
        }
    }
}

impl Div<i64> for Length {
    type Output = Self;

    fn div(self, rhs: i64) -> Self::Output {
        Self {
            ticks: self.ticks / rhs,
        }
    }
}

impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut ticks = self.ticks;
        if ticks < 0 {
            write!(f, "-")?;
            ticks = ticks.abs();
        }

        let total_inches = ticks / Self::TICKS_PER_INCH;
        let frac = ticks % Self::TICKS_PER_INCH;
        let feet = total_inches / 12;
        let inches = total_inches % 12;

        if frac == 0 {
            write!(f, "{feet}' {inches}\"")
        } else {
            let divisor = greatest_common_divisor(frac, Self::TICKS_PER_INCH);
            write!(
                f,
                "{feet}' {inches} {}/{}\"",
                frac / divisor,
                Self::TICKS_PER_INCH / divisor
            )
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Point2 {
    pub x: Length,
    pub y: Length,
}

impl Point2 {
    pub const fn new(x: Length, y: Length) -> Self {
        Self { x, y }
    }
}

const fn greatest_common_divisor(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        let next = a % b;
        a = b;
        b = next;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_formats_fractional_inches() {
        assert_eq!(Length::from_inches(97.5).to_string(), "8' 1 1/2\"");
    }

    #[test]
    fn length_serializes_as_ticks() {
        let json = serde_json::to_string(&Length::from_inches(1.5)).unwrap();
        assert_eq!(json, r#"{"ticks":24}"#);
    }
}
