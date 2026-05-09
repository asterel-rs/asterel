//! Transport-local bridge for companion plugin contracts used by the
//! gateway surfaces.
pub(crate) use crate::plugins::companion::context::{
    CompanionContextIngressDecision, CompanionContextIngressGate, CompanionContextIngressReason,
    CompanionContextKind, CompanionCtxEvent,
};
pub(crate) use crate::plugins::companion::multimodal::{
    CompanionEmotionalImpact, CompanionMediaKind, CompanionMultimodalMemoryRecord,
};
#[cfg(test)]
pub(crate) use crate::plugins::companion::surface::CompanionAction;
pub(crate) use crate::plugins::companion::surface::{
    CompanionCaptionChannel, CompanionCaptionEvt, CompanionWidgetCommand, CompanionWidgetRuntime,
    CompanionWidgetRuntimeResult, CompanionWidgetState, CompanionWindow,
};

pub(crate) struct CompanionContextBridgeInput {
    pub session_id: crate::contracts::ids::SessionId,
    pub tab_id: String,
    pub kind: CompanionContextKind,
    pub topic: String,
    pub source: String,
    pub source_url: Option<String>,
    pub media_ref: Option<String>,
    pub payload: serde_json::Value,
}

pub(crate) fn build_context_event(
    input: CompanionContextBridgeInput,
) -> Result<CompanionCtxEvent, String> {
    CompanionCtxEvent::new(crate::plugins::companion::context::CompanionCtxInput {
        session_id: input.session_id,
        tab_id: input.tab_id,
        kind: input.kind,
        topic: input.topic,
        source: input.source,
        source_url: input.source_url,
        media_ref: input.media_ref,
        payload: input.payload,
    })
    .map_err(|error| error.to_string())
}
