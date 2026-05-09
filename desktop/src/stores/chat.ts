import { create } from "zustand";
import type { ChatMessage, ToolCallState } from "@/lib/types";

export type { ChatMessage, ToolCallState };

interface ChatState {
  activeSessionId: string | null;
  messages: ChatMessage[];
  streamingMessageId: string | null;
  streamingContent: string;
  agentTyping: boolean;
  typingSince: number | null;
  activeToolCalls: ToolCallState[];
  historyLoaded: boolean;
  sendError: string | null;
  searchQuery: string;
  searchRoleFilter: "all" | "user" | "assistant";

  setActiveSessionId: (id: string | null) => void;
  adoptSessionId: (id: string) => void;
  appendMessage: (msg: ChatMessage) => void;
  appendDelta: (messageId: string, delta: string) => void;
  completeMessage: (
    messageId: string,
    content: string,
    inputTokens?: number,
    outputTokens?: number,
  ) => void;
  setAgentTyping: (typing: boolean) => void;
  updateToolCall: (tc: ToolCallState) => void;
  clearMessages: () => void;
  loadHistory: (messages: ChatMessage[]) => void;
  setSendError: (error: string | null) => void;
  setSearchQuery: (query: string) => void;
  setSearchRoleFilter: (role: "all" | "user" | "assistant") => void;
}

export const useChatStore = create<ChatState>()((set) => ({
  activeSessionId: null,
  messages: [],
  streamingMessageId: null,
  streamingContent: "",
  agentTyping: false,
  typingSince: null,
  activeToolCalls: [],
  historyLoaded: false,
  sendError: null,
  searchQuery: "",
  searchRoleFilter: "all",

  setActiveSessionId: (id) =>
    set({
      activeSessionId: id,
      messages: [],
      streamingMessageId: null,
      streamingContent: "",
      activeToolCalls: [],
      agentTyping: false,
      typingSince: null,
      historyLoaded: false,
      sendError: null,
    }),

  adoptSessionId: (id) =>
    set((state) => ({
      activeSessionId: state.activeSessionId ?? id,
    })),

  appendMessage: (msg) =>
    set((state) => {
      if (state.messages.some((m) => m.id === msg.id)) return state;
      return {
        messages: [...state.messages, msg],
        streamingMessageId: msg.role === "assistant" ? msg.id : state.streamingMessageId,
        streamingContent: msg.role === "assistant" ? msg.content : state.streamingContent,
        agentTyping: false,
        typingSince: null,
        sendError: null,
      };
    }),

  appendDelta: (messageId, delta) =>
    set((state) => ({
      streamingMessageId: messageId,
      streamingContent:
        state.streamingMessageId === messageId ? state.streamingContent + delta : delta,
    })),

  completeMessage: (messageId, content, inputTokens, outputTokens) =>
    set((state) => ({
      messages: state.messages.map((m) =>
        m.id === messageId
          ? {
              ...m,
              content,
              ...(inputTokens != null ? { input_tokens: inputTokens } : {}),
              ...(outputTokens != null ? { output_tokens: outputTokens } : {}),
            }
          : m,
      ),
      streamingMessageId: null,
      streamingContent: "",
      agentTyping: false,
      typingSince: null,
      activeToolCalls: [],
    })),

  setAgentTyping: (typing) => set({ agentTyping: typing, typingSince: typing ? Date.now() : null }),

  updateToolCall: (tc) =>
    set((state) => {
      const existing = state.activeToolCalls.findIndex((t) => t.id === tc.id);
      if (existing >= 0) {
        const next = [...state.activeToolCalls];
        next[existing] = tc;
        return {
          activeToolCalls:
            tc.status === "running"
              ? next
              : next.filter((t) => t.id !== tc.id || t.status === "running"),
        };
      }
      return { activeToolCalls: [...state.activeToolCalls, tc] };
    }),

  clearMessages: () =>
    set({
      messages: [],
      streamingMessageId: null,
      streamingContent: "",
      activeToolCalls: [],
      agentTyping: false,
      typingSince: null,
      historyLoaded: false,
      sendError: null,
    }),

  loadHistory: (messages) =>
    set({
      messages,
      historyLoaded: true,
      streamingMessageId: null,
      streamingContent: "",
      activeToolCalls: [],
      agentTyping: false,
      typingSince: null,
    }),

  setSendError: (error) => set({ sendError: error }),
  setSearchQuery: (query) => set({ searchQuery: query }),
  setSearchRoleFilter: (role) => set({ searchRoleFilter: role }),
}));
