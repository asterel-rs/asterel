import { memo, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CopyButton } from "@/components/copy-button";
import { InkArrow, InkCross, InkEmptyPage } from "@/components/ink-marks";
import { StaleAwareTypingIndicator } from "@/components/stale-aware-typing-indicator";
import { hasRichMarkdown, MarkdownMessageContent } from "@/components/message-bubble";
import { StatusBadge } from "@/components/status-badge";
import { useI18n } from "@/lib/i18n";
import type { ChatMessage, ToolCallState } from "@/stores/chat";
import { useChatStore } from "@/stores/chat";

export function ChatMessageList({ onSend }: { onSend?: (message: string) => void }) {
  const { t } = useI18n();
  const allMessages = useChatStore((s) => s.messages);
  const streamingMessageId = useChatStore((s) => s.streamingMessageId);
  const streamingContent = useChatStore((s) => s.streamingContent);
  const agentTyping = useChatStore((s) => s.agentTyping);
  const activeToolCalls = useChatStore((s) => s.activeToolCalls);
  const sendError = useChatStore((s) => s.sendError);
  const searchQuery = useChatStore((s) => s.searchQuery);
  const searchRoleFilter = useChatStore((s) => s.searchRoleFilter);

  const isSearching = searchQuery.length > 0 || searchRoleFilter !== "all";

  const messages = useMemo(() => {
    if (!isSearching) return allMessages;
    const q = searchQuery.toLowerCase();
    return allMessages.filter((msg) => {
      if (searchRoleFilter !== "all" && msg.role.toLowerCase() !== searchRoleFilter) {
        return false;
      }
      if (q && !msg.content.toLowerCase().includes(q)) {
        return false;
      }
      return true;
    });
  }, [allMessages, searchQuery, searchRoleFilter, isSearching]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const [showScrollDown, setShowScrollDown] = useState(false);

  const scrollToBottom = useCallback(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, []);

  useEffect(() => {
    scrollToBottom();
  }, [messages.length, streamingContent, scrollToBottom]);

  useEffect(() => {
    const container = scrollRef.current;
    if (!container) return;

    let frameId: number | null = null;

    const handleScroll = () => {
      if (frameId !== null) {
        return;
      }

      frameId = window.requestAnimationFrame(() => {
        const { scrollTop, scrollHeight, clientHeight } = container;
        setShowScrollDown(scrollHeight - scrollTop - clientHeight > 100);
        frameId = null;
      });
    };

    container.addEventListener("scroll", handleScroll, { passive: true });
    handleScroll();

    return () => {
      container.removeEventListener("scroll", handleScroll);
      if (frameId !== null) {
        window.cancelAnimationFrame(frameId);
      }
    };
  }, []);

  const handlePromptClick = useCallback(
    (prompt: string) => {
      if (!onSend) return;
      useChatStore.getState().appendMessage({
        id: `user-${Date.now()}`,
        role: "user",
        content: prompt,
        created_at: new Date().toISOString(),
      });
      onSend(prompt);
    },
    [onSend],
  );

  if (allMessages.length === 0 && !agentTyping) {
    const prompts = [
      t("What can you help me with?"),
      t("Show recent sessions"),
      t("Check system status"),
    ];

    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-5">
        <InkEmptyPage
          size={52}
          color="var(--fg-muted)"
          className="app-empty-bob"
          style={{ opacity: 0.6 }}
        />
        <p
          className="font-display"
          style={{
            fontSize: "17px",
            fontWeight: 600,
            color: "var(--fg-soft)",
            letterSpacing: "-0.01em",
          }}
        >
          {t("Start a conversation")}
        </p>
        <div className="flex flex-wrap justify-center gap-3" style={{ maxWidth: "420px" }}>
          {prompts.map((prompt) => (
            <button
              key={prompt}
              type="button"
              onClick={() => handlePromptClick(prompt)}
              className="chat-prompt-tab"
            >
              {prompt}
            </button>
          ))}
        </div>
        <p
          style={{
            fontSize: "11px",
            color: "var(--fg-muted)",
            opacity: 0.5,
            marginTop: "4px",
          }}
        >
          {t("Press Enter to send")}
        </p>
      </div>
    );
  }

  return (
    <div ref={scrollRef} className="relative min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-3xl px-6 py-6" style={{ display: "grid", gap: "2px" }}>
        {isSearching && messages.length === 0 ? (
          <p className="py-8 text-center text-xs" style={{ color: "var(--fg-muted)" }}>
            {t("No matching messages")}
          </p>
        ) : null}
        {messages.map((msg, idx) => (
          <ChatBubble
            key={msg.id}
            message={msg}
            isStreaming={msg.id === streamingMessageId}
            {...(msg.id === streamingMessageId ? { streamingContent } : {})}
            isConsecutive={idx > 0 && messages[idx - 1]?.role === msg.role}
          />
        ))}

        {activeToolCalls.length > 0 ? (
          <div
            style={{
              padding: "4px 0 4px 16px",
              display: "grid",
              gap: "6px",
            }}
          >
            {activeToolCalls.map((tc) => (
              <ToolCallCard key={tc.id} toolCall={tc} />
            ))}
          </div>
        ) : null}

        {agentTyping && !streamingMessageId ? (
          <StaleAwareTypingIndicator
            onCancel={() => {
              const s = useChatStore.getState();
              s.setAgentTyping(false);
              s.setSendError("Turn cancelled by user");
            }}
          />
        ) : null}

        {sendError ? <SendErrorBanner error={sendError} /> : null}

        <div ref={bottomRef} />
      </div>

      {showScrollDown ? (
        <button
          type="button"
          onClick={scrollToBottom}
          className="chat-scroll-ribbon absolute bottom-0 left-1/2 -translate-x-1/2"
          style={{ zIndex: 10 }}
          title={t("Scroll to latest")}
          aria-label={t("Scroll to latest")}
        >
          <InkArrow size={11} color="var(--accent-strong)" style={{ transform: "rotate(90deg)" }} />
        </button>
      ) : null}
    </div>
  );
}

