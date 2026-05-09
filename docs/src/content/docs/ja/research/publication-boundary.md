---
title: Publication boundary
description: internal material のうち何を public research evidence に蒸留でき、何を private に残すべきか。
---

public research packet は、paper を支えられるだけの強さを持つべきです。ただし、raw internal operations を公開してはいけません。ルールは単純です。claims、methods、citations、synthetic fixtures、reproducible results は公開する。private logs、unresolved security notes、personal / operator context は repository から外す。

## Public after distillation

- citation metadata と module / concept mapping を持つ bibliography entries
- 反証可能な public statements として書き直した design claims
- verification commands と aggregate pass / fail results
- synthetic replay fixtures と test names
- public source から生成された architecture diagrams
- redacted evaluation summaries と failure taxonomies

## Private by default

- Raw semantic-review findings と unresolved vulnerability backlog
- secret fingerprints、provider account details、operational rotation history を含む incident notes
- real user transcripts、private memory payloads、relationship state、tenant identifiers
- agent handoff prompts、local session notes、personal workspace paths
- private grounding context を含む provider responses または prompts

## Distillation pattern

| Internal material | Public form |
|---|---|
| Work packet | Implemented claim + source owner + verification command |
| Review finding | Fixed invariant + regression test, without exploit narrative |
| Incident note | General hardening lesson + current security control |
| Implementation log | Evidence ledger entry with command and result |
| Reference index | Bibliography entry with role and module / concept mapping |

この boundary は research method の一部です。privacy-aware governance を主張する companion runtime が、証拠として private evidence を公開することはできません。
