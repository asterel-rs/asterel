//! Cognitive context shared with introspective tools during the
//! tool loop, granting read/write access to internal cognitive state.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::contracts::affect::AffectReading;
use crate::contracts::ids::{EntityId, PersonId};
use crate::core::affect::desire::DesireState;
use crate::core::memory::Memory;
use crate::core::persona::scaffolding::ScaffoldingState;

const MAX_TIER1_CALLS: u32 = 5;
const MAX_TIER2_CALLS: u32 = 2;
const MAX_TIER3_CALLS: u32 = 1;

pub(crate) struct CognitiveContext {
    pub(crate) memory: Arc<dyn Memory>,
    pub(crate) entity_id: EntityId,
    pub(crate) person_id: PersonId,
    pub(crate) affect_reading: AffectReading,
    pub(crate) desire_state: DesireState,
    pub(crate) scaffolding: ScaffoldingState,
    pub(crate) persona_spec: String,
    tier1_calls: AtomicU32,
    tier2_calls: AtomicU32,
    tier3_calls: AtomicU32,
}

impl CognitiveContext {
    pub(crate) fn try_tier1_call(&self) -> bool {
        let prev = self.tier1_calls.load(Ordering::Relaxed);
        if prev < MAX_TIER1_CALLS {
            self.tier1_calls.store(prev + 1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    pub(crate) fn try_tier2_call(&self) -> bool {
        let prev = self.tier2_calls.load(Ordering::Relaxed);
        if prev < MAX_TIER2_CALLS {
            self.tier2_calls.store(prev + 1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    pub(crate) fn try_tier3_call(&self) -> bool {
        let prev = self.tier3_calls.load(Ordering::Relaxed);
        if prev < MAX_TIER3_CALLS {
            self.tier3_calls.store(prev + 1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}
