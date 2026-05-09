export type {
  ActivityEvent,
  ActivityTimelineResponse,
  AgentEntry,
  AgentListResponse,
  AuthProfile,
  AuthProfilesResponse,
  Channel,
  ChannelsResponse,
  CompanionCaption,
  CompanionCaptionListResponse,
  CompanionScope,
  CompanionScopeListResponse,
  CompanionWidget,
  CompanionWidgetListResponse,
  CompanionWindowEntry,
  CompanionWindowListResponse,
  CronJob,
  CronJobListResponse,
  DomainTrustEntry,
  GovernanceSummary,
  MemoryConsolidationStatusResponse,
  MemoryConsolidationWorker,
  MemoryEntitiesResponse,
  MemoryEntitySummary,
  MemoryExposureStatusResponse,
  MemorySlotProvenance,
  MemorySlotsResponse,
  MemorySlotSummary,
  Message,
  MoodResponse,
  PendingWindow,
  ProvidersResponse,
  RuntimeStatus,
  Session,
  SessionListResponse,
  SessionMessageListResponse,
  SettingsResponse,
  Skill,
  SkillsResponse,
  SkillTool,
  StatusResponse,
  TenantContextResponse,
  TenantContextUpdateResponse,
  TenantRegistryRow,
  TenantsResponse,
  UsageResponse,
} from "./admin-contract.generated";

export interface DaemonResponse<T = unknown> {
  status: number;
  body: T;
}

export interface CompanionConfigPatch {
  enabled?: boolean;
  caption_retention_seconds?: number;
  widget_ttl_seconds?: number;
  [key: string]: unknown;
}

export interface CompanionIngressPayload {
  kind: "text" | "clipboard" | "file" | "screenshot";
  content: string;
  mime_type?: string;
  metadata?: Record<string, unknown>;
}

export interface ChatMessage {
  id: string;
  role: string;
  content: string;
  created_at: string;
  input_tokens?: number;
  output_tokens?: number;
}

export interface ToolCallState {
  id: string;
  name: string;
  status: "running" | "completed" | "failed";
  detail?: string;
}
