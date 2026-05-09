import type { QueryClient } from "@tanstack/react-query";
import {
  createRootRouteWithContext,
  Outlet,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import { Component, type ReactNode, useEffect } from "react";
import { Sidebar } from "@/components/sidebar";
import { useI18n } from "@/lib/i18n";
import { translateNow } from "@/lib/i18n-core";
import { useDaemonReconnect } from "@/lib/use-daemon-reconnect";

interface RouterContext {
  queryClient: QueryClient;
}

export const Route = createRootRouteWithContext<RouterContext>()({
  component: RootLayout,
});

function RootLayout() {
  const { t } = useI18n();
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  });
  const showSidebar = pathname !== "/pair";

  useDaemonReconnect();
  useTrayNavigation();

  return (
    <div className="app-shell flex h-full">
      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-50 focus:px-4 focus:py-2 ui-button ui-button-accent-fill"
      >
        {t("Skip to content")}
      </a>
      {showSidebar ? <Sidebar /> : null}
      <main
        id="main-content"
        className={
          showSidebar ? "app-main flex-1 overflow-y-auto" : "app-main flex-1 overflow-hidden"
        }
      >
        <RootErrorBoundary>
          <Outlet />
        </RootErrorBoundary>
      </main>
    </div>
  );
}

function useTrayNavigation() {
  const navigate = useNavigate();

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        unlisten = await listen<string>("tray-navigate", (event) => {
          navigate({
            to: event.payload as
              | "/"
              | "/companion"
              | "/dashboard"
              | "/memory"
              | "/pair"
              | "/channels"
              | "/chat"
              | "/extensions"
              | "/sessions"
              | "/settings",
            search: {},
          });
        });
      } catch {
        // Not in Tauri context — ignore
      }
    })();

    return () => {
      unlisten?.();
    };
  }, [navigate]);
}

class RootErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: { componentStack?: string | null }) {
    console.error("[ErrorBoundary]", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="flex h-full items-center justify-center px-6">
          <div className="max-w-md space-y-4 text-center">
            <p
              className="font-display"
              style={{
                fontSize: "20px",
                fontWeight: 700,
                color: "var(--error)",
              }}
            >
              {translateNow("Something went wrong")}
            </p>
            <p
              style={{
                fontSize: "13px",
                lineHeight: 1.85,
                color: "var(--fg-muted)",
              }}
            >
              {this.state.error.message}
            </p>
            <button
              type="button"
              onClick={() => this.setState({ error: null })}
              className="ui-button ui-button-accent-fill px-4 py-2"
            >
              {translateNow("Try again")}
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}
