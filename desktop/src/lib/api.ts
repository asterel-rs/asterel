import { invoke } from "@tauri-apps/api/core";
import { translateNow } from "@/lib/i18n-core";
import { useConnectionStore } from "@/stores/connection";
import { adminPath } from "./admin-contract.generated";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Browser-mode fetch — uses Vite proxy (same-origin), bypassing CORS. */
async function browserFetch<T>(
  method: string,
  path: string,
  opts?: { body?: object; headers?: Record<string, string>; signal?: AbortSignal },
): Promise<DaemonResponse<T>> {
  const token = getToken();
  const headers: Record<string, string> = { ...opts?.headers };
  if (token) headers["Authorization"] = `Bearer ${token}`;
  if (opts?.body) headers["Content-Type"] = "application/json";

  const res = await fetch(`/api${path}`, {
    method,
    headers,
    ...(opts?.body ? { body: JSON.stringify(opts.body) } : {}),
    ...(opts?.signal ? { signal: opts.signal } : {}),
  });
  const body = await res.json().catch(() => ({}) as T);
  return { status: res.status, body };
}

import type {
  ActivityTimelineResponse,
  AgentListResponse,
  AuthProfilesResponse,
  ChannelsResponse,
  CompanionCaptionListResponse,
  CompanionConfigPatch,
  CompanionIngressPayload,
  CompanionScopeListResponse,
  CompanionWidgetListResponse,
  CompanionWindowListResponse,
  CronJob,
  CronJobListResponse,
  DaemonResponse,
  MoodResponse,
  Session,
  SessionListResponse,
  SessionMessageListResponse,
  StatusResponse,
  UsageResponse,
  MemoryConsolidationStatusResponse,
  MemoryEntitiesResponse,
  MemoryExposureStatusResponse,
  MemorySlotsResponse,
  GovernanceSummary,
  ProvidersResponse,
  RuntimeStatus,
  SettingsResponse,
  SkillsResponse,
  TenantContextResponse,
  TenantContextUpdateResponse,
  TenantsResponse,
} from "./types";

function getToken(): string | null {
  return useConnectionStore.getState().token;
}

function translateApiError(status: number, path: string): string {
  if (status === 401) return translateNow("Authentication expired. Re-pair with the daemon.");
  if (status === 403) return translateNow("Permission denied by daemon security policy.");
  if (status === 404) return translateNow("Resource not found: {{path}}", { path });
  if (status === 409 && path === "/admin/v1/memory/correct") {
    return translateNow("Memory changed since you opened it. Refresh the slot and try again.");
  }
  if (status === 500) return translateNow("Internal daemon error. Check daemon logs.");
  if (status === 502 || status === 503)
    return translateNow("Daemon is starting up or unreachable.");
  return translateNow("Request failed ({{status}})", { status });
}

export async function healthCheck(): Promise<DaemonResponse> {
  try {
    if (isTauri()) {
      return await invoke<DaemonResponse>("health_check");
    }
    return await browserFetch("GET", "/health");
  } catch {
    throw new Error(translateNow("Cannot reach daemon. Is it running?"));
  }
}

export async function pairWithDaemon(code: string): Promise<DaemonResponse<{ token: string }>> {
  try {
    if (isTauri()) {
      return await invoke<DaemonResponse<{ token: string }>>("pair_with_daemon", {
        req: { code },
      });
    }
    return await browserFetch<{ token: string }>("POST", "/pair", {
      headers: { "X-Pairing-Code": code },
    });
  } catch {
    throw new Error(translateNow("Pairing failed. Check that the daemon is running."));
  }
}

async function daemonGet<T>(path: string, signal?: AbortSignal): Promise<T> {
  let res: DaemonResponse<T>;
  try {
    if (isTauri()) {
      const token = getToken();
      res = await invoke<DaemonResponse<T>>("daemon_request", {
        params: { method: "GET", path, token },
      });
    } else {
      res = await browserFetch<T>("GET", path, signal !== undefined ? { signal } : undefined);
    }
  } catch {
    throw new Error(translateNow("Cannot reach daemon. Is it running?"));
  }
  if (res.status >= 400) {
    throw new Error(translateApiError(res.status, path));
  }
  return res.body;
}

