import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useRouterState } from "@tanstack/react-router";
import { fetchMood } from "@/lib/api";
import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { getOpticalInlineControlStyle } from "@/lib/ui-polish";
import { useConnectionStore } from "@/stores/connection";
import { type Theme, useThemeStore } from "@/stores/theme";

interface NavItem {
  to:
    | "/dashboard"
    | "/sessions"
    | "/companion"
    | "/memory"
    | "/channels"
    | "/settings"
    | "/chat"
    | "/extensions";
  label: string;
  nested?: boolean;
}

interface NavSection {
  label: string;
  color: string;
  dotColor: string;
  items: NavItem[];
}

const navSections: NavSection[] = [
  {
    label: "Overview",
    color: "var(--section-overview-label)",
    dotColor: "var(--section-overview)",
    items: [{ to: "/dashboard", label: "Dashboard" }],
  },
  {
    label: "Companion console",
    color: "var(--section-operations-label)",
    dotColor: "var(--section-operations)",
    items: [
      { to: "/sessions", label: "Sessions" },
      { to: "/companion", label: "Companion" },
      { to: "/memory", label: "Memory" },
      { to: "/channels", label: "Channels" },
      { to: "/settings", label: "Settings" },
    ],
  },
  {
    label: "Secondary surfaces",
    color: "var(--section-system-label)",
    dotColor: "var(--section-system)",
    items: [
      { to: "/chat", label: "Chat sandbox" },
      { to: "/extensions", label: "Tools" },
    ],
  },
];

export function Sidebar() {
  const { locale, setLocale, t } = useI18n();
  const status = useConnectionStore((s) => s.status);
  const theme = useThemeStore((s) => s.theme);
  const setTheme = useThemeStore((s) => s.setTheme);
  const navigate = useNavigate();
  const routerState = useRouterState();
  const currentPath = routerState.location.pathname;

  const daemonOnline = status === "connected";

  const moodQuery = useQuery({
    queryKey: ["mood"],
    queryFn: fetchMood,
    refetchInterval: 30_000,
    enabled: daemonOnline,
  });
  const compactControlStyle = getOpticalInlineControlStyle({ density: "compact" });

  return (
    <aside
      aria-label="Sidebar"
      className="app-sidebar flex h-full w-[248px] shrink-0 flex-col border-r border-[var(--border)]"
    >
      <div
        data-tauri-drag-region
        className="app-sidebar-brand shrink-0 border-b border-[var(--border)] px-4 py-4"
      >
        <div className="space-y-3">
          <span className="app-kicker select-none">Asterel</span>
          <div className="space-y-1">
            <p className="font-display text-[22px] font-semibold leading-[0.95] tracking-[-0.05em]">
              {t("Operator Console")}
            </p>
            <p className="text-muted text-[11px]">
              {moodQuery.data
                ? moodQuery.data.description
                : t("Review runtime health, sessions, memory, and channel posture.")}
            </p>
          </div>
        </div>
      </div>

      <nav aria-label="Main navigation" className="flex-1 overflow-y-auto overflow-x-hidden py-3">
        {navSections.map((section, idx) => (
          <div key={section.label} className={cn(idx > 0 && "mt-3")}>
            <div className="flex items-center gap-2 px-4 pb-1">
              <span
                aria-hidden="true"
                style={{
                  display: "inline-block",
                  width: "5px",
                  height: "5px",
                  borderRadius: "var(--radius-pill)",
                  background: section.dotColor,
                  opacity: 0.7,
                  flexShrink: 0,
                }}
              />
              <span
                className="ui-meta-label select-none uppercase"
                style={{ color: section.color }}
              >
                {t(section.label)}
              </span>
            </div>
            <div>
              {section.items.map((item) => {
                const active = item.nested
                  ? currentPath === item.to
                  : currentPath.startsWith(item.to);
                return (
                  <Link
                    key={item.to}
                    to={item.to}
                    preload="intent"
                    data-active={active}
                    className="ui-sidebar-link"
                    style={{
                      fontSize: "12px",
                      paddingLeft: item.nested ? "28px" : undefined,
                      color: item.nested && !active ? "var(--fg-soft)" : undefined,
                      ...(active
                        ? ({
                            "--marker-color": `color-mix(in oklch, ${section.color} 30%, transparent)`,
                            borderColor: "transparent",
                            color: section.color,
                            background: "transparent",
                          } as React.CSSProperties)
                        : {}),
                    }}
                  >
                    {t(item.label)}
                  </Link>
                );
              })}
            </div>
          </div>
        ))}
      </nav>

      <div className="shrink-0 border-t border-[var(--border)] px-4 py-4">
        <div className="mb-3 space-y-2">
          <span className="ui-meta-label block uppercase">{t("Language")}</span>
          <div className="grid grid-cols-2 gap-2">
            {(["en", "ja"] as const).map((option) => (
              <button
                key={option}
                type="button"
                onClick={() => setLocale(option)}
                data-active={locale === option}
                className="ui-button ui-button-muted text-[10px] font-bold uppercase"
                style={{
                  ...compactControlStyle,
                  color: locale === option ? "var(--accent)" : "var(--fg-soft)",
                }}
              >
                {option === "en" ? t("English") : t("Japanese")}
              </button>
            ))}
          </div>
        </div>
        <div className="mb-3 space-y-2">
          <span className="ui-meta-label block uppercase">{t("Theme")}</span>
          <div className="grid grid-cols-3 gap-2">
            {(["light", "dark", "system"] as const).map((option: Theme) => (
              <button
                key={option}
                type="button"
                onClick={() => setTheme(option)}
                data-active={theme === option}
                className="ui-button ui-button-muted text-[10px] font-bold uppercase"
                style={{
                  ...compactControlStyle,
                  color: theme === option ? "var(--accent)" : "var(--fg-soft)",
                }}
              >
                {option === "light" ? t("Light") : option === "dark" ? t("Dark") : t("System")}
              </button>
            ))}
          </div>
        </div>
        <button
          type="button"
          onClick={() => navigate({ to: "/pair" })}
          className="ui-button ui-button-muted ui-meta-label block w-full text-left uppercase"
          style={{
            ...compactControlStyle,
            color: daemonOnline ? "var(--accent)" : "var(--error)",
          }}
        >
          <span className="text-muted mb-1.5 block">{t("Connection")}</span>
          <span className="block">{daemonOnline ? t("Connected") : t("Waiting to pair")}</span>
        </button>
      </div>
    </aside>
  );
}
