---
title: Publication boundary
description: What internal material can be distilled into public research evidence and what must remain private.
---

The public research packet should be strong enough to support a paper, but it
must not publish raw internal operations. The rule is: publish claims, methods,
citations, synthetic fixtures, and reproducible results; keep private logs,
unresolved security notes, and personal/operator context out of the repo.

## Public after distillation

- Bibliography entries with citation metadata and module/concept mapping.
- Design claims rewritten as falsifiable public statements.
- Verification commands and aggregate pass/fail results.
- Synthetic replay fixtures and test names.
- Architecture diagrams generated from public source.
- Redacted evaluation summaries and failure taxonomies.

## Private by default

- Raw semantic-review findings and unresolved vulnerability backlog.
- Incident notes containing secret fingerprints, provider account details, or
  operational rotation history.
- Real user transcripts, private memory payloads, relationship state, and tenant
  identifiers.
- Agent handoff prompts, local session notes, and personal workspace paths.
- Provider responses or prompts that include private grounding context.

## Distillation pattern

| Internal material | Public form |
|---|---|
| Work packet | Implemented claim + source owner + verification command |
| Review finding | Fixed invariant + regression test, without exploit narrative |
| Incident note | General hardening lesson + current security control |
| Implementation log | Evidence ledger entry with command and result |
| Reference index | Bibliography entry with role and module/concept mapping |

This boundary is part of the research method: a companion runtime cannot claim
privacy-aware governance while publishing private evidence as proof.
