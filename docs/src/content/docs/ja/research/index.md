---
title: Research packet
description: Asterel の公開研究フレーミング。claims、methods、evidence、references、非公開に残すもの。
---

この packet は深く読むための導線です。一般向け docs は Asterel が何で、どう動かすかを説明します。この section は、何を主張できるか、その主張をどの証拠が支えているか、どこに benchmark や human evaluation がまだ必要かを説明します。

private design archive より意図的に狭くしています。raw review logs、private transcripts、operator notes を公開せず、読者が検査できる claims、evidence classes、references だけを残します。

Asterel は、ここで完成した empirical paper として提示されているわけではありません。現在の packet は次を分けます。

- code と regression tests に支えられた **implemented claims**
- reproducible checks と fixture design に支えられた **method claims**
- paper-level conclusion の前に external benchmarks、ablations、human studies が必要な **research gaps**

## Packet structure

- [Claims](./claims/) — 反証可能な design claims と現在の evidence。
- [Methodology](./methodology/) — claims をどう評価し、何を evidence と数えるか。
- [Evidence ledger](./evidence-ledger/) — 公開可能な evidence classes と再現 command。
- [Technical report v0.1](./technical-report-v0-1/) — 現在の runtime design、local gates、harness ablation result をまとめた artifact report。
- [Public release roadmap](./public-release-roadmap/) — private history を持ち込まず repository を公開する clean-snapshot release phases。
- [Benchmark roadmap](./benchmark-roadmap/) — empirical paper claims の前に必要な external / local benchmark tracks。
- [Ablation plan](./ablation-plan/) — memory、affect、exposure、naturalness、security controls の feature-removal studies。
- [Reflective support stance integrity plan](./reflective-support-stance-integrity-plan/) — 個人的な相談で共感を保ったまま迎合・断定・依存助長を避けるための実装計画。
- [Experimental protocol](./experimental-protocol/) — benchmark、ablation、human-rating、artifact publication の clean-snapshot runbook。
- [Reproducibility](./reproducibility/) — public release の environment、commands、snapshot discipline。
- [Publication boundary](./publication-boundary/) — 何を公開でき、何を private に残すべきか。internal material をどう蒸留するか。
- [Research references](../reference/references/) — concepts と modules に mapping された bibliography。

## Current thesis

Asterel は、長く続く companionship を prompting problem ではなく runtime problem として扱います。中心となる thesis は、durable companion behavior には transport-independent continuity state、governed memory writeback、surface-aware exposure control、そして response が relationship history の一部になる前の pre-send verification が必要だ、というものです。

現在の repository は、architecture checks、boundary tests、memory / governance tests、response-verifier fixtures、replay-based quality gates によってこの thesis を支えています。ただし、外部 system に対する統計的に検証済みの優位性は、まだ主張しません。
