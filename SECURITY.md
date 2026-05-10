# Security Policy: Single-Operator Companion Runtime

## Threat Model

Asterel is a **single-operator companion runtime**. One person, or a small
trusted team, runs the daemon on a machine they control. It is designed for a
Discord-first, text-first companion with durable memory and operator governance.

It is **not** a multi-tenant SaaS, a public RAG endpoint, a shared inference
gateway, or an autonomous business-action workbench. The operator is trusted.
The threat model focuses on containing what the LLM, tools, memory pipeline,
channel adapters, and externally reachable surfaces can do on the operator's
behalf.

Security approval paths exist to contain risky actions. They are not the primary
product loop. The primary product loop is companion conversation:

`channel input -> pickup policy -> turn enrichment -> response assembly -> pre-send verification -> reply delivery -> post-turn update`

## Supported Versions

Asterel is in active pre-release development. Security fixes land on the
default branch. Until tagged releases define a support window, only the current
`main` branch is treated as supported.

## Containment Boundaries

### 1. Tool Execution

The LLM may attempt to misuse tools, hallucinate destructive actions, or be
prompted into undesired behavior. Tool execution is contained by:

- **Deny-by-default command allowlist.** Commands must be explicitly permitted by
  policy.
- **Workspace path policy.** File tools are constrained to the workspace by
  default; writeback/path guards reject out-of-scope writes unless explicitly
  allowed.
- **Approval broker.** Risky tool or command actions are routed through
  operator approval before execution. Missing approval capability denies the
  action rather than silently executing it.
- **Fixed middleware chain.** The default tool pipeline is:
  `SecurityMiddleware -> HookMiddleware -> EntityRateLimitMiddleware -> AuditMiddleware -> SemanticCompactionMiddleware -> ToolOutputCompactionMiddleware -> OutputSizeLimitMiddleware -> ToolResultSanitizationMiddleware -> SecretScrubMiddleware -> TaintMiddleware`.

### 2. Memory and Relationship State

Durable memory is part of the product, so memory corruption or privacy leakage is
a security issue. Asterel treats the memory layer as a governed write surface:

- **Writeback guard.** LLM-produced writebacks are validated against immutable
  fields, declared slot contracts, provenance requirements, and privacy rules.
- **Privacy levels.** Memory slots carry `Public`, `Private`, or `Secret`
  privacy levels. Default governance redacts private and secret values from
  inspect/export paths unless sensitive output is explicitly requested.
- **Public/private exposure control.** Public channel responses must not expose
  deep DM-derived or sensitive memory. Exposure diagnostics are available through
  the admin memory surface.
- **Correction and forgetting.** Memory correction and forget tools exist so
  stale or incorrect memory can be amended instead of accumulating silently.

### 3. Externally Reachable Ingress

Webhooks, A2A messages, companion HTTP routes, and tunnel-exposed gateway
surfaces can be reached by untrusted clients if the operator exposes them. The
runtime consumes **edge-verified trust signals** injected by a trusted reverse
proxy or signature verifier:

- `X-Signature-Verified`, `X-Signature-Status`, `X-Webhook-Signature-Status`
- `X-Source-Url`, `X-External-Source-Url`
- `X-Forwarded-Proto`, `Origin`, `Referer`

These headers influence runtime trust scoring **only when injected by a trusted
edge verifier**. Untrusted clients must not be able to set them directly.
Operators are responsible for terminating ingress at a reverse proxy that strips
these headers from untrusted requests and re-injects them only after
verification.

Trust scoring is configured under `[security.external_knowledge_trust]` in
`~/.asterel/config.toml`; see the README Configuration section for an
example.

### 4. Gateway, Admin, and Pairing

The gateway and desktop operator console are governance surfaces, not the
primary companion surface. They can inspect or mutate runtime state and therefore
must be treated as privileged:

- Gateway pairing uses token hashing and lockout handling.
- Admin and companion routes should be bound to localhost or placed behind a
  trusted reverse proxy unless deliberately exposed.
