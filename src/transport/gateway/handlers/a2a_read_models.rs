use crate::transport::gateway::types::{
    A2A_CAPABILITY_TOOLS, A2A_CONTEXT_ENVELOPE_VERSION, A2A_PROTOCOL_VERSION, A2A_TEXT_OUTPUT_MODE,
    A2aAgentCard, A2aAuthentication, A2aCapabilities, A2aMessageContract, A2aTask,
};

pub(super) fn task_visible_to_principal(
    task: &A2aTask,
    caller_tenant: Option<&str>,
    caller_principal: &str,
) -> bool {
    task.tenant_id.as_deref() == caller_tenant
        && task.owner_principal.as_deref() == Some(caller_principal)
}

pub(super) fn build_a2a_agent_card(auth_mode: &str) -> A2aAgentCard {
    A2aAgentCard {
        schema_version: "a2a-agent-card/v1".to_string(),
        agent_id: "asterel-gateway".to_string(),
        name: "Asterel Gateway Agent".to_string(),
        description: "Gateway A2A surface for text handoff and tool-backed responses.".to_string(),
        version: "v1".to_string(),
        url: "/a2a/v1/messages".to_string(),
        authentication: A2aAuthentication {
            auth_type: auth_mode.to_string(),
        },
        capabilities: A2aCapabilities {
            streaming: false,
            history: false,
            tools: true,
        },
        default_input_modes: vec!["text/plain".to_string(), "application/json".to_string()],
        default_output_modes: vec![A2A_TEXT_OUTPUT_MODE.to_string()],
        required_capabilities: vec![A2A_CAPABILITY_TOOLS.to_string()],
        message_contract: Some(A2aMessageContract {
            protocol_version: A2A_PROTOCOL_VERSION.to_string(),
            context_envelope_version: A2A_CONTEXT_ENVELOPE_VERSION.to_string(),
            supported_output_modes: vec![A2A_TEXT_OUTPUT_MODE.to_string()],
        }),
    }
}
