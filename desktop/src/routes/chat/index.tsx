// Route: how can an operator test the companion in a clearly secondary sandbox?
import { useQuery } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";
import { useCallback, useEffect, useState } from "react";
import { fetchMessages, fetchSessions } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import { usePageTitle } from "@/lib/use-page-title";
import { useDaemonWs } from "@/lib/use-daemon-ws";
import { useChatStore } from "@/stores/chat";
import { ChatComposer } from "./-composer";
import { ChatHeader } from "./-header";
import { ChatMessageList } from "./-message-list";

function withOptionalTokenCounts(
  message: {
    id: string;
    role: string;
    content: string;
    created_at: string;
  },
  inputTokens?: number,
  outputTokens?: number,
) {
  return {
    ...message,
    ...(inputTokens !== undefined ? { input_tokens: inputTokens } : {}),
    ...(outputTokens !== undefined ? { output_tokens: outputTokens } : {}),
  };
}

export const Route = createFileRoute("/chat/")({
  component: ChatPage,
});

function ChatPage() {
  const { t } = useI18n();
  const { wsStatus, sendChat, sendTyping } = useDaemonWs();
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const historyLoaded = useChatStore((s) => s.historyLoaded);
  const [historyError, setHistoryError] = useState(false);

  usePageTitle("Chat Sandbox");

  const sessionsQuery = useQuery({
    queryKey: ["sessions"],
    queryFn: fetchSessions,
    refetchInterval: 30_000,
  });

  // Load message history when switching to an existing session.
  useEffect(() => {
    if (!activeSessionId || historyLoaded) return;

    let cancelled = false;
    fetchMessages(activeSessionId)
      .then((result) => {
        if (cancelled) return;
        const store = useChatStore.getState();
        if (store.activeSessionId !== activeSessionId) return;

        store.loadHistory(
          result.items.map((m) =>
            withOptionalTokenCounts(
              {
                id: m.id,
                role: m.role.toLowerCase(),
                content: m.content,
                created_at: m.created_at,
              },
              m.input_tokens,
              m.output_tokens,
            ),
          ),
        );
      })
      .catch(() => {
        // History load failed — still allow new messages
        if (!cancelled) {
          useChatStore.getState().loadHistory([]);
          setHistoryError(true);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [activeSessionId, historyLoaded]);

  const handleSend = useCallback(
    (message: string, attachments?: import("@/lib/ws-types").ChatAttachment[]) => {
      try {
        sendChat(message, activeSessionId ?? undefined, attachments);
      } catch {
        useChatStore.getState().setSendError("WebSocket not connected");
      }
    },
    [sendChat, activeSessionId],
  );

  const handleTyping = useCallback(() => {
    sendTyping(activeSessionId ?? undefined);
  }, [sendTyping, activeSessionId]);

  return (
    <div
      className="flex h-full flex-col"
      style={{ "--page-accent": "var(--section-operations)" } as React.CSSProperties}
    >
      <ChatHeader
        wsStatus={wsStatus}
        sessions={sessionsQuery.data?.items ?? []}
        activeSessionId={activeSessionId}
      />
      {historyError ? (
        <div aria-live="polite" className="trust-advisory mx-6 mt-1" data-level="nudge">
          <p className="trust-advisory-title">{t("History could not be loaded")}</p>
          <div className="mt-1 flex items-center gap-3">
            <p style={{ fontSize: "12px" }}>{t("New messages will still appear below.")}</p>
            <button
              type="button"
              onClick={() => setHistoryError(false)}
              className="ui-button ui-button-muted px-2 py-1 text-[10px] font-bold uppercase"
            >
              {t("Dismiss")}
            </button>
          </div>
        </div>
      ) : null}
      <ChatMessageList onSend={handleSend} />
      <ChatComposer wsStatus={wsStatus} onSend={handleSend} onTyping={handleTyping} />
    </div>
  );
}