- Tunnel features are optional and materially change the exposure boundary.
  Enabling a tunnel means the operator is responsible for the public edge.

### 5. Local Secret Handling

Provider API keys, OAuth tokens, channel credentials, tunnel tokens, and other
runtime secrets are protected by:

- **Encrypted local secret vault** using ChaCha20-Poly1305 with Argon2id-based
  key derivation for password-backed encryption.
- **Config secret encryption** for supported secret fields when secret
  encryption is enabled.
- **Secret scrubbing** across logs, errors, tool outputs, traces, telemetry, and
  user-facing surfaces before data leaves the process.

### 6. External Content and Attachments

External content can carry prompt-injection attempts or SSRF targets. The runtime
has explicit defenses for these paths:

- External content is normalized and wrapped before being fed back into the
  model context.
- URL fetching rejects private, loopback, file, and unsupported URL forms where
  applicable.
- Tool result sanitization and taint propagation preserve provenance labels
  through the tool pipeline.

## In Scope

- Single-operator workspace running on a machine controlled by the operator
- Discord and other channel adapters consuming inbound messages
- Gateway HTTP/WS ingress, including `/webhook`, `/a2a/v1/*`, `/companion/*`,
  `/admin/v1/*`, and `/pair`
- Tool execution invoked by the LLM or direct CLI commands
- Memory writeback, memory review, correction, forgetting, and exposure
  diagnostics
- Local secret storage, config secret encryption, and outbound secret scrubbing
- SSRF, prompt-injection, replay, or trust-signal bypasses affecting reachable
  runtime surfaces

## Out of Scope

- **Multi-tenant isolation.** Asterel does not isolate multiple untrusted
  operators on the same daemon. Run one daemon per operator.
- **Public RAG / hosted inference.** The runtime is not designed to be exposed
  as a public endpoint that strangers query directly.
- **Human impersonation safety claims.** Asterel must be honest about being
  AI, but this project does not claim to solve general AI deception or social
  engineering outside its configured surfaces.
- **Defending the operator from themselves.** If the operator allowlists a
  destructive command, disables pairing, exposes admin routes publicly, or grants
  approval to a risky action, the runtime will follow that configuration.
- **General dependency triage without exploitability.** Dependency surveillance
  is handled by Dependabot and `cargo audit`. Reports are most useful when they
  show reachability, exploitability, or a blocked update path.

## Reporting a Vulnerability

If you believe you have found a security vulnerability in Asterel:

1. **Do not** open a public GitHub issue.
2. Use GitHub private vulnerability reporting / repository security advisories:
   <https://github.com/asterel-rs/asterel/security/advisories/new>.
   Repository maintainers must enable private vulnerability reporting for public
   external reports to use this path.
3. If the private report form is not available, email **security@asterel.rs**
   with the subject prefix `[security]`. If email bounces, open a public issue
   asking for a private security contact **without** including vulnerability
   details.
4. Include the affected commit or version, configuration, channel/provider
   surface, reproduction steps, expected impact, and any logs or proof of
   concept that help reproduce the issue.

Useful reports include:

- unauthorized gateway/admin access
- command/path policy bypass
- approval bypass for risky tools
- memory exposure from private/secret context into public channel output
- memory writeback poisoning or provenance bypass
- secret leakage in logs, traces, tool output, or messages
- SSRF or unsafe attachment fetch behavior
- replay or spoofing bypass for externally reachable ingress

There is no bug bounty program. Coordinated disclosure is appreciated; please
give maintainers time to ship a fix before publishing details.

## Hardening References

- README §Gateway and §Security — operator-facing configuration summary
- Public docs §Security and governance — containment model for ingress, tools,
  secrets, and admin access
- Public docs §Layered dependencies — source layout and dependency direction
- `CONTRIBUTING.md` — issue/PR expectations and security-reporting reminders
