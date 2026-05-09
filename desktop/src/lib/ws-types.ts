// ---------------------------------------------------------------------------
// WebSocket protocol types — matches daemon ws::ServerEvent / ClientMessage
// ---------------------------------------------------------------------------

export interface ChatAttachment {
  upload_id: string;
  filename: string;
  content_type: string;
}

export type ClientMessage =
  | {
      type: "chat";
      session_id?: string;
      message: string;
      attachments?: ChatAttachment[];
    }
  | { type: "typing"; session_id?: string }
  | { type: "ping" };

export interface ServerEventEnvelope {
  type: string;
  tenant_id?: string;
  ts?: string;
  payload: unknown;
}

// --- Payload types ---

export interface ChatResponsePayload {
  session_id: string;
  content: string;
  input_tokens?: number;
  output_tokens?: number;
}

export interface MessageCreatedPayload {
  session_id: string;
  message_id: string;
  role: string;
  content?: string;
  created_at?: string;
}

export interface MessageDeltaPayload {
  session_id: string;
  message_id: string;
  delta: string;
}

export interface MessageCompletedPayload {
  session_id: string;
  message_id: string;
  content?: string;
  input_tokens?: number;
  output_tokens?: number;
}

export interface TypingPayload {
  session_id?: string;
  agent?: boolean;
  is_typing?: boolean;
}

export interface SessionUpdatedPayload {
  session_id: string;
  title?: string;
  updated_at?: string;
}

export interface ToolCallUpdatedPayload {
  session_id: string;
  tool_call_id: string;
  tool_name: string;
  status: "running" | "completed" | "failed";
  detail?: string;
}

export interface ConnectedPayload {
  connection_id: string;
}

export interface ErrorPayload {
  code: string;
  message: string;
}

export interface AgentStatePayload {
  agent_id: string;
  state: string;
  detail?: string;
}

export type ServerEventType =
  | "chat_response"
  | "message_created"
  | "message_delta"
  | "message_completed"
  | "typing"
  | "session_updated"
  | "tool_call_updated"
  | "agent_state"
  | "connected"
  | "error"
  | "pong";