async function daemonPost<T>(path: string, body?: object, signal?: AbortSignal): Promise<T> {
  let res: DaemonResponse<T>;
  try {
    if (isTauri()) {
      const token = getToken();
      res = await invoke<DaemonResponse<T>>("daemon_request", {
        params: { method: "POST", path, body, token },
      });
    } else {
      res = await browserFetch<T>("POST", path, {
        ...(body !== undefined ? { body } : {}),
        ...(signal !== undefined ? { signal } : {}),
      });
    }
  } catch {
    throw new Error(translateNow("Cannot reach daemon. Is it running?"));
  }
  if (res.status >= 400) {
    throw new Error(translateApiError(res.status, path));
  }
  return res.body;
}

async function daemonPatch<T>(path: string, body?: object, signal?: AbortSignal): Promise<T> {
  let res: DaemonResponse<T>;
  try {
    if (isTauri()) {
      const token = getToken();
      res = await invoke<DaemonResponse<T>>("daemon_request", {
        params: { method: "PATCH", path, body, token },
      });
    } else {
      res = await browserFetch<T>("PATCH", path, {
        ...(body !== undefined ? { body } : {}),
        ...(signal !== undefined ? { signal } : {}),
      });
    }
  } catch {
    throw new Error(translateNow("Cannot reach daemon. Is it running?"));
  }
  if (res.status >= 400) {
    throw new Error(translateApiError(res.status, path));
  }
  return res.body;
}

export async function fetchRuntime(): Promise<RuntimeStatus> {
  return daemonGet<RuntimeStatus>(adminPath("adminRuntime"));
}

export async function fetchUsage(): Promise<UsageResponse> {
  return daemonGet<UsageResponse>(adminPath("adminUsage"));
}

export async function fetchMood(): Promise<MoodResponse> {
  return daemonGet<MoodResponse>(adminPath("adminMood"));
}

export async function fetchActivityTimeline(): Promise<ActivityTimelineResponse> {
  return daemonGet<ActivityTimelineResponse>(adminPath("adminActivity"));
}

export async function restartGateway(): Promise<StatusResponse> {
  return daemonPost<StatusResponse>(adminPath("adminGatewayRestart"));
}

export async function fetchSessions(): Promise<SessionListResponse> {
  return daemonGet<SessionListResponse>(adminPath("adminSessions"));
}

export async function fetchMessages(sessionId: string): Promise<SessionMessageListResponse> {
  return daemonGet<SessionMessageListResponse>(
    adminPath("adminSessionMessages", { session_id: sessionId }),
  );
}

export async function fetchMemoryEntities(): Promise<MemoryEntitiesResponse> {
  return daemonGet<MemoryEntitiesResponse>(adminPath("adminMemoryEntities"));
}

export async function fetchMemoryConsolidationStatus(): Promise<MemoryConsolidationStatusResponse> {
  return daemonGet<MemoryConsolidationStatusResponse>(adminPath("adminMemoryConsolidation"));
}

export async function fetchMemoryExposureStatus(): Promise<MemoryExposureStatusResponse> {
  return daemonGet<MemoryExposureStatusResponse>(adminPath("adminMemoryExposure"));
}

export async function fetchMemorySlots(entityId: string): Promise<MemorySlotsResponse> {
  return daemonGet<MemorySlotsResponse>(
    adminPath("adminMemoryEntitySlots", { entity_id: entityId }),
  );
}

export async function correctMemorySlot(body: {
  entity_id: string;
  slot_key: string;
  old_value: string;
  new_value: string;
  reason: string;
}): Promise<{ status: string; event_id: string }> {
  return daemonPost(adminPath("adminMemoryCorrect"), body);
}

