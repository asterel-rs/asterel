import { useCallback, useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { InkCross } from "@/components/ink-marks";
import { type UploadResult, fetchAgents, uploadFile } from "@/lib/api";
import { compressMedia } from "@/lib/media-compress";
import { useI18n } from "@/lib/i18n";
import type { AgentEntry } from "@/lib/types";
import type { ChatAttachment } from "@/lib/ws-types";
import type { WsStatus } from "@/lib/use-daemon-ws";
import { useChatStore } from "@/stores/chat";
import { ImageCropModal } from "@/components/image-crop-modal";

import { MentionPopup } from "./-mention-popup";

const MAX_ROWS = 6;
const TYPING_DEBOUNCE_MS = 3_000;

export function ChatComposer({
  wsStatus,
  onSend,
  onTyping,
}: {
  wsStatus: WsStatus;
  onSend: (message: string, attachments?: ChatAttachment[]) => void;
  onTyping: () => void;
}) {
  const { t } = useI18n();
  const [draft, setDraft] = useState("");
  const [pendingFiles, setPendingFiles] = useState<File[]>([]);
  const [uploading, setUploading] = useState(false);
  const [mentionOpen, setMentionOpen] = useState(false);
  const [mentionFilter, setMentionFilter] = useState("");
  const [mentionIndex, setMentionIndex] = useState(0);
  const [targetAgent, setTargetAgent] = useState<AgentEntry | null>(null);
  const [cropQueue, setCropQueue] = useState<File[]>([]);

  const [cropTarget, setCropTarget] = useState<{
    file: File;
    objectUrl: string;
  } | null>(null);

  const agentsQuery = useQuery({
    queryKey: ["agents"],
    queryFn: fetchAgents,
    staleTime: 30_000,
  });
  const agents = agentsQuery.data?.items ?? [];
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const typingTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const streamingMessageId = useChatStore((s) => s.streamingMessageId);
  const agentTyping = useChatStore((s) => s.agentTyping);

  const cropTargetRef = useRef(cropTarget);
  cropTargetRef.current = cropTarget;

  useEffect(() => {
    if (cropTarget !== null || cropQueue.length === 0) return;
    const [next, ...rest] = cropQueue;
    if (!next) return;
    setCropQueue(rest);
    setCropTarget({ file: next, objectUrl: URL.createObjectURL(next) });
  }, [cropTarget, cropQueue]);

  useEffect(() => {
    return () => {
      if (cropTargetRef.current) {
        URL.revokeObjectURL(cropTargetRef.current.objectUrl);
      }
    };
  }, []);

  const isBusy = streamingMessageId !== null || agentTyping;
  const disabled = wsStatus !== "connected" || isBusy || uploading;
  const filteredAgents = agents.filter((a) =>
    a.name.toLowerCase().includes(mentionFilter.toLowerCase()),
  );
  const mentionPopupId = "chat-mention-popup";
  const activeMentionId = mentionOpen
    ? `mention-option-${filteredAgents[mentionIndex]?.id ?? "none"}`
    : undefined;

  const handleSend = useCallback(async () => {
    const trimmed = draft.trim();
    if ((!trimmed && pendingFiles.length === 0) || disabled) return;

    const store = useChatStore.getState();
    const messageContent = trimmed || `[${pendingFiles.map((f) => f.name).join(", ")}]`;

    store.appendMessage({
      id: `user-${Date.now()}`,
      role: "user",
      content: messageContent,
      created_at: new Date().toISOString(),
    });

    try {
      let attachments: ChatAttachment[] | undefined;

      if (pendingFiles.length > 0) {
        setUploading(true);
        const results: UploadResult[] = [];
        for (const file of pendingFiles) {
          const compressed = await compressMedia(file);
          results.push(await uploadFile(compressed));
        }
        attachments = results.map((r) => ({
          upload_id: r.upload_id,
          filename: r.filename,
          content_type: r.content_type,
        }));
        setUploading(false);
      }

      onSend(messageContent, attachments);
    } catch {
      setUploading(false);
      store.setSendError(t("Failed to send message"));
    }

    setDraft("");
    setPendingFiles([]);
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
  }, [draft, pendingFiles, disabled, onSend, t]);

  const appendPendingFiles = useCallback((files: File[]) => {
    if (files.length === 0) return;

    setPendingFiles((prev) => {
      const next = [...prev];
      for (const file of files) {
        const duplicate = next.some(
          (f) =>
            f.name === file.name && f.size === file.size && f.lastModified === file.lastModified,
        );
        if (!duplicate) next.push(file);
      }
      return next;
    });

    useChatStore.getState().setSendError(null);
  }, []);

  const enqueueForCrop = useCallback((files: File[]) => {
    if (files.length === 0) return;
    setCropQueue((prev) => [...prev, ...files]);
  }, []);

  const handleCropConfirm = useCallback(
    (croppedFile: File) => {
      if (cropTarget) {
        URL.revokeObjectURL(cropTarget.objectUrl);
      }
      appendPendingFiles([croppedFile]);
      setCropTarget(null);
    },
    [cropTarget, appendPendingFiles],
  );

  const handleCropCancel = useCallback(() => {
    if (cropTarget) {
      appendPendingFiles([cropTarget.file]);
      URL.revokeObjectURL(cropTarget.objectUrl);
    }
    setCropTarget(null);
  }, [cropTarget, appendPendingFiles]);

  const handlePaste = useCallback(
    (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
      const clipboardItems = Array.from(e.clipboardData.items ?? []);
      if (clipboardItems.length === 0) return;

      const imageFiles: File[] = [];
      let hasNonImageFile = false;

      for (const item of clipboardItems) {
        if (item.kind !== "file") {
          continue;
        }

        const file = item.getAsFile();
        if (!file) {
          continue;
        }

        if (file.type.startsWith("image/")) {
          imageFiles.push(file);
        } else {
          hasNonImageFile = true;
        }
      }

      if (imageFiles.length === 0) {
        if (hasNonImageFile) {
          e.preventDefault();
          useChatStore.getState().setSendError(t("Only image files can be pasted as attachments."));
        }
        return;
      }

      e.preventDefault();
      enqueueForCrop(imageFiles);
    },
    [enqueueForCrop, t],
  );

  const handleMentionSelect = useCallback(
    (agent: AgentEntry) => {
      setTargetAgent(agent);
      // Replace the @filter text with @name
      const atIndex = draft.lastIndexOf("@");
      if (atIndex >= 0) {
        setDraft(
          `${draft.slice(0, atIndex)}@${agent.name} ${draft.slice(atIndex + 1 + mentionFilter.length)}`,
        );
      }
      setMentionOpen(false);
      setMentionFilter("");
      setMentionIndex(0);
      textareaRef.current?.focus();
    },
    [draft, mentionFilter],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (mentionOpen) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setMentionIndex((i) => Math.min(i + 1, filteredAgents.length - 1));
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setMentionIndex((i) => Math.max(i - 1, 0));
          return;
        }
        if (e.key === "Enter" || e.key === "Tab") {
          e.preventDefault();
          if (filteredAgents[mentionIndex]) {
            handleMentionSelect(filteredAgents[mentionIndex]);
          }
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          setMentionOpen(false);
          return;
        }
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend, mentionOpen, filteredAgents, mentionIndex, handleMentionSelect],
  );

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      const value = e.target.value;
      setDraft(value);

      // Detect @mention trigger
      const cursorPos = e.target.selectionStart ?? value.length;
      const textBefore = value.slice(0, cursorPos);
      const atMatch = textBefore.match(/@(\w*)$/);
      if (atMatch && agents.length > 0) {
        setMentionOpen(true);
        setMentionFilter(atMatch[1] ?? "");
        setMentionIndex(0);
      } else {
        setMentionOpen(false);
      }

      const el = e.target;
      el.style.height = "auto";
      const lineHeight = 22;
      const maxHeight = lineHeight * MAX_ROWS;
      el.style.height = `${Math.min(el.scrollHeight, maxHeight)}px`;

      if (!typingTimerRef.current) {
        onTyping();
      }
      if (typingTimerRef.current) clearTimeout(typingTimerRef.current);
      typingTimerRef.current = setTimeout(() => {
        typingTimerRef.current = null;
      }, TYPING_DEBOUNCE_MS);
    },
    [agents.length, onTyping],
  );

  const handleFileSelect = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = Array.from(e.target.files ?? []);
      if (files.length === 0) {
        e.target.value = "";
        return;
      }
      const images: File[] = [];
      const others: File[] = [];
      for (const f of files) {
        if (f.type.startsWith("image/")) {
          images.push(f);
        } else {
          others.push(f);
        }
      }
      if (others.length > 0) {
        appendPendingFiles(others);
      }
      if (images.length > 0) {
        enqueueForCrop(images);
      }
      e.target.value = "";
    },
    [appendPendingFiles, enqueueForCrop],
  );

  const removeFile = useCallback((index: number) => {
    setPendingFiles((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const placeholder = (() => {
    if (wsStatus !== "connected") return t("Reconnecting...");
    if (isBusy) return t("Agent is responding...");
    return t("Message...");
  })();

  const charCount = draft.length;

  return (
    <div className="shrink-0 border-t border-[var(--border)] bg-[var(--bg)] px-6 py-4">
      {/* Attachment & target agent chips — single scrollable row */}
      {pendingFiles.length > 0 || targetAgent ? (
        <div
          className="mx-auto mb-2 flex max-w-3xl items-center gap-1.5 overflow-x-auto"
          style={{ scrollbarWidth: "none" }}
        >
          {targetAgent ? (
            <span className="ui-chip shrink-0" style={{ color: "var(--info)" }}>
              @{targetAgent.name}
              <button
                type="button"
                onClick={() => setTargetAgent(null)}
                className="ui-button-ink"
                style={{ padding: 0 }}
              >
                <InkCross size={9} color="currentColor" />
              </button>
            </span>
          ) : null}
          {pendingFiles.map((file, i) => (
            <span
              key={`${file.name}-${file.lastModified}-${file.size}`}
              className="ui-chip shrink-0"
            >
              <span className="max-w-[100px] truncate">{file.name}</span>
              <span style={{ fontSize: "9px", opacity: 0.6 }}>{formatFileSize(file.size)}</span>
              <button
                type="button"
                onClick={() => removeFile(i)}
                className="ui-button-ink"
                style={{ padding: 0 }}
              >
                <InkCross size={9} color="currentColor" />
              </button>
            </span>
          ))}
        </div>
      ) : null}

      <div className="mx-auto flex max-w-3xl items-end gap-3">
        {/* File attach button */}
        <button
          type="button"
          onClick={() => fileInputRef.current?.click()}
          disabled={wsStatus !== "connected"}
          className="ui-button ui-button-ink shrink-0 mb-1"
          title={t("Attach file")}
          aria-label={t("Attach file")}
        >
          <svg
            aria-hidden="true"
            viewBox="0 0 16 16"
            width="16"
            height="16"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M 13.5 7.5 L 8 13 C 6.3 14.7 3.7 14.7 2 13 C 0.3 11.3 0.3 8.7 2 7 L 8.5 0.5 C 9.6 -0.6 11.4 -0.6 12.5 0.5 C 13.6 1.6 13.6 3.4 12.5 4.5 L 6 11 C 5.4 11.6 4.6 11.6 4 11 C 3.4 10.4 3.4 9.6 4 9 L 9.5 3.5" />
          </svg>
        </button>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          onChange={handleFileSelect}
          className="hidden"
        />

        <div className="relative min-w-0 flex-1">
          {mentionOpen ? (
            <MentionPopup
              id={mentionPopupId}
              agents={agents}
              filter={mentionFilter}
              selectedIndex={mentionIndex}
              onSelect={handleMentionSelect}
            />
          ) : null}
          <textarea
            ref={textareaRef}
            value={draft}
            onChange={handleChange}
            onPaste={handlePaste}
            onKeyDown={handleKeyDown}
            placeholder={placeholder}
            disabled={wsStatus !== "connected"}
            rows={1}
            className="ui-field w-full"
            style={{
              resize: "none",
              lineHeight: "22px",
              fontSize: "13px",
              padding: "10px 14px 10px 16px",
              paddingRight: charCount > 200 ? "60px" : "14px",
              overflow: "auto",
              borderLeft: "2px solid color-mix(in oklch, var(--page-accent) 25%, var(--border))",
            }}
            role="combobox"
            aria-autocomplete="list"
            aria-expanded={mentionOpen}
            aria-controls={mentionOpen ? mentionPopupId : undefined}
            aria-activedescendant={activeMentionId}
          />
          {charCount > 200 ? (
            <span
              className="absolute right-3 bottom-2.5"
              style={{
                fontSize: "10px",
                color: charCount > 4000 ? "var(--error)" : "var(--fg-muted)",
                opacity: 0.6,
              }}
            >
              {charCount}
            </span>
          ) : null}
        </div>
        <button
          type="button"
          onClick={handleSend}
          disabled={disabled || (!draft.trim() && pendingFiles.length === 0)}
          className="ui-button ui-button-accent-fill ui-button-stamp shrink-0"
          title={t("Enter to send, Shift+Enter for newline")}
          aria-label={t("Send message")}
        >
          {uploading ? t("Uploading...") : isBusy ? "···" : t("Send")}
        </button>
      </div>
      {wsStatus !== "connected" ? (
        <p className="mx-auto mt-2 max-w-3xl text-xs text-warn">
          {wsStatus === "connecting"
            ? t("Connecting to daemon...")
            : t("Disconnected. Reconnecting...")}
        </p>
      ) : null}
      {cropTarget ? (
        <ImageCropModal
          imageSrc={cropTarget.objectUrl}
          fileName={cropTarget.file.name}
          onConfirm={handleCropConfirm}
          onCancel={handleCropCancel}
        />
      ) : null}
    </div>
  );
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)}KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)}MB`;
}
