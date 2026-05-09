import { create } from "zustand";
import { persist } from "zustand/middleware";

export type Theme = "light" | "dark" | "system";

interface ThemeState {
  theme: Theme;
  setTheme: (theme: Theme) => void;
}

function applyThemeClass(theme: Theme) {
  const root = document.documentElement;
  if (theme === "dark") {
    root.classList.add("dark");
  } else if (theme === "light") {
    root.classList.remove("dark");
  } else {
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    root.classList.toggle("dark", prefersDark);
  }
}

export const useThemeStore = create<ThemeState>()(
  persist(
    (set) => ({
      theme: "system",
      setTheme: (theme) => {
        applyThemeClass(theme);
        set({ theme });
      },
    }),
    { name: "asterel-theme" },
  ),
);

function onSystemChange(e: MediaQueryListEvent) {
  const { theme } = useThemeStore.getState();
  if (theme === "system") {
    document.documentElement.classList.toggle("dark", e.matches);
  }
}

const mql = window.matchMedia("(prefers-color-scheme: dark)");
mql.addEventListener("change", onSystemChange);

applyThemeClass(useThemeStore.getState().theme);
