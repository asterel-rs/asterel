---
title: Research references
description: Public, distilled bibliography for concepts referenced from Asterel source and research claims.
---

This is the public reference index for academic and technical work cited by
Asterel source comments and research claims. It is intentionally distilled from
private design notes: it preserves reusable citations without publishing raw
review logs, work packets, or release-risk notes.

Roles:

- **Foundation** — theory or prior work that shapes the design vocabulary.
- **Implementation** — technique or architecture pattern used in code.
- **Evaluation** — benchmark, rubric, or failure mode that should shape testing.
- **Related work** — adjacent systems or surveys used for positioning.

## Companion and relationship framing

| ID | Role | Citation | Used by |
|---|---|---|---|
| GENERATIVE-AGENTS | Foundation | Park, J. S. et al. (2023). *Generative Agents: Interactive Simulacra of Human Behavior*. UIST 2023. | Persistent memory, reflection, and social behavior framing |
| RELATIONAL-AGENTS | Foundation | Bickmore, T. W. & Picard, R. W. (2005). Establishing and Maintaining Long-Term Human-Computer Relationships. *ACM TOCHI*, 12(2), 293-327. DOI: 10.1145/1067860.1067867. | Companion relationship continuity |
| CASA | Foundation | Nass, C., Steuer, J., & Tauber, E. R. (1994). Computers Are Social Actors. *CHI '94*, 72-78. DOI: 10.1145/191666.191703. | Human social response boundaries |
| XIAOICE | Related work | Zhou, L., Gao, J., Li, D., & Shum, H.-Y. (2020). The Design and Implementation of XiaoIce, an Empathetic Social Chatbot. *Computational Linguistics*, 46(1), 53-93. DOI: 10.1162/coli_a_00368. | Social-chatbot comparison and engagement caution |
| CHATBOT-RELATIONSHIPS | Evaluation | Skjuve, M., Folstad, A., Fostervold, K. I., & Brandtzaeg, P. B. (2022). A Longitudinal Study of Human-Chatbot Relationships. *IJHCS*, 168, 102903. DOI: 10.1016/j.ijhcs.2022.102903. | Longitudinal relationship risk framing |
| COMPANION-RISKS-NMI | Related work | Nature Machine Intelligence Editorial. (2025). Emotional Risks of AI Companions Demand Attention. *Nature Machine Intelligence*, 7. | Public/private distance and safety policy |
| COMPANION-RELATIONSHIPS-RISK | Related work | Malfacini, F. (2025). The Impacts of Companion AI on Human Relationships: Risks, Benefits, and Design Considerations. *AI & Society*. DOI: 10.1007/s00146-025-02318-6. | Non-substitution and safeguard framing |
| AI-HUMAN-CONNECTION | Related work | Smith, M. G., Bradbury, T. N., & Karney, B. R. (2025). Can Generative AI Chatbots Emulate Human Connection? A Relationship Science Perspective. *Perspectives on Psychological Science*. | Companion scope and non-claim boundaries |

## Memory, retrieval, and knowledge graphs

| ID | Role | Citation | Used by |
|---|---|---|---|
| GRAPHRAG | Implementation | Edge, D. et al. (2024). *From Local to Global: A Graph RAG Approach to Query-Focused Summarization*. arXiv:2404.16130. | `src/core/memory/graphrag/` |
| GRAPHRAG-SURVEY-PENG | Related work | Peng, B. et al. (2024). *Graph Retrieval-Augmented Generation: A Survey*. ACM Computing Surveys. | GraphRAG taxonomy |
| HIPPORAG-2 | Implementation | Jiménez Gutiérrez, B. et al. (2025). *From RAG to Memory: Non-Parametric Continual Learning for Large Language Models*. ICML 2025. | Associative/provenance graph retrieval |
| DPR | Implementation | Karpukhin, V. et al. (2020). Dense Passage Retrieval for Open-Domain Question Answering. EMNLP 2020. | `src/core/memory/embeddings/`, vector retrieval |
| RRF | Implementation | Cormack, G. V. et al. (2009). Reciprocal Rank Fusion. SIGIR 2009. | `src/core/memory/reranking.rs` |
| FELLEGI-SUNTER | Foundation | Fellegi, I. P. & Sunter, A. B. (1969). A Theory for Record Linkage. | Entity resolution |
| ER-SURVEY | Related work | Christophides, V. et al. (2020). End-to-End Entity Resolution for Big Data. | Entity-resolution pipeline design |
| DL-HANDBOOK | Foundation | Baader, F. et al. (2007). *The Description Logic Handbook*. | Ontology and schema constraints |
| OWL | Foundation | Horrocks, I. et al. (2003). From SHIQ and RDF to OWL. | Ontology trade-offs |
| REBEL | Implementation | Cabot, P.-L. H. & Navigli, R. (2021). REBEL: Relation Extraction By End-to-end Language Generation. | Relation extraction |
| TEXT2KGBENCH | Evaluation | Mihindukulasooriya, N. et al. (2023). Text2KGBench. | Ontology-constrained KG extraction evaluation |
| SNODGRASS | Foundation | Snodgrass, R. T. (1999). *Developing Time-Oriented Database Applications in SQL*. | Bitemporal graph/memory validity |
| BITEMPORAL-SEMANTICS | Foundation | Jensen, C. S. & Snodgrass, R. T. (1996). Semantics of Time-Varying Information. | Temporal semantics |
| MEMORY-SLEEP | Foundation | Diekelmann, S. & Born, J. (2010). The memory function of sleep. | Memory hygiene and consolidation |
| MEM0-PAPER | Related work | Chhikara, P. et al. (2025). *Mem0: Building Production-Ready AI Agents with Scalable Long-Term Memory*. | Memory lifecycle comparison |
| LONGMEMEVAL | Evaluation | Wu, D. et al. (2025). *LongMemEval: Benchmarking Chat Assistants on Long-Term Interactive Memory*. ICLR 2025. | Long-term chat memory evaluation target |
| PERSONAMEM | Evaluation | Jiang, B. et al. (2025). *Know Me, Respond to Me*. COLM 2025. | Dynamic user profiling evaluation target |
| MEMORYCD | Evaluation | Zhang, W. et al. (2026). *MemoryCD*. ICLR workshop. | Cross-domain lifelong personalization evaluation target |

