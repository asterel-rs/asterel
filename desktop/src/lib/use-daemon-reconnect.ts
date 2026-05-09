import { useEffect, useRef } from "react";
import { healthCheck } from "@/lib/api";
import { sendNotification } from "@/lib/desktop-shell";
import { translateNow } from "@/lib/i18n-core";
import { useConnectionStore } from "@/stores/connection";

const RECONNECT_INTERVAL_MS = 5_000;
const MAX_SILENT_RETRIES = 3;

export function useDaemonReconnect() {
  const token = useConnectionStore((s) => s.token);
  const status = useConnectionStore((s) => s.status);
  const setStatus = useConnectionStore((s) => s.setStatus);
  const requestWsReconnect = useConnectionStore((s) => s.requestWsReconnect);
  const httpPollResetVersion = useConnectionStore((s) => s.httpPollResetVersion);
  const retriesRef = useRef(0);

  useEffect(() => {
    void httpPollResetVersion;
    retriesRef.current = 0;
  }, [httpPollResetVersion]);

  useEffect(() => {
    if (!token) return;
    if (status === "connected") {
      retriesRef.current = 0;
      return;
    }

    let cancelled = false;
    let handle: ReturnType<typeof setTimeout> | null = null;

    const scheduleProbe = () => {
      handle = setTimeout(() => {
        void probe();
      }, RECONNECT_INTERVAL_MS);
    };

    async function probe() {
      try {
        const res = await healthCheck();
        if (cancelled) return;
        if (res.status === 200) {
          retriesRef.current = 0;
          requestWsReconnect();
        } else {
          throw new Error("non-200");
        }
      } catch {
        if (cancelled) return;
        retriesRef.current += 1;
        setStatus("connecting");
        if (retriesRef.current === MAX_SILENT_RETRIES) {
          sendNotification(
            translateNow("Asterel"),
            translateNow("Daemon unreachable. Retrying..."),
          );
        }
      } finally {
        if (!cancelled) {
          scheduleProbe();
        }
      }
    }

    void probe();
    return () => {
      cancelled = true;
      if (handle) {
        clearTimeout(handle);
      }
    };
  }, [token, status, setStatus, requestWsReconnect]);
}
