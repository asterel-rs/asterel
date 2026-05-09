//! Bounded score newtypes for persisted / external-boundary values.
//!
//! All types clamp to `[0.0, 1.0]` on construction and use
//! `#[serde(transparent)]` so serialized representations remain plain
//! `f64` values.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! define_score {
    (
        $(#[$meta:meta])*
        $name:ident
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(f64);

        impl $name {
            pub const ZERO: Self = Self(0.0);
            pub const ONE: Self = Self(1.0);

            #[must_use]
            pub fn new(value: f64) -> Self {
                if value.is_nan() {
                    Self::ZERO
                } else {
                    Self(value.clamp(0.0, 1.0))
                }
            }

            #[must_use]
            pub fn get(self) -> f64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{:.4}", self.0)
            }
        }

        impl From<f64> for $name {
            fn from(v: f64) -> Self {
                Self::new(v)
            }
        }

        impl From<$name> for f64 {
            fn from(s: $name) -> Self {
                s.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::ZERO
            }
        }
    };
}

define_score! { Confidence }
define_score! { Importance }

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_f64_close(left: f64, right: f64) {
        assert!((left - right).abs() < f64::EPSILON);
    }

    #[test]
    fn clamping_on_construction() {
        assert_f64_close(Confidence::new(1.5).get(), 1.0);
        assert_f64_close(Confidence::new(-0.3).get(), 0.0);
        assert_f64_close(Importance::new(0.7).get(), 0.7);
    }

    #[test]
    fn serde_round_trip() {
        let c = Confidence::new(0.85);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "0.85");
        let back: Confidence = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn default_is_zero() {
        assert_f64_close(Confidence::default().get(), 0.0);
        assert_f64_close(Importance::default().get(), 0.0);
    }

    #[test]
    fn from_f64_clamps() {
        let c: Confidence = 2.0_f64.into();
        assert_f64_close(c.get(), 1.0);
    }

    #[test]
    fn nan_construction_falls_back_to_zero() {
        assert_f64_close(Confidence::new(f64::NAN).get(), 0.0);
        assert_f64_close(Importance::new(f64::NAN).get(), 0.0);
    }

    #[test]
    fn infinite_construction_clamps_to_bounds() {
        assert_f64_close(Confidence::new(f64::INFINITY).get(), 1.0);
        assert_f64_close(Importance::new(f64::NEG_INFINITY).get(), 0.0);
    }

    #[test]
    fn into_f64() {
        let c = Confidence::new(0.42);
        let v: f64 = c.into();
        assert!((v - 0.42).abs() < f64::EPSILON);
    }

    #[test]
    fn ordering() {
        let a = Importance::new(0.3);
        let b = Importance::new(0.7);
        assert!(a < b);
    }

    #[test]
    fn display_format() {
        let c = Confidence::new(0.5);
        assert_eq!(format!("{c}"), "0.5000");
    }
}
