//! Strongly-typed domain identifier wrappers.
//!
//! All types use `#[serde(transparent)]` so serialized representations
//! remain unchanged.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! define_id {
    (
        $(#[$meta:meta])*
        $name:ident
    ) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

define_id! {
    /// Identifies a persisted conversation session.
    ///
    /// The session store scopes reuse and lookup through a separate `surface`
    /// plus `owner_scope` grammar (unscoped, tenant, principal, or
    /// tenant-principal). The ID itself is only the stable handle for the
    /// resulting transcript and metadata record.
    SessionId
}

define_id! {
    /// Identifies any actor in the system: a user, agent, subagent, or channel.
    ///
    /// `EntityId` is the broadest identity primitive. It is used wherever a
    /// component needs to reference "something that acts or is acted upon"
    /// without constraining the actor kind at the type level. Prefer the
    /// narrower `UserId`, `ChannelId`, or operator-specific IDs when the kind
    /// is statically known.
    EntityId
}

define_id! {
    /// Identifies a communication channel (e.g., a Discord guild+channel pair,
    /// a Telegram chat, or a Slack channel).
    ///
    /// Scopes incoming messages and outgoing replies to a specific transport
    /// endpoint. Memory and session state are partitioned by channel so that
    /// activity in one channel does not bleed into another.
    ChannelId
}

define_id! {
    /// Identifies a human or service user in the system.
    ///
    /// Distinct from `EntityId` to make user-specific policy checks (trust,
    /// rate-limits, permissions) unambiguous at the type level.
    UserId
}

define_id! {
    /// Identifies an individual message within a channel or session.
    ///
    /// Used for reference, threading, and deduplication when the memory or
    /// event layer needs to trace back to the originating message.
    MessageId
}

define_id! {
    /// Identifies a discrete event in the event ledger.
    ///
    /// Events are immutable records of things that happened (tool calls,
    /// memory mutations, session transitions). `EventId` allows subscribers
    /// and audit queries to refer to a specific occurrence.
    EventId
}

define_id! {
    /// Namespaced key addressing a single memory slot within an entity's store.
    ///
    /// Slots are the fundamental addressable unit of persistent memory. A key
    /// conventionally uses dot-separated namespaces (e.g.,
    /// `"user.preference.language"`) so that related slots can be listed or
    /// invalidated by prefix.
    SlotKey
}

define_id! {
    /// Identifies a single agentic run (one invocation of the tool loop).
    ///
    /// A run begins when the agent receives a trigger and ends when the loop
    /// exits normally or is interrupted. `RunId` is used to correlate log
    /// lines, tool calls, and memory events that belong to the same execution.
    RunId
}

define_id! {
    /// Identifies a companion persona within the system.
    ///
    /// A persona is a named personality configuration (e.g., `"local-default"`)
    /// that owns its own style profile, affect topology, state header, and
    /// relationship model. `PersonId` is distinct from `EntityId` because a
    /// single entity (user or agent) may interact with multiple personas, and
    /// persona-specific state (drift score, writeback history) must not be
    /// confused with entity-level state (trust score, rate limits).
    PersonId
}

define_id! {
    /// Identifies a single tool-call action within a run.
    ///
    /// Every tool invocation gets an `ActionId` so that results, retries, and
    /// approval records can be matched back to the exact call that triggered
    /// them.
    ActionId
}

define_id! {
    /// Identifies an observation recorded during a run.
    ///
    /// Observations are the outputs of tool calls and environment probes. They
    /// are stored separately from request/run/action IDs so the reasoning layer
    /// can retrieve them independently for context or evidence.
    ObservationId
}

define_id! {
    /// Identifies a test scenario in the evaluation harness.
    ///
    /// Scenarios define the inputs, expected behaviours, and success criteria
    /// used for offline evaluation. `ScenarioId` links run results back to the
    /// scenario definition that produced them.
    ScenarioId
}

define_id! {
    /// Identifies an inbound API or user request before it is dispatched.
    ///
    /// Assigned at the ingress boundary so that the entire processing pipeline
    /// — routing, tool loop, response — can be correlated by a single token
    /// without coupling to session or run lifetime.
    RequestId
}

define_id! {
    /// Identifies a piece of evidence supporting a belief or claim.
    ///
    /// Evidence items are attached to inferred facts or contradiction reports
    /// so that the memory layer can trace why a belief was held or revised.
    EvidenceId
}

define_id! {
    /// Identifies a specific case within a scenario or evaluation suite.
    ///
    /// A scenario may contain multiple cases that exercise different input
    /// permutations. `CaseId` scopes results and artefacts to the individual
    /// case rather than the parent scenario.
    CaseId
}

define_id! {
    /// Identifies a human operator who configured or is supervising the system.
    ///
    /// Operators are distinct from regular users: they set policy (allowed
    /// commands, autonomy level, secret store paths) and may receive
    /// approval-request notifications. `OperatorId` is used in audit logs and
    /// approval workflows to record which operator authorised an action.
    OperatorId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_round_trip() {
        let id = SessionId::new("ses-001");
        assert_eq!(id.as_str(), "ses-001");
        assert_eq!(id.to_string(), "ses-001");

        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"ses-001\"");
        let back: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn action_id_round_trip() {
        let id = ActionId::new("act-001");
        assert_eq!(id.as_str(), "act-001");
        assert_eq!(id.to_string(), "act-001");

        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"act-001\"");
        let back: ActionId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn observation_id_round_trip() {
        let id = ObservationId::new("obs-001");
        assert_eq!(id.as_str(), "obs-001");
        assert_eq!(id.to_string(), "obs-001");

        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"obs-001\"");
        let back: ObservationId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn scenario_id_round_trip() {
        let id = ScenarioId::new("scn-001");
        assert_eq!(id.as_str(), "scn-001");
        assert_eq!(id.to_string(), "scn-001");

        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"scn-001\"");
        let back: ScenarioId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn request_id_round_trip() {
        let id = RequestId::new("req-001");
        assert_eq!(id.as_str(), "req-001");
        assert_eq!(id.to_string(), "req-001");

        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"req-001\"");
        let back: RequestId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn evidence_id_round_trip() {
        let id = EvidenceId::new("evid-001");
        assert_eq!(id.as_str(), "evid-001");
        assert_eq!(id.to_string(), "evid-001");

        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"evid-001\"");
        let back: EvidenceId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn entity_id_from_conversions() {
        let from_str: EntityId = "ent-42".into();
        let from_string: EntityId = String::from("ent-42").into();
        assert_eq!(from_str, from_string);

        let back: String = from_str.into();
        assert_eq!(back, "ent-42");
    }

    #[test]
    fn ids_are_distinct_types() {
        let _s = SessionId::new("abc");
        let _e = EntityId::new("abc");
    }

    #[test]
    fn ordering_works() {
        let a = SlotKey::new("aaa");
        let b = SlotKey::new("bbb");
        assert!(a < b);
    }

    #[test]
    fn hash_in_collections() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(ChannelId::new("ch-1"));
        set.insert(ChannelId::new("ch-2"));
        set.insert(ChannelId::new("ch-1"));
        assert_eq!(set.len(), 2);
    }
}