export async function forgetMemorySlot(body: {
  entity_id: string;
  slot_key: string;
  reason: string;
  mode?: "soft" | "hard" | "tombstone";
}): Promise<{ status?: string; mode?: string }> {
  return daemonPost(adminPath("adminMemoryForget"), body);
}

export async function fetchAuthProfiles(): Promise<AuthProfilesResponse> {
  return daemonGet<AuthProfilesResponse>(adminPath("adminAuthProfiles"));
}

export async function fetchProviders(): Promise<ProvidersResponse> {
  return daemonGet<ProvidersResponse>(adminPath("adminProviders"));
}

export async function fetchSettings(): Promise<SettingsResponse> {
  return daemonGet<SettingsResponse>(adminPath("adminSettings"));
}

async function daemonDelete<T>(path: string): Promise<T> {
  let res: DaemonResponse<T>;
  try {
    if (isTauri()) {
      const token = getToken();
      res = await invoke<DaemonResponse<T>>("daemon_request", {
        params: { method: "DELETE", path, token },
      });
    } else {
      res = await browserFetch<T>("DELETE", path);
    }
  } catch {
    throw new Error(translateNow("Cannot reach daemon. Is it running?"));
  }
  if (res.status >= 400) {
    throw new Error(translateApiError(res.status, path));
  }
  return res.body;
}

export async function fetchChannels(): Promise<ChannelsResponse> {
  return daemonGet<ChannelsResponse>(adminPath("adminChannels"));
}

export async function fetchSkills(): Promise<SkillsResponse> {
  return daemonGet<SkillsResponse>(adminPath("adminSkills"));
}

export async function installSkill(source: string): Promise<StatusResponse> {
  return daemonPost<StatusResponse>(adminPath("adminSkillInstall"), { source });
}

export async function removeSkill(name: string): Promise<StatusResponse> {
  return daemonDelete<StatusResponse>(adminPath("adminSkillDelete", { skill_id: name }));
}

export async function fetchCronJobs(): Promise<CronJobListResponse> {
  return daemonGet<CronJobListResponse>(adminPath("adminCronJobs"));
}

export async function createCronJob(expression: string, command: string): Promise<CronJob> {
  return daemonPost<CronJob>(adminPath("adminCronJobCreate"), { expression, command });
}

export async function removeCronJob(id: string): Promise<StatusResponse> {
  return daemonDelete<StatusResponse>(adminPath("adminCronJobDelete", { job_id: id }));
}

export async function patchChannel(
  id: string,
  body: { enabled?: boolean },
): Promise<{ status: string }> {
  return daemonPatch(adminPath("adminChannelPatch", { channel_id: id }), body);
}

export async function channelAction(
  id: string,
  action: "doctor" | "test",
): Promise<{ status: string; result?: string }> {
  return daemonPost(adminPath("adminChannelAction", { channel_id: id }), {
    action,
  });
}

export async function patchSkill(
  id: string,
  body: { enabled?: boolean },
): Promise<{ status: string }> {
  return daemonPatch(adminPath("adminSkillPatch", { skill_id: id }), body);
}

export async function patchCronJob(
  id: string,
  body: { expression?: string; command?: string; enabled?: boolean },
): Promise<CronJob> {
  return daemonPatch<CronJob>(adminPath("adminCronJobPatch", { job_id: id }), body);
}

export async function runCronJob(id: string): Promise<{ status: string }> {
  return daemonPost(adminPath("adminCronJobRun", { job_id: id }));
}

export async function fetchCompanionScopes(): Promise<CompanionScopeListResponse> {
  return daemonGet<CompanionScopeListResponse>(adminPath("adminCompanions"));
}

export async function fetchCompanionCaptions(scope: string): Promise<CompanionCaptionListResponse> {
  return daemonGet<CompanionCaptionListResponse>(adminPath("adminCompanionCaptions", { scope }));
}

