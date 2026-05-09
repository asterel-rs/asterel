import type React from "react";
import type { CSSProperties, ReactNode } from "react";
import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";

export function PageShell({
  children,
  className,
  accent,
}: {
  children: ReactNode;
  className?: string;
  accent?: string;
}) {
  return (
    <div
      className={cn("app-page space-y-10 px-6 py-10 md:px-10", className)}
      style={accent ? ({ "--page-accent": accent } as React.CSSProperties) : undefined}
    >
      {children}
    </div>
  );
}

export function PageHeader({
  eyebrow,
  title,
  description,
  actions,
  aside,
}: {
  eyebrow?: string;
  title: string;
  description: string;
  actions?: ReactNode;
  aside?: ReactNode;
}) {
  const { t } = useI18n();

  return (
    <header className="page-header">
      <div className="page-header-main">
        {eyebrow ? <span className="app-kicker">{t(eyebrow)}</span> : null}
        <div>
          <h1 className="page-title">{t(title)}</h1>
          <p className="page-description mt-3">{t(description)}</p>
        </div>
      </div>
      {aside ? <div className="page-header-aside">{aside}</div> : null}
      {actions ? <div className="page-header-actions">{actions}</div> : null}
    </header>
  );
}

export function StatPill({
  label,
  value,
  hint,
  tone,
}: {
  label: string;
  value: string;
  hint?: string;
  tone?: string;
}) {
  const { t } = useI18n();

  return (
    <div className="ui-stat-pill">
      <p className="ui-stat-pill-label">{t(label)}</p>
      <p className="ui-stat-pill-value" style={{ color: tone ?? "var(--fg)" }}>
        {value}
      </p>
      {hint ? <p className="ui-stat-pill-hint">{t(hint)}</p> : null}
    </div>
  );
}

export function SectionLead({
  title,
  description,
  action,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  const { t } = useI18n();

  return (
    <div className="flex flex-col gap-3 pb-4 md:flex-row md:items-start md:justify-between">
      <div>
        <h2 className="app-section-title">{t(title)}</h2>
        {description ? (
          <p className="text-muted mt-2 text-sm leading-relaxed">{t(description)}</p>
        ) : null}
      </div>
      {action}
    </div>
  );
}

export function Panel({
  children,
  className,
  strong,
  variant,
  style,
}: {
  children: ReactNode;
  className?: string;
  strong?: boolean;
  variant?: "panel" | "stage";
  style?: CSSProperties;
}) {
  const baseClass =
    variant === "stage"
      ? strong
        ? "app-stage-strong"
        : "app-stage"
      : strong
        ? "app-panel-strong"
        : "app-panel";

  return (
    <div className={cn(baseClass, className)} style={style}>
      {children}
    </div>
  );
}
