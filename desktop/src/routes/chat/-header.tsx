import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import { ConfirmAction } from "@/components/confirm-action";
import { InkCross } from "@/components/ink-marks";
import { StatusBadge } from "@/components/status-badge";
import { createSession, deleteSession } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import { getOpticalInlineControlStyle } from "@/lib/ui-polish";
import type { Session } from "@/lib/types";
import type { WsStatus } from "@/lib/use-daemon-ws";
import { useChatStore } from "@/stores/chat";

export function ChatHeader({
  wsStatus,
  sessions,
  activeSessionId,
}: {
  wsStatus: WsStatus;
  sessions: Session[];
  activeSessionId: string | null;
}) {
  const { t } = useI18n();
  const setActiveSessionId = useChatStore((s) => s.setActiveSessionId);
  const clearMessages = useChatStore((s) => s.clearMessages);
  const messageCount = useChatStore((s) => s.messages.length);
  const searchQuery = useChatStore((s) => s.searchQuery);
  const searchRoleFilter = useChatStore((s) => s.searchRoleFilter);
  const agentTyping = useChatStore((s) => s.agentTyping);
  const streamingMessageId = useChatStore((s) => s.streamingMessageId);
  const queryClient = useQueryClient();
  const [searchOpen, setSearchOpen] = useState(false);

  const newSessionMutation = useMutation({
    mutationFn: () => createSession(),
    onSuccess: (session) => {
      setActiveSessionId(session.id);
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => deleteSession(id),
    onSuccess: () => {
      setActiveSessionId(null);
      clearMessages();
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
    },
  });

  const wsVariant =
    wsStatus === "connected" ? "ok" : wsStatus === "connecting" ? "degraded" : "error";
  const hasSearchState = searchQuery.length > 0 || searchRoleFilter !== "all";
  const showSearchRow = messageCount > 0 && (searchOpen || hasSearchState);
  const compactLeadingIconStyle = getOpticalInlineControlStyle({
    density: "compact",
    icon: "leading",
  });

  const isAgentActive = streamingMessageId !== null || agentTyping;
  const agentState = streamingMessageId
    ? t("Responding...")
    : agentTyping
      ? t("Thinking...")
      : wsStatus === "connected"
        ? t("Ready")
        : wsStatus === "connecting"
          ? t("Connecting...")
          : t("Disconnected");

  return (
    <header className="shrink-0 border-b border-[var(--border)] px-6 py-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-3">
          <div className="min-w-0">
            <p
              className="font-display"
              style={{
                fontSize: "15px",
                fontWeight: 600,
                color: "var(--fg)",
                letterSpacing: "-0.01em",
                lineHeight: 1.2,
              }}
            >
              {t("Chat sandbox")}
            </p>
          </div>

          <StatusBadge variant={isAgentActive ? "degraded" : wsVariant} label={agentState} />

          <select
            value={activeSessionId ?? ""}
            onChange={(e) => {
              setActiveSessionId(e.target.value || null);
            }}
            className="ui-field max-w-[220px]"
            style={{
              fontSize: "11px",
              padding: "4px 10px",
              background: "transparent",
              border: "1px solid var(--border)",
              boxShadow: "none",
            }}
          >
            <option value="">{t("New session")}</option>
            {sessions.map((s) => (
              <option key={s.id} value={s.id}>
                {s.id.slice(0, 8)}… — {s.surface ?? t("gui")}
              </option>
            ))}
          </select>

          <button
            type="button"
            onClick={() => newSessionMutation.mutate()}
            disabled={newSessionMutation.isPending}
            className="ui-button ui-button-ink"
            title={t("New session")}
            aria-label={t("New session")}
          >
            <svg
              aria-hidden="true"
              viewBox="0 0 14 14"
              width="14"
              height="14"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
            >
              <path d="M 7 2.5 C 7.1 4.8 6.9 9.2 7 11.5" />
              <path d="M 2.5 7 C 4.8 6.9 9.2 7.1 11.5 7" />
            </svg>
          </button>
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setSearchOpen((open) => !open)}
            disabled={messageCount === 0}
            className="ui-button ui-button-ink"
            title={showSearchRow ? t("Hide search") : t("Show search")}
            aria-label={showSearchRow ? t("Hide search") : t("Show search")}
          >
            <svg
              aria-hidden="true"
              viewBox="0 0 14 14"
              width="13"
              height="13"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.4"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="6" cy="6" r="3.5" />
              <path d="M 8.7 8.7 L 12 12" />
            </svg>
          </button>
          <StatusBadge variant={wsVariant} label={wsStatus} />
        </div>
      </div>

      {(showSearchRow || messageCount > 0 || activeSessionId) && (
        <div className="mt-3 flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border)] pt-3">
          <div>{showSearchRow ? <SearchBar /> : null}</div>

          <div className="flex flex-wrap items-center gap-2">
            {messageCount > 0 ? (
              <button
                type="button"
                onClick={clearMessages}
                className="ui-button ui-button-muted text-xs font-bold uppercase"
                style={compactLeadingIconStyle}
                title={t("Clear messages")}
              >
                <span className="flex items-center gap-1.5">
                  <InkCross size={12} color="currentColor" />
                  {t("Clear")}
                </span>
              </button>
            ) : null}

            {activeSessionId ? (
              <ConfirmAction
                trigger={t("Delete")}
                onConfirm={() => deleteMutation.mutate(activeSessionId)}
                isPending={deleteMutation.isPending}
                confirmLabel={t("Delete this session")}
              />
            ) : null}
          </div>
        </div>
      )}
    </header>
  );
}

function SearchBar() {
  const { t } = useI18n();
  const searchQuery = useChatStore((s) => s.searchQuery);
  const searchRoleFilter = useChatStore((s) => s.searchRoleFilter);
  const setSearchQuery = useChatStore((s) => s.setSearchQuery);
  const setSearchRoleFilter = useChatStore((s) => s.setSearchRoleFilter);

  const handleClear = useCallback(() => {
    setSearchQuery("");
    setSearchRoleFilter("all");
  }, [setSearchQuery, setSearchRoleFilter]);

  const roles = ["all", "user", "assistant"] as const;
  const roleLabels = { all: t("All"), user: t("You"), assistant: t("Agent") };

  return (
    <div className="flex items-center gap-1.5">
      <input
        type="text"
        value={searchQuery}
        onChange={(e) => setSearchQuery(e.target.value)}
        placeholder={t("Search...")}
        className="ui-field"
        style={{
          width: "120px",
          fontSize: "11px",
          padding: "3px 8px",
          boxShadow: "none",
        }}
        aria-label={t("Search messages")}
      />
      <div className="flex gap-0" role="tablist" aria-label={t("Search role filter")}>
        {roles.map((role) => (
          <button
            key={role}
            type="button"
            onClick={() => setSearchRoleFilter(role)}
            data-active={searchRoleFilter === role}
            className="ui-segment-button"
            style={{ fontSize: "9px", padding: "3px 8px" }}
            role="tab"
            aria-selected={searchRoleFilter === role}
          >
            {roleLabels[role]}
          </button>
        ))}
      </div>
      {searchQuery || searchRoleFilter !== "all" ? (
        <button
          type="button"
          onClick={handleClear}
          className="ui-button ui-button-ink"
          title={t("Clear search")}
        >
          <InkCross size={10} color="currentColor" />
        </button>
      ) : null}
    </div>
  );
}
