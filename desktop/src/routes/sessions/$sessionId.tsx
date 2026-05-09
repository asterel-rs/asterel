// Route: which conversation details need review right now?
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { CopyButton } from "@/components/copy-button";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import {
  hasRichMarkdown,
  MessageBubble,
  preloadMarkdownMessageContent,
} from "@/components/message-bubble";
import { PageHeader, PageShell, Panel, SectionLead, StatPill } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { StatusBadge } from "@/components/status-badge";
import { deleteSession, fetchMessages } from "@/lib/api";
import { formatDate, formatTokenCount } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import { formatNumber } from "@/lib/i18n-core";
import { usePageTitle } from "@/lib/use-page-title";
import { useDaemonWs } from "@/lib/use-daemon-ws";
import type { Message } from "@/lib/types";
import { useChatStore } from "@/stores/chat";

export const Route = createFileRoute("/sessions/$sessionId")({
  component: SessionChatPage,
});

function SessionChatPage() {
  const { sessionId } = Route.useParams();
  const { locale, t } = useI18n();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [selectedMessageId, setSelectedMessageId] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const liveMessages = useChatStore((s) => s.messages);
  const streamingMessageId = useChatStore((s) => s.streamingMessageId);
  const streamingContent = useChatStore((s) => s.streamingContent);
  const { wsStatus } = useDaemonWs();

  const liveSessionVisible = wsStatus === "connected" && activeSessionId === sessionId;

  const deleteMutation = useMutation({
    mutationFn: () => deleteSession(sessionId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
      navigate({ to: "/sessions" });
    },
  });

  usePageTitle(`Session ${sessionId.slice(0, 8)}`);

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["messages", sessionId],
    queryFn: () => fetchMessages(sessionId),
    refetchInterval: liveSessionVisible ? false : 5_000,
  });

  const messageItems = data?.items;
  const messages = liveSessionVisible
    ? mergeLiveMessages(messageItems ?? [], liveMessages, streamingMessageId, streamingContent)
    : (messageItems ?? []);
  const assistantCount = messages.filter((message) => message.role === "assistant").length;
  const containsRichMarkdown = messages.some((message) => hasRichMarkdown(message.content));
  const richTurnCount = messages.filter((message) => hasRichMarkdown(message.content)).length;
  const resolvedSelectedMessageId = (() => {
    if (messages.length === 0) return null;
    if (selectedMessageId && messages.some((message) => message.id === selectedMessageId)) {
      return selectedMessageId;
    }
    return messages[messages.length - 1]?.id ?? null;
  })();
  const selectedMessage =
    messages.find((message) => message.id === resolvedSelectedMessageId) ?? null;
  const selectedIndex = selectedMessage
    ? messages.findIndex((message) => message.id === selectedMessage.id)
    : -1;

  useEffect(() => {
    if (containsRichMarkdown) {
      void preloadMarkdownMessageContent();
    }
  }, [containsRichMarkdown]);

  return (
    <PageShell className="flex h-full flex-col" accent="var(--section-operations)">
      <PageHeader
        eyebrow={t("Conversation trace")}
        title={t("Session detail")}
        description={t("Trace for {id}", { id: `${sessionId.slice(0, 12)}...` })}
        actions={
          <>
            <StatPill label={t("Session")} value={`${sessionId.slice(0, 12)}...`} />
            <StatPill label={t("Messages")} value={String(messages.length)} />
            <StatPill label={t("Rich turns")} value={String(richTurnCount)} />
            <StatPill label={t("Assistant")} value={String(assistantCount)} tone="var(--accent)" />
          </>
        }
        aside={
          <div className="flex items-center gap-2">
            {confirmDelete ? (
              <>
                <span className="text-xs text-error">{t("Delete session?")}</span>
                <button
                  type="button"
                  onClick={() => deleteMutation.mutate()}
                  disabled={deleteMutation.isPending}
                  className="ui-button ui-button-accent-fill px-3 py-2 text-xs font-bold uppercase"
                  style={{
                    background: "var(--error)",
                    borderColor: "var(--error)",
                  }}
                >
                  {deleteMutation.isPending ? t("Deleting...") : t("Confirm")}
                </button>
                <button
                  type="button"
                  onClick={() => setConfirmDelete(false)}
                  className="ui-button ui-button-muted px-3 py-2 text-xs font-bold uppercase"
                >
                  {t("Cancel")}
                </button>
              </>
            ) : (
              <button
                type="button"
                onClick={() => setConfirmDelete(true)}
                className="ui-button ui-button-muted px-4 py-2 text-xs font-bold uppercase text-error"
              >
                {t("Delete session")}
              </button>
            )}
            <Link
              to="/sessions"
              className="ui-button ui-button-muted inline-flex items-center px-4 py-2 text-xs font-bold uppercase no-underline text-fg"
              style={{ textDecoration: "none" }}
            >
              {t("Back to sessions")}
            </Link>
          </div>
        }
      />

      {isLoading ? (
        <SkeletonLoader />
      ) : isError ? (
        <ErrorState
          title={t("Failed to load messages")}
          message={error instanceof Error ? error.message : t("Could not reach the daemon.")}
          onRetry={() => refetch()}
        />
      ) : messages.length === 0 ? (
        <EmptyState
          title={t("No messages yet")}
          description={t("Messages will appear here as the conversation progresses.")}
        />
      ) : (
        <div className="grid min-h-0 flex-1 gap-6 xl:grid-cols-[minmax(0,1fr)_340px]">
          <div className="flex min-h-0 flex-col">
            <SectionLead
              title={t("Transcript")}
              description={
                liveSessionVisible
                  ? t("{count} turns - live over WebSocket", { count: messages.length })
                  : t("{count} turns - polling every 5 seconds", { count: messages.length })
              }
              action={
                <button
                  type="button"
                  onClick={() => refetch()}
                  className="ui-button ui-button-muted px-3 py-1 text-xs font-bold uppercase"
                  style={{ color: "var(--fg-soft)" }}
                >
                  {t("Refresh")}
                </button>
              }
            />
            <div className="mt-4 min-h-0 flex-1 overflow-y-auto">
              <div className="mx-auto max-w-3xl">
                {messages.map((message, index) => (
                  <TranscriptRow
                    key={message.id}
                    message={message}
                    index={index}
                    isSelected={message.id === selectedMessage?.id}
                    onSelect={() => setSelectedMessageId(message.id)}
                  />
                ))}
              </div>
            </div>
          </div>

          <div className="min-h-0 overflow-y-auto">
            <Panel strong variant="stage" className="px-5 py-5">
              <SectionLead
                title={t("Turn inspector")}
                description={t("Metadata for the selected turn.")}
                action={
                  selectedMessage ? (
                    <StatusBadge
                      variant={roleVariant(selectedMessage.role)}
                      label={t("Turn {index}", { index: selectedIndex + 1 })}
                    />
                  ) : undefined
                }
              />
              {selectedMessage ? (
                <>
                  <div className="ui-rule-list mt-4">
                    <MetaRow label={t("Role")} value={selectedMessage.role} />
                    <MetaRow label={t("Created")} value={formatDate(selectedMessage.created_at)} />
                    <MetaRow
                      label={t("Input tokens")}
                      value={formatTokenCount(selectedMessage.input_tokens)}
                    />
                    <MetaRow
                      label={t("Output tokens")}
                      value={formatTokenCount(selectedMessage.output_tokens)}
                    />
                    <MetaRow
                      label={t("Content mode")}
                      value={
                        hasRichMarkdown(selectedMessage.content) ? t("markdown") : t("plain text")
                      }
                    />
                    <MetaRow
                      label={t("Characters")}
                      value={formatNumber(selectedMessage.content.length, locale)}
                    />
                  </div>

                  <div className="mt-4 border-t border-[var(--border)] pt-3">
                    <div className="flex items-center justify-between">
                      <p className="app-section-title">{t("Raw content")}</p>
                      <CopyButton text={selectedMessage.content} />
                    </div>
                    <pre className="ui-code-block mt-3 max-h-[280px] overflow-auto font-mono">
                      {selectedMessage.content}
                    </pre>
                  </div>
                </>
              ) : (
                <div className="mt-4">
                  <p className="text-sm" style={{ color: "var(--fg-muted)" }}>
                    {t("Select a message from the transcript to inspect it.")}
                  </p>
                </div>
              )}
            </Panel>
          </div>
        </div>
      )}
    </PageShell>
  );
}

