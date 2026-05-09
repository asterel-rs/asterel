---
title: Memory review
description: How operators should think about reviewing, correcting, forgetting, and approving companion memory.
---

Memory review is where Asterel's companion promise becomes inspectable. The goal
is not to expose every raw event by default. The goal is to let an operator see
why a remembered fact exists, correct it when wrong, and prevent private context
from leaking into the wrong surface.

## Review model

Memory should be read in layers:

```text
compact view -> timeline / provenance -> full evidence recovery
```

The compact view is enough for most live operation. Provenance and timeline help
answer "why does the companion believe this?" Full evidence recovery is for
review, correction, deletion, or incident analysis.

## Correct, forget, delete

Use different operations for different intent:

| Intent | Operator meaning |
|---|---|
| Correct | The old fact was wrong or stale; keep lineage and make the current view right |
| Forget | The fact should not be used again, subject to backend and policy semantics |
| Delete / hard removal | Stronger removal path when supported and policy permits it |

Do not silently overwrite facts that shaped prior behavior. Correction lineage is
part of the trust story.

## Public/private exposure

The companion may use private memory as latent grounding in a private context, but
that does not mean it may say the fact in a public channel. Exposure review should
ask:

- where did this memory originate?
- is the current turn public, thread, DM, or gateway?
- was the fact corrected or marked sensitive?
- did response finalization block or revise the draft?

## Self-amendment review

Sometimes the useful memory is not a fact about the user. It is a lesson about
how the companion should approach this person or surface next time. Those lessons
are self-amendment candidates.

They should stay governed:

- generated as reviewable candidates;
- bounded and redacted rather than copied raw transcripts;
- approved by an operator before durable persistence;
- stored as private procedural memory, not as character-core mutation.

Do not turn a user correction into an automatic intimacy upgrade. Repair should
improve future behavior while preserving the user's freedom to correct, reset,
leave, or ask for forgetting.