export async function fetchCompanionWidgets(scope: string): Promise<CompanionWidgetListResponse> {
  return daemonGet<CompanionWidgetListResponse>(adminPath("adminCompanionWidgets", { scope }));
}

export async function fetchCompanionWindows(scope: string): Promise<CompanionWindowListResponse> {
  return daemonGet<CompanionWindowListResponse>(adminPath("adminCompanionWindows", { scope }));
}

export async function fetchTenants(): Promise<TenantsResponse> {
  return daemonGet<TenantsResponse>(adminPath("adminTenants"));
}

export async function fetchTenantContext(): Promise<TenantContextResponse> {
  return daemonGet<TenantContextResponse>(adminPath("adminTenantContext"));
}

export async function patchCompanionConfig(
  config: CompanionConfigPatch,
): Promise<{ status: string }> {
  return daemonPatch(adminPath("adminCompanionsPatch"), config);
}

export async function postCompanionIngress(
  scope: string,
  payload: CompanionIngressPayload,
): Promise<{ status: string }> {
  return daemonPost(adminPath("adminCompanionIngress", { scope }), payload);
}

export async function postTenantContext(
  tenantId: string | null,
): Promise<TenantContextUpdateResponse> {
  return daemonPost(adminPath("adminTenantContextSet"), { tenant_id: tenantId });
}

// --- Session CRUD ---

export async function createSession(title?: string): Promise<Session> {
  return title
    ? daemonPost<Session>(adminPath("adminSessionCreate"), { title })
    : daemonPost<Session>(adminPath("adminSessionCreate"));
}

export async function deleteSession(sessionId: string): Promise<void> {
  await daemonDelete(adminPath("adminSessionDelete", { session_id: sessionId }));
}

// --- Settings mutations ---

export async function patchSettings(body: Record<string, unknown>): Promise<{ status: string }> {
  return daemonPatch(adminPath("adminSettingsPatch"), body);
}

export async function patchProviders(body: Record<string, unknown>): Promise<{ status: string }> {
  return daemonPatch(adminPath("adminProvidersPatch"), body);
}

// --- Auth profile mutations ---

export async function patchAuthProfile(
  id: string,
  body: { disabled?: boolean },
): Promise<{ status: string }> {
  return daemonPatch(adminPath("adminAuthProfilePatch", { id }), body);
}

// --- Agents ---

export async function fetchAgents(): Promise<AgentListResponse> {
  return daemonGet<AgentListResponse>(adminPath("adminAgents"));
}

// --- File upload ---

export interface UploadResult {
  upload_id: string;
  filename: string;
  content_type: string;
}

export async function uploadFile(file: File): Promise<UploadResult> {
  const formData = new FormData();
  formData.append("file", file);

  const token = getToken();
  const uploadPath = adminPath("adminUpload");
  const res = await fetch(`/api${uploadPath}`, {
    method: "POST",
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    body: formData,
  });

  if (res.status >= 400) {
    throw new Error(translateApiError(res.status, uploadPath));
  }

  const body = await res.json();
  const item = body.items?.[0];
  return {
    upload_id: item?.upload_id ?? item?.stored_path ?? file.name,
    filename: file.name,
    content_type: file.type || "application/octet-stream",
  };
}

// --- Companion window actions ---

export async function confirmCompanionWindow(
  scope: string,
  windowId: string,
): Promise<{ status: string }> {
  return daemonPost(adminPath("adminCompanionWindowConfirm", { scope, window_id: windowId }));
}

export async function fetchGovernanceSummary(): Promise<GovernanceSummary> {
  return daemonGet<GovernanceSummary>(adminPath("adminGovernance"));
}

export async function cancelCompanionWindow(
  scope: string,
  windowId: string,
): Promise<{ status: string }> {
  return daemonPost(adminPath("adminCompanionWindowCancel", { scope, window_id: windowId }));
}
