//! Memory consistency subsystem: contradiction detection and resolution.
//!
//! When a new inferred event arrives, [`ConsistencyService`] compares it
//! against existing belief slots to detect conflicting facts. Contradicted
//! claims are marked before persistence so the write path can apply the
//! configured resolution strategy (e.g., supersede the older slot).

mod detector;
mod service;
mod types;

pub(crate) use detector::{ContradictionDetector, SlotValueDetector};
pub(crate) use service::{ConsistencyService, SlotValueConsistencyService};
pub(crate) use types::{Claim, ContradictionFinding};