function mergeLiveMessages(
  persistedMessages: Message[],
  liveMessages: ReturnType<typeof useChatStore.getState>["messages"],
  streamingMessageId: string | null,
  streamingContent: string,
): Message[] {
  const mergedById = new Map<string, Message>();
  const orderedIds: string[] = [];

  for (const message of persistedMessages) {
    mergedById.set(message.id, message);
    orderedIds.push(message.id);
  }

  for (const message of liveMessages) {
    const existing = mergedById.get(message.id);
    mergedById.set(message.id, {
      id: message.id,
      role: message.role,
      content: message.id === streamingMessageId ? streamingContent : message.content,
      created_at: message.created_at,
      ...(message.input_tokens !== undefined
        ? { input_tokens: message.input_tokens }
        : existing?.input_tokens !== undefined
          ? { input_tokens: existing.input_tokens }
          : {}),
      ...(message.output_tokens !== undefined
        ? { output_tokens: message.output_tokens }
        : existing?.output_tokens !== undefined
          ? { output_tokens: existing.output_tokens }
          : {}),
    });

    if (!orderedIds.includes(message.id)) {
      orderedIds.push(message.id);
    }
  }

  return orderedIds
    .map((id) => mergedById.get(id))
    .filter((message): message is Message => !!message);
}

function TranscriptRow({
  message,
  index,
  isSelected,
  onSelect,
}: {
  message: Message;
  index: number;
  isSelected: boolean;
  onSelect: () => void;
}) {
  const { t } = useI18n();

  return (
    <button
      type="button"
      onClick={onSelect}
      data-selected={isSelected}
      className="ui-ledger-button ui-transcript-focus"
    >
      <div className="mb-2">
        <span className="ui-chip">
          {t("Turn {index}", { index: String(index + 1).padStart(2, "0") })}
        </span>
      </div>
      <MessageBubble message={message} />
    </button>
  );
}

function MetaRow({ label, value }: { label: string; value: string }) {
  const { t } = useI18n();

  return (
    <div className="ui-rule-row">
      <p className="ui-rule-key">{t(label)}</p>
      <p className="ui-rule-value" style={{ color: "var(--fg-soft)" }}>
        {value}
      </p>
    </div>
  );
}

function roleVariant(role: string): "ok" | "degraded" | "error" | "info" | "neutral" {
  if (role === "assistant") {
    return "info";
  }

  if (role === "tool") {
    return "degraded";
  }

  if (role === "system") {
    return "neutral";
  }

  return "ok";
}