function formatMessageTime(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return "";
  }
}

const ChatBubble = memo(function ChatBubble({
  message,
  isStreaming,
  streamingContent,
  isConsecutive,
}: {
  message: ChatMessage;
  isStreaming: boolean;
  streamingContent?: string | undefined;
  isConsecutive?: boolean | undefined;
}) {
  const { t } = useI18n();
  const isUser = message.role.toLowerCase() === "user";
  const content = isStreaming && streamingContent != null ? streamingContent : message.content;
  const renderRich = !isUser && hasRichMarkdown(content);
  const timeStr = formatMessageTime(message.created_at);

  /* ── User message: right-aligned bubble with right accent ── */
  if (isUser) {
    return (
      <div className="flex justify-end" style={{ padding: isConsecutive ? "2px 0" : "8px 0" }}>
        <div className="chat-bubble-user group relative chat-selectable">
          {!isConsecutive ? (
            <div className="flex items-center gap-2" style={{ marginBottom: "4px" }}>
              <span className="chat-role-label" style={{ color: "var(--accent-strong)" }}>
                {t("You")}
              </span>
              {timeStr ? <span className="chat-timestamp">{timeStr}</span> : null}
            </div>
          ) : null}
          <div className="chat-bubble-content">{content}</div>
        </div>
      </div>
    );
  }

  return (
    <div
      className="group relative chat-selectable"
      style={{ padding: isConsecutive ? "2px 0" : "10px 0" }}
    >
      {!isConsecutive ? (
        <div
          className="flex items-center gap-2"
          style={{ marginBottom: "6px", paddingLeft: "16px" }}
        >
          <span className="chat-role-label" style={{ color: "var(--fg-soft)" }}>
            {t("Agent")}
          </span>
          {timeStr ? <span className="chat-timestamp">{timeStr}</span> : null}
          {message.output_tokens || message.input_tokens ? (
            <span
              className="opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
              style={{
                fontSize: "10px",
                color: "var(--fg-muted)",
                transitionDuration: "120ms",
              }}
            >
              {message.output_tokens ?? ""}
              {message.input_tokens ? `→${message.input_tokens}` : ""} tok
            </span>
          ) : null}
        </div>
      ) : null}
      <div className="chat-agent-border chat-bubble-content">
        {renderRich ? (
          <Suspense fallback={<PlainContent content={content} />}>
            <MarkdownMessageContent content={content} />
          </Suspense>
        ) : (
          <PlainContent content={content} />
        )}
        {isStreaming ? (
          <span
            className="animate-blink inline-block"
            style={{
              width: "2px",
              height: "14px",
              background: "var(--accent)",
              marginLeft: "2px",
              verticalAlign: "text-bottom",
            }}
          />
        ) : null}
      </div>

      {!isStreaming && content.length > 0 ? (
        <div
          className="absolute top-2 right-0 opacity-0 transition-opacity group-hover:opacity-100"
          style={{ transitionDuration: "120ms" }}
        >
          <CopyButton text={content} />
        </div>
      ) : null}
    </div>
  );
});

function PlainContent({ content }: { content: string }) {
  return <div style={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>{content}</div>;
}

function ToolCallCard({ toolCall }: { toolCall: ToolCallState }) {
  const isRunning = toolCall.status === "running";
  const variant =
    toolCall.status === "completed" ? "ok" : toolCall.status === "failed" ? "error" : "degraded";

  return (
    <div
      className={`chat-agent-border flex items-center gap-3 px-4 py-2.5${isRunning ? " animate-tool-running" : ""}`}
      style={{ fontSize: "12px" }}
    >
      <StatusBadge variant={variant} label={toolCall.status} />
      <span className="text-fg" style={{ fontWeight: 600 }}>
        {toolCall.name}
      </span>
      {toolCall.detail ? <span style={{ color: "var(--fg-muted)" }}>{toolCall.detail}</span> : null}
    </div>
  );
}

function SendErrorBanner({ error }: { error: string }) {
  const { t } = useI18n();

  return (
    <div
      className="flex items-center gap-3 py-2"
      style={{
        borderLeft: "2px solid var(--error)",
        paddingLeft: "16px",
        fontSize: "12px",
      }}
    >
      <span className="text-error" style={{ fontWeight: 600 }}>
        {t("Send failed")}
      </span>
      <span style={{ color: "var(--fg-muted)" }}>{error}</span>
      <button
        type="button"
        onClick={() => useChatStore.getState().setSendError(null)}
        className="ui-button ui-button-ink ml-auto"
        title={t("Dismiss")}
      >
        <InkCross size={12} color="var(--error)" />
      </button>
    </div>
  );
}
