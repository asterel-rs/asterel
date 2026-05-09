import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createRouter, RouterProvider } from "@tanstack/react-router";
import { LazyMotion, MotionConfig, domAnimation } from "motion/react";
import React from "react";
import ReactDOM from "react-dom/client";
import { I18nProvider } from "@/lib/i18n";
import "@/stores/theme";
import { routeTree } from "./routeTree.gen";
import "./app.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      retry: 1,
      staleTime: 10_000,
    },
  },
});

const router = createRouter({
  routeTree,
  context: { queryClient },
  defaultPreload: "intent",
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <LazyMotion features={domAnimation}>
      <MotionConfig
        transition={{ type: "spring", stiffness: 400, damping: 30 }}
        reducedMotion="user"
      >
        <QueryClientProvider client={queryClient}>
          <I18nProvider>
            <RouterProvider router={router} />
          </I18nProvider>
        </QueryClientProvider>
      </MotionConfig>
    </LazyMotion>
  </React.StrictMode>,
);
