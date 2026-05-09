import { useCallback, useEffect, useRef } from "react";
import { useChatStore } from "@/stores/chat";
import { useConnectionStore } from "@/stores/connection";
import type {
  AgentStatePayload,
  ChatResponsePayload,
  ClientMessage,
  ErrorPayload,
  MessageCompletedPayload,
  MessageCreatedPayload,
  MessageDeltaPayload,
  ServerEventEnvelope,
  ToolCallUpdatedPayload,
  TypingPayload,
} from "./ws-types";

export type WsStatus = "connecting" | "connected" | "disconnected";

const PING_INTERVAL_MS = 25_000;
const INITIAL_BACKOFF_MS = 1_000;
const MAX_BACKOFF_MS = 15_000;

interface UseDaemonWsOptions {
  onEvent?: (envelope: ServerEventEnvelope) => void;
}

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function buildWsUrl(token: string): string {
  if (isTauri()) {
    return `ws://127.0.0.1:3000/ws?token=${encodeURIComponent(token)}`;
  }
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${window.location.host}/ws?token=${encodeURIComponent(token)}`;
}

export function useDaemonWs(options: UseDaemonWsOptions = {}) {
  const token = useConnectionStore((s) => s.token);
  const connectionStatus = useConnectionStore((s) => s.status);
  const wsReconnectVersion = useConnectionStore((s) => s.wsReconnectVersion);
  const wsRef = useRef<WebSocket | null>(null);
  const pingRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const backoffRef = useRef(INITIAL_BACKOFF_MS);
  const reconnectRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const deltaBufferRef = useRef<Map<string, string>>(new Map());
  const deltaFrameRef = useRef<number | null>(null);
  const onEventRef = useRef(options.onEvent);
  const mountedRef = useRef(true);

  useEffect(() => {
    onEventRef.current = options.onEvent;
  }, [options.onEvent]);

  const flushDeltaBuffer = useCallback((messageId?: string) => {
    const store = useChatStore.getState();

    if (messageId !== undefined) {
      const delta = deltaBufferRef.current.get(messageId);
      if (!delta) {
        return;
      }
      deltaBufferRef.current.delete(messageId);
      store.appendDelta(messageId, delta);
      return;
    }

    for (const [bufferedMessageId, delta] of deltaBufferRef.current.entries()) {
      store.appendDelta(bufferedMessageId, delta);
    }
    deltaBufferRef.current.clear();
  }, []);

  const scheduleDeltaFlush = useCallback(() => {
    if (deltaFrameRef.current !== null) {
      return;
    }

    deltaFrameRef.current = requestAnimationFrame(() => {
      deltaFrameRef.current = null;
      flushDeltaBuffer();
    });
  }, [flushDeltaBuffer]);

  const cleanup = useCallback(() => {
    if (pingRef.current) {
      clearInterval(pingRef.current);
      pingRef.current = null;
    }
    if (reconnectRef.current) {
      clearTimeout(reconnectRef.current);
      reconnectRef.current = null;
    }
    if (wsRef.current) {
      wsRef.current.onopen = null;
      wsRef.current.onclose = null;
      wsRef.current.onerror = null;
      wsRef.current.onmessage = null;
      wsRef.current.close();
      wsRef.current = null;
    }
    if (deltaFrameRef.current !== null) {
      cancelAnimationFrame(deltaFrameRef.current);
      deltaFrameRef.current = null;
    }
    deltaBufferRef.current.clear();
  }, []);

  const handleMessage = useCallback(
    (event: MessageEvent) => {
      let envelope: ServerEventEnvelope;
      try {
        envelope = JSON.parse(event.data as string) as ServerEventEnvelope;
      } catch {
        return;
      }

      const store = useChatStore.getState();
      const payload = envelope.payload;

      switch (envelope.type) {
        case "chat_response": {
          const p = payload as ChatResponsePayload;
          const msgId = `resp-${Date.now()}`;
          store.appendMessage({
            id: msgId,
            role: "assistant",
            content: p.content,
            created_at: envelope.ts ?? new Date().toISOString(),
            ...(p.input_tokens !== undefined && { input_tokens: p.input_tokens }),
            ...(p.output_tokens !== undefined && { output_tokens: p.output_tokens }),
          });
          // chat_response is a complete message, not streaming — clear immediately
          store.completeMessage(msgId, p.content, p.input_tokens, p.output_tokens);
          if (p.session_id) {
            store.adoptSessionId(p.session_id);
          }
          break;
        }
        case "message_created": {
          const p = payload as MessageCreatedPayload;
          store.appendMessage({
            id: p.message_id,
            role: p.role,
            content: p.content ?? "",
            created_at: p.created_at ?? envelope.ts ?? new Date().toISOString(),
          });
          break;
        }
        case "message_delta": {
          const p = payload as MessageDeltaPayload;
          deltaBufferRef.current.set(
            p.message_id,
            `${deltaBufferRef.current.get(p.message_id) ?? ""}${p.delta}`,
          );
          scheduleDeltaFlush();
          break;
        }
        case "message_completed": {
          const p = payload as MessageCompletedPayload;
          const finalContent =
            p.content ??
            deltaBufferRef.current.get(p.message_id) ??
            store.messages.find((m) => m.id === p.message_id)?.content ??
            "";
          flushDeltaBuffer(p.message_id);
          store.completeMessage(p.message_id, finalContent, p.input_tokens, p.output_tokens);
          break;
        }
        case "typing": {
          const p = payload as TypingPayload;
          store.setAgentTyping(p.agent ?? p.is_typing ?? false);
          break;
        }
        case "tool_call_updated": {
          const p = payload as ToolCallUpdatedPayload;
          store.updateToolCall({
            id: p.tool_call_id,
            name: p.tool_name,
            status: p.status,
            ...(p.detail !== undefined && { detail: p.detail }),
          });
          break;
        }
        case "agent_state": {
          const p = payload as AgentStatePayload;
          if (p.state === "thinking" || p.state === "tool_use") {
            store.setAgentTyping(true);
          } else if (p.state === "idle" || p.state === "error") {
            store.setAgentTyping(false);
          }
          break;
        }
        case "error": {
          const p = payload as ErrorPayload;
          store.setSendError(p.message || p.code || "Unknown error");
          store.setAgentTyping(false);
          break;
        }
        case "connected":
          break;
        case "pong":
          break;
        default:
          break;
      }

      onEventRef.current?.(envelope);
    },
    [flushDeltaBuffer, scheduleDeltaFlush],
  );

  const connect = useCallback(
    (currentToken: string) => {
      if (!mountedRef.current) return;
      cleanup();

      const url = buildWsUrl(currentToken);
      useConnectionStore.getState().setStatus("connecting");

      const ws = new WebSocket(url);
      wsRef.current = ws;

      ws.onopen = () => {
        if (!mountedRef.current) return;
        useConnectionStore.getState().acknowledgeWsConnected();
        backoffRef.current = INITIAL_BACKOFF_MS;

        pingRef.current = setInterval(() => {
          if (ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({ type: "ping" }));
          }
        }, PING_INTERVAL_MS);
      };

      ws.onmessage = handleMessage;

      ws.onclose = () => {
        if (!mountedRef.current) return;
        useConnectionStore
          .getState()
          .setStatus(useConnectionStore.getState().token ? "connecting" : "disconnected");
        if (pingRef.current) {
          clearInterval(pingRef.current);
          pingRef.current = null;
        }

        const delay = backoffRef.current;
        backoffRef.current = Math.min(delay * 2, MAX_BACKOFF_MS);
        reconnectRef.current = setTimeout(() => {
          const latestToken = useConnectionStore.getState().token;
          if (latestToken && mountedRef.current) {
            connect(latestToken);
          }
        }, delay);
      };

      ws.onerror = () => {
        // onclose will fire after onerror
      };
    },
    [cleanup, handleMessage],
  );

  useEffect(() => {
    mountedRef.current = true;

    // Small delay to survive React Strict Mode's unmount-remount cycle
    const timer = setTimeout(() => {
      if (!mountedRef.current) return;
      if (token) {
        connect(token);
      } else {
        cleanup();
        useConnectionStore.getState().setStatus("disconnected");
      }
    }, 50);

    return () => {
      mountedRef.current = false;
      clearTimeout(timer);
      cleanup();
    };
  }, [token, connect, cleanup]);

  useEffect(() => {
    if (!token || !mountedRef.current) {
      return;
    }

    void wsReconnectVersion;

    const readyState = wsRef.current?.readyState;
    if (readyState === WebSocket.OPEN || readyState === WebSocket.CONNECTING) {
      return;
    }

    connect(token);
  }, [token, wsReconnectVersion, connect]);

  const send = useCallback((msg: ClientMessage) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(msg));
    }
  }, []);

  const sendChat = useCallback(
    (message: string, sessionId?: string, attachments?: import("./ws-types").ChatAttachment[]) => {
      send({
        type: "chat",
        ...(sessionId !== undefined && { session_id: sessionId }),
        message,
        ...(attachments && attachments.length > 0 ? { attachments } : {}),
      });
    },
    [send],
  );

  const sendTyping = useCallback(
    (sessionId?: string) => {
      send({ type: "typing", ...(sessionId !== undefined && { session_id: sessionId }) });
    },
    [send],
  );

  const wsStatus: WsStatus =
    connectionStatus === "connected"
      ? "connected"
      : connectionStatus === "connecting"
        ? "connecting"
        : "disconnected";

  return { wsStatus, sendChat, sendTyping };
}