## Persona, affect, and response quality

| ID | Role | Citation | Used by |
|---|---|---|---|
| BIG-FIVE | Foundation | Costa, P. T. & McCrae, R. R. (1992). *Revised NEO Personality Inventory*. | `src/core/persona/big_five.rs` |
| LLM-PERSONALITY | Evaluation | Serapio-García, G. et al. (2023). Personality Traits in Large Language Models. | LLM personality measurement caveats |
| PERSONALLM | Evaluation | Jiang, H. et al. (2023). PersonaLLM. | Trait expression evaluation |
| OCC | Foundation | Ortony, A., Clore, G. L., & Collins, A. (1988). *The Cognitive Structure of Emotions*. | Affect ontology |
| OCC-FORMAL | Implementation | Steunebrink, B. R. et al. (2008). Formalizing OCC appraisal. | Computational appraisal structure |
| AFFECTIVE-COMPUTING | Foundation | Picard, R. W. (1997). *Affective Computing*. MIT Press. | Affect detection and interpretation |
| PAD | Foundation | Mehrabian, A. (1996). Pleasure-arousal-dominance affect representation. | Mood representation |
| ALMA | Related work | Gebhard, P. (2005). ALMA: a layered model of affect. | Emotion/mood/personality layering |
| CULEMO | Evaluation | Belay, T. D. et al. (2025). *CULEMO: Cultural Lenses on Emotion*. ACL 2025. DOI: 10.18653/v1/2025.acl-long.925. | Cross-cultural emotion evaluation target |
| TRACE | Evaluation | Jeon, E. et al. (2025). "Going to a trap house" conveys more fear than "Going to a mall": Benchmarking Emotion Context Sensitivity for LLMs. *Findings of EMNLP 2025*, 14848-14869. DOI: 10.18653/v1/2025.findings-emnlp.802. | Affect-context evaluation target |
| HEART | Evaluation | Iyer, L. et al. (2026). *HEART: A Unified Benchmark for Assessing Humans and LLMs in Emotional Support Dialogue*. arXiv:2601.19922. | Companion response quality target |
| CONSTITUTIONAL-AI | Implementation | Bai, Y. et al. (2022). Constitutional AI. | Critique/revision and policy framing |

## Security and agent governance

| ID | Role | Citation | Used by |
|---|---|---|---|
| TAINT-LATTICE | Foundation | Denning, D. E. (1976). A Lattice Model of Secure Information Flow. | `src/security/taint/label.rs` |
| WASP | Evaluation | Evtimov, I. et al. (2025). *WASP: Benchmarking Web Agent Security Against Prompt Injection Attacks*. ICML 2025. arXiv:2504.18575. | External-content/tool security evaluation target |
| A2ASECBENCH | Evaluation | Li, T. et al. (2026). *A2ASecBench: A Protocol-Aware Security Benchmark for Agent-to-Agent Multi-Agent Systems*. ICLR 2026. | A2A/gateway security evaluation target |
| AGENT-SECURITY-SURVEY | Related work | Ferrag, M. A. et al. (2025). From Prompt Injections to Protocol Exploits: Threats in LLM-Powered AI Agents Workflows. *ICT Express*. DOI: 10.1016/j.icte.2025.12.001. | Threat taxonomy and public security framing |
| SOTOPIA | Evaluation | Zhou, X., Zhu, H., Mathur, L., et al. (2023). *SOTOPIA: Interactive Evaluation for Social Intelligence in Language Agents*. | Social intelligence and private-goal evaluation target |

## 9. Cross-Reference Index

Current public mappings are limited to modules that still exist in the repository.

| Reference area | Current code |
|---|---|
| Companion turn contract | `src/runtime/services/companion_turn.rs`, `tests/runtime/companion_turn_contract.rs` |
| Affect and appraisal | `src/core/affect/` |
| Memory retrieval and GraphRAG | `src/core/memory/` |
| Persona and relationship modeling | `src/core/persona/` |
| Response verification and naturalness | `src/core/agent/response_finalize/`, `src/core/agent/naturalness_gate/` |
| Taint and policy enforcement | `src/security/` |
| Gateway/channel containment | `src/transport/gateway/`, `src/transport/channels/` |
