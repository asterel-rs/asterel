import { create } from "zustand";
import { persist } from "zustand/middleware";

export type ConnectionStatus = "disconnected" | "connecting" | "connected" | "error";

interface ConnectionState {
  daemonUrl: string;
  token: string | null;
  status: ConnectionStatus;
  wsReconnectVersion: number;
  httpPollResetVersion: number;
  setDaemonUrl: (url: string) => void;
  setToken: (token: string) => void;
  clearToken: () => void;
  setStatus: (status: ConnectionStatus) => void;
  requestWsReconnect: () => void;
  acknowledgeWsConnected: () => void;
}

export const useConnectionStore = create<ConnectionState>()(
  persist(
    (set) => ({
      daemonUrl: "http://127.0.0.1:3000",
      token: null,
      status: "disconnected",
      wsReconnectVersion: 0,
      httpPollResetVersion: 0,
      setDaemonUrl: (url) => set({ daemonUrl: url }),
      setToken: (token) =>
        set((state) => ({
          token,
          status: "connecting",
          wsReconnectVersion: state.wsReconnectVersion + 1,
        })),
      clearToken: () => set({ token: null, status: "disconnected" }),
      setStatus: (status) => set({ status }),
      requestWsReconnect: () =>
        set((state) => ({
          status: state.token ? "connecting" : "disconnected",
          wsReconnectVersion: state.wsReconnectVersion + 1,
        })),
      acknowledgeWsConnected: () =>
        set((state) => ({
          status: "connected",
          httpPollResetVersion: state.httpPollResetVersion + 1,
        })),
    }),
    {
      name: "asterel-connection",
      partialize: (state) => ({
        daemonUrl: state.daemonUrl,
        token: state.token,
        status: state.status,
      }),
    },
  ),
);
