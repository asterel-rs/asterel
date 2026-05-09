import { useCallback, useState } from "react";
import Markdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import { useI18n } from "@/lib/i18n";

export function MarkdownMessageContent({ content }: { content: string }) {
  return (
    <div className="chat-markdown max-w-none">
      <Markdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeHighlight]}
        components={{ pre: CodeBlockWrapper, code: InlineCode }}
      >
        {content}
      </Markdown>
    </div>
  );
}

/** Wraps fenced code blocks with language label + copy button */
function CodeBlockWrapper({ children, ...props }: React.HTMLAttributes<HTMLPreElement>) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);

  // Extract language and text from the child <code> element
  const codeElement = Array.isArray(children) ? children[0] : children;
  const lang = extractLanguage(codeElement);
  const text = extractText(codeElement);

  const handleCopy = useCallback(() => {
    if (!text) return;
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [text]);

  return (
    <div className="chat-code-block-wrap">
      <div className="chat-code-block-header">
        <span className="chat-code-block-lang">{lang || "code"}</span>
        <button
          type="button"
          onClick={handleCopy}
          data-copied={copied || undefined}
          className="chat-code-block-copy"
        >
          {copied ? t("Copied") : t("Copy")}
        </button>
      </div>
      <pre {...props} className="chat-code-block">
        {children}
      </pre>
    </div>
  );
}

/** Inline code — don't wrap in CodeBlockWrapper */
function InlineCode({ children, className, ...props }: React.HTMLAttributes<HTMLElement>) {
  // If it has a language class, it's inside a <pre> — render as-is
  if (className?.startsWith("hljs") || className?.startsWith("language-")) {
    return (
      <code className={className} {...props}>
        {children}
      </code>
    );
  }

  return (
    <code className="chat-inline-code" {...props}>
      {children}
    </code>
  );
}

function extractLanguage(node: unknown): string | null {
  if (!node || typeof node !== "object") return null;
  const el = node as { props?: { className?: string } };
  const className = el.props?.className ?? "";
  // rehype-highlight adds "hljs language-xxx" classes
  const match = className.match(/language-(\S+)/);
  return match?.[1] ?? null;
}

function extractText(node: unknown): string | null {
  if (!node || typeof node !== "object") return null;
  const el = node as { props?: { children?: unknown } };
  const children = el.props?.children;
  if (typeof children === "string") return children;
  if (Array.isArray(children)) {
    return children
      .map((child: unknown) => {
        if (typeof child === "string") return child;
        if (child && typeof child === "object") {
          const c = child as { props?: { children?: unknown } };
          return typeof c.props?.children === "string" ? c.props.children : "";
        }
        return "";
      })
      .join("");
  }
  return null;
}
