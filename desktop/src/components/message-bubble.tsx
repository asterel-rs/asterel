import { lazy, memo, Suspense } from "react";
import { useI18n } from "@/lib/i18n";
import { formatDateTime, formatNumber } from "@/lib/i18n-core";
import type { Message } from "@/lib/types";

interface MessageBubbleProps {
  message: Message;
}

const loadMarkdownMessageContent = () =>
  import("@/components/markdown-message-content").then((module) => ({
    default: module.MarkdownMessageContent,
  }));

export const MarkdownMessageContent = lazy(loadMarkdownMessageContent);

export function preloadMarkdownMessageContent() {
  return loadMarkdownMessageContent();
}

export function hasRichMarkdown(content: string): boolean {
  return (
    /```/.test(content) ||
    /`[^`\n]+`/.test(content) ||
    /^\s{0,3}(#{1,6}|\* |- |\+ |> )/m.test(content) ||
    /^\s{0,3}\d+\.\s/m.test(content) ||
    /\[[^\]]+\]\([^)]+\)/.test(content) ||
    /\|.+\|/.test(content)
  );
}

export const MessageBubble = memo(function MessageBubble({ message }: MessageBubbleProps) {
  const { locale, t } = useI18n();
  const ts = formatDateTime(
    message.created_at,
    {
      hour: "2-digit",
      minute: "2-digit",
    },
    locale,
  );

  const hasTokens = message.input_tokens !== undefined || message.output_tokens !== undefined;
  const renderRichMarkdown = hasRichMarkdown(message.content);

  return (
    <div className="border-b border-[var(--border)] py-2">
      <div className="flex items-center gap-3">
        <span className="ui-meta-label uppercase tracking-wider">{message.role}</span>
        <span className="text-muted text-[10px] tabular-nums">{ts}</span>
        {hasTokens ? (
          <span className="text-muted text-[10px] tabular-nums">
            {message.input_tokens !== undefined
              ? `${formatNumber(message.input_tokens, locale)} ${t("in")}`
              : null}
            {message.input_tokens !== undefined && message.output_tokens !== undefined
              ? " / "
              : null}
            {message.output_tokens !== undefined
              ? `${formatNumber(message.output_tokens, locale)} ${t("out")}`
              : null}
          </span>
        ) : null}
      </div>

      <div className="mt-2 text-[13px]">
        {renderRichMarkdown ? (
          <Suspense fallback={<PlainMessageContent content={message.content} />}>
            <MarkdownMessageContent content={message.content} />
          </Suspense>
        ) : (
          <PlainMessageContent content={message.content} />
        )}
      </div>
    </div>
  );
}, areMessagesEqual);

function PlainMessageContent({ content }: { content: string }) {
  return (
    <div
      style={{
        color: "var(--fg)",
        lineHeight: 1.65,
        whiteSpace: "pre-wrap",
        wordBreak: "break-word",
      }}
    >
      {content}
    </div>
  );
}

function areMessagesEqual(previous: MessageBubbleProps, next: MessageBubbleProps): boolean {
  return (
    previous.message.id === next.message.id &&
    previous.message.role === next.message.role &&
    previous.message.content === next.message.content &&
    previous.message.created_at === next.message.created_at &&
    previous.message.input_tokens === next.message.input_tokens &&
    previous.message.output_tokens === next.message.output_tokens
  );
}
