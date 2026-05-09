import { useEffect, useRef } from "react";
import { StatusBadge } from "@/components/status-badge";
import type { AgentEntry } from "@/lib/types";

export function MentionPopup({
  id,
  agents,
  filter,
  selectedIndex,
  onSelect,
}: {
  id: string;
  agents: AgentEntry[];
  filter: string;
  selectedIndex: number;
  onSelect: (agent: AgentEntry) => void;
}) {
  const listRef = useRef<HTMLDivElement>(null);

  const filtered = agents.filter((a) => a.name.toLowerCase().includes(filter.toLowerCase()));

  useEffect(() => {
    if (listRef.current && selectedIndex >= 0) {
      const el = listRef.current.children[selectedIndex] as HTMLElement;
      el?.scrollIntoView({ block: "nearest" });
    }
  }, [selectedIndex]);

  if (filtered.length === 0) return null;

  return (
    <div
      id={id}
      ref={listRef}
      className="chat-mention-popup absolute bottom-full left-0 mb-1"
      style={{ minWidth: "200px", maxHeight: "180px", zIndex: 20 }}
      role="listbox"
    >
      {filtered.map((agent, i) => (
        <button
          id={`mention-option-${agent.id}`}
          key={agent.id}
          type="button"
          onClick={() => onSelect(agent)}
          data-selected={i === selectedIndex}
          className="chat-mention-option"
          style={{ position: "relative" }}
          role="option"
          aria-selected={i === selectedIndex}
        >
          <span style={{ color: "var(--accent-strong)", fontWeight: 700 }}>@</span>
          <span>{agent.name}</span>
          <span
            style={{
              fontSize: "10px",
              color: "var(--fg-muted)",
              marginLeft: "auto",
            }}
          >
            {agent.model}
          </span>
          <StatusBadge
            variant={agent.status === "active" || agent.status === "running" ? "ok" : "neutral"}
            label={agent.status}
          />
        </button>
      ))}
    </div>
  );
}
