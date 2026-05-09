import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Panel, SectionLead, StatPill } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { StatusBadge } from "@/components/status-badge";
import { fetchTenantContext, fetchTenants, postTenantContext } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import type { TenantRegistryRow } from "@/lib/types";

function useSwitchTenant() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (tenantId: string | null) => postTenantContext(tenantId),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["tenants"] }),
        queryClient.invalidateQueries({ queryKey: ["tenant-context"] }),
      ]);
    },
  });
}

// ---------------------------------------------------------------------------
// Tenants tab — routing map workbench
// ---------------------------------------------------------------------------

export function TenantsTab() {
  const { t } = useI18n();
  const [expandedTenantId, setExpandedTenantId] = useState<string | null>(null);
  const clearTenantMutation = useSwitchTenant();

  const tenantsQuery = useQuery({
    queryKey: ["tenants"],
    queryFn: fetchTenants,
  });
  const tenantContextQuery = useQuery({
    queryKey: ["tenant-context"],
    queryFn: fetchTenantContext,
  });

  const data = tenantsQuery.data;
  const activeTenant = tenantContextQuery.data?.active_tenant ?? null;
  const tenantRows = data?.rows ?? [];
  const workspaceItems = data?.discovered_workspaces;
  const workspaceList = workspaceItems ?? [];
  const bindingCount = data?.binding_count ?? 0;
  const isEmpty = bindingCount === 0 && workspaceList.length === 0 && activeTenant === null;
  const coverageRatio =
    workspaceList.length > 0
      ? `${Math.min(bindingCount, workspaceList.length)}/${workspaceList.length}`
      : "0/0";
  const isLoading = tenantsQuery.isLoading || tenantContextQuery.isLoading;
  const isError = tenantsQuery.isError || tenantContextQuery.isError;
  const error = tenantsQuery.error ?? tenantContextQuery.error;

  const refetchAll = () => {
    void tenantsQuery.refetch();
    void tenantContextQuery.refetch();
  };

  if (isLoading) {
    return <SkeletonLoader />;
  }

  if (isError) {
    return (
      <ErrorState
        title={t("Failed to load tenants")}
        message={error instanceof Error ? error.message : t("Could not reach the daemon.")}
        onRetry={refetchAll}
      />
    );
  }

  if (isEmpty) {
    return (
      <EmptyState
        title={t("No tenant activity")}
        description={t(
          "Tenants appear when clients send the X-Tenant-ID header. Configure multi-tenant routing in the gateway settings.",
        )}
      />
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-4">
        <div className="flex flex-wrap gap-2">
          <StatPill label={t("Bindings")} value={String(bindingCount)} />
          <StatPill label={t("Tenants")} value={String(tenantRows.length)} tone="var(--accent)" />
          <StatPill label={t("Workspaces")} value={String(workspaceList.length)} />
          <StatPill label={t("Coverage")} value={coverageRatio} tone="var(--warn)" />
        </div>
        <button
          type="button"
          onClick={refetchAll}
          className="ui-button ui-button-muted text-fg px-4 py-2 text-xs font-bold uppercase"
        >
          {t("Refresh map")}
        </button>
      </div>

      {activeTenant ? (
        <Panel variant="stage" className={`px-5 py-5 app-signal-raised`}>
          <SectionLead
            title={t("Active tenant")}
            description={t("Sessions and runtime memory scoped to this tenant")}
            action={
              <button
                type="button"
                onClick={() => clearTenantMutation.mutate(null)}
                disabled={clearTenantMutation.isPending}
                className="ui-button ui-button-muted px-3 py-1 text-xs font-bold uppercase"
                style={{ color: "var(--fg-soft)" }}
              >
                {clearTenantMutation.isPending ? t("Clearing...") : t("Clear context")}
              </button>
            }
          />
          <div className="ui-rule-list mt-4">
            <div className="ui-rule-row">
              <p className="ui-rule-key">{t("Tenant")}</p>
              <p className="ui-rule-value text-accent break-all font-mono">{activeTenant}</p>
            </div>
          </div>
        </Panel>
      ) : null}

      {clearTenantMutation.isError ? (
        <p className="text-error text-xs">
          {clearTenantMutation.error instanceof Error
            ? clearTenantMutation.error.message
            : t("Tenant clear failed.")}
        </p>
      ) : null}

      <div className="grid gap-6 xl:grid-cols-[minmax(0,1fr)_320px]">
        <div>
          <SectionLead
            title={t("Tenant registry")}
            description={t("Expand a tenant to inspect bindings or activate it to scope sessions.")}
            action={
              <span className="ui-chip">{t("{count} groups", { count: tenantRows.length })}</span>
            }
          />
          <div className="mt-4">
            {tenantRows.length > 0 ? (
              tenantRows.map((tenant, index) => (
                <TenantRoutingRow
                  key={tenant.tenant_id}
                  tenant={tenant}
                  index={index}
                  isExpanded={tenant.tenant_id === expandedTenantId}
                  isActive={tenant.tenant_id === activeTenant}
                  onToggle={() =>
                    setExpandedTenantId(
                      tenant.tenant_id === expandedTenantId ? null : tenant.tenant_id,
                    )
                  }
                />
              ))
            ) : (
              <EmptyState
                title={t("No bindings yet")}
                description={t(
                  "Workspace discovery is present, but principal-to-tenant bindings have not been reported.",
                )}
              />
            )}
          </div>
        </div>

        <div className="space-y-6">
          <TenantSwitchPanel tenantRows={tenantRows} activeTenant={activeTenant} />

          <Panel variant="stage" className="px-5 py-5">
            <SectionLead
              title={t("Workspace inventory")}
              description={t("Known workspace roots available to the daemon.")}
            />
            <div className="mt-4">
              {workspaceList.length > 0 ? (
                workspaceList.map((workspace) => (
                  <div key={workspace} className="ui-ledger-card px-4 py-4">
                    <div className="ui-rule-list">
                      <div className="ui-rule-row">
                        <p className="ui-rule-key">{t("Root")}</p>
                        <p
                          className="ui-rule-value break-all font-mono"
                          style={{ color: "var(--fg-soft)" }}
                        >
                          {workspace}
                        </p>
                      </div>
                    </div>
                  </div>
                ))
              ) : (
                <div className="ui-ledger-card px-4 py-4">
                  <p className="text-sm" style={{ color: "var(--fg-muted)" }}>
                    {t("No workspaces discovered")}
                  </p>
                </div>
              )}
            </div>
          </Panel>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function TenantSwitchPanel({
  tenantRows,
  activeTenant,
}: {
  tenantRows: TenantRegistryRow[];
  activeTenant: string | null;
}) {
  const { t } = useI18n();
  const [customTenantId, setCustomTenantId] = useState("");
  const switchMutation = useSwitchTenant();

  return (
    <Panel variant="stage" className="px-5 py-5">
      <SectionLead
        title={t("Tenant context")}
        description={t("Switch tenant to scope sessions and runtime memory.")}
      />

      <div className="mt-4 space-y-3">
        {tenantRows.length > 0 ? (
          <div>
            <p className="ui-field-label">{t("Switch to known tenant")}</p>
            <div className="space-y-1">
              {tenantRows.map((tenant) => (
                <button
                  key={tenant.tenant_id}
                  type="button"
                  onClick={() => switchMutation.mutate(tenant.tenant_id)}
                  disabled={tenant.tenant_id === activeTenant || switchMutation.isPending}
                  data-selected={tenant.tenant_id === activeTenant}
                  aria-label={t("Switch to tenant {id}", { id: tenant.tenant_id })}
                  className="ui-ledger-button ui-ledger-card px-4 py-4"
                >
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-fg break-all text-xs" style={{ lineHeight: 1.6 }}>
                      {tenant.tenant_id}
                    </span>
                    {tenant.tenant_id === activeTenant ? (
                      <StatusBadge variant="ok" label={t("active")} />
                    ) : (
                      <span className="ui-chip">
                        {t("{count} principals", {
                          count: tenant.binding_count,
                        })}
                      </span>
                    )}
                  </div>
                </button>
              ))}
            </div>
          </div>
        ) : null}

        <div>
          <label className="ui-field-label" htmlFor="custom-tenant-id">
            {t("Or enter tenant ID")}
          </label>
          <div className="flex gap-2">
            <input
              id="custom-tenant-id"
              type="text"
              value={customTenantId}
              onChange={(e) => setCustomTenantId(e.target.value)}
              placeholder={t("tenant-id")}
              className="ui-field flex-1"
            />
            <button
              type="button"
              disabled={customTenantId.trim().length === 0 || switchMutation.isPending}
              onClick={() => {
                switchMutation.mutate(customTenantId.trim());
                setCustomTenantId("");
              }}
              className="ui-button ui-button-accent-fill px-4 text-xs font-bold uppercase"
              style={{ paddingTop: "8px", paddingBottom: "8px" }}
            >
              {switchMutation.isPending ? "..." : t("Switch")}
            </button>
          </div>
        </div>

        {switchMutation.isError ? (
          <p className="text-error text-xs">
            {switchMutation.error instanceof Error
              ? switchMutation.error.message
              : t("Tenant switch failed.")}
          </p>
        ) : null}

        {switchMutation.isSuccess ? (
          <p className="text-accent text-xs">{t("Tenant context updated.")}</p>
        ) : null}
      </div>
    </Panel>
  );
}

function TenantRoutingRow({
  tenant,
  index,
  isExpanded,
  isActive,
  onToggle,
}: {
  tenant: TenantRegistryRow;
  index: number;
  isExpanded: boolean;
  isActive: boolean;
  onToggle: () => void;
}) {
  const { t } = useI18n();
  const switchMutation = useSwitchTenant();

  return (
    <div className="ui-ledger-card" data-selected={isExpanded || isActive}>
      <button type="button" onClick={onToggle} className="ui-ledger-button px-0 py-0">
        <div className="ui-ledger-columns px-4 py-4">
          <div className="ui-ledger-cell flex items-center gap-3">
            <span className="ui-chip">{String(index + 1).padStart(2, "0")}</span>
            <span className="text-fg break-all text-sm" style={{ lineHeight: 1.6 }}>
              {tenant.tenant_id}
            </span>
            {isActive ? <StatusBadge variant="ok" label={t("active")} /> : null}
          </div>
          <div className="ui-ledger-cell">
            <p className="app-section-title">{t("Principals")}</p>
            <p className="mt-1 text-sm" style={{ color: "var(--fg-soft)" }}>
              {tenant.binding_count}
            </p>
          </div>
          <div className="ui-ledger-cell">
            <p className="app-section-title">{t("Sample hash")}</p>
            <p
              className="mt-1 break-all font-mono text-xs"
              style={{ color: "var(--fg-muted)", lineHeight: 1.6 }}
            >
              {truncateHash(tenant.principal_hashes[0] ?? "-")}
            </p>
          </div>
          <div className="ui-ledger-cell">
            <p className="app-section-title">{t("Workspace")}</p>
            <p className="mt-1 text-sm" style={{ color: "var(--fg-soft)" }}>
              {tenant.workspace_present ? t("present") : t("missing")}
            </p>
          </div>
          <div className="ui-ledger-cell flex items-start justify-between gap-3 xl:justify-end">
            <span className="ui-chip">{isExpanded ? t("Expanded") : t("Inspect")}</span>
          </div>
        </div>
      </button>

      {isExpanded && (
        <div className="px-4 pb-4 border-t border-[var(--border)] pt-3">
          <div className="flex items-center justify-between gap-3">
            <p className="app-section-title">{t("All principal bindings")}</p>
            {!isActive ? (
              <button
                type="button"
                disabled={switchMutation.isPending}
                onClick={() => switchMutation.mutate(tenant.tenant_id)}
                className="ui-button ui-button-accent-soft px-3 py-1 text-xs font-bold uppercase"
              >
                {switchMutation.isPending ? t("Switching...") : t("Activate tenant")}
              </button>
            ) : (
              <StatusBadge variant="ok" label={t("Currently active")} />
            )}
          </div>

          {switchMutation.isError ? (
            <p className="text-error mt-2 text-xs">
              {switchMutation.error instanceof Error
                ? switchMutation.error.message
                : t("Tenant switch failed.")}
            </p>
          ) : null}

          <div className="ui-rule-list mt-3">
            {tenant.principal_hashes.map((hash, hashIndex) => (
              <div key={hash} className="ui-rule-row">
                <p className="ui-rule-key">{String(hashIndex + 1).padStart(2, "0")}</p>
                <p
                  className="ui-rule-value break-all font-mono"
                  style={{ color: "var(--fg-soft)" }}
                >
                  {hash}
                </p>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function truncateHash(value: string): string {
  if (value.length <= 20) return value;
  return `${value.slice(0, 10)}...${value.slice(-6)}`;
}
