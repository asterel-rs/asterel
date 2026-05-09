---
title: 依存レイヤー
description: 継続性を担うモジュールがトランスポートに偶然依存しないように `src/` をどう整理し、なぜレイヤリングを提案ではなく強制にしているか。
---

`src/` は六つのレイヤー (L0–L5) に分かれています。依存は上向きだけです。高いレイヤーは低いレイヤーに依存できますが、低いレイヤーが高いレイヤーを import することはできません。違反はレビューとアーキテクチャチェックで捕まえます。

| レイヤー | モジュール | 役割 |
|---|---|---|
| L0 | `contracts/`, `config/`, `utils/` | 境界をまたぐ型、ID、TOML スキーマ |
| L1 | `core/memory/`, `core/persona/`, `core/providers/`, `core/sessions/`, `core/subagents/`, `core/experience/`, `core/eval/` | 持続状態、同一性、プロバイダー抽象、共有ランタイム領域 |
| L2 | `core/tools/`, `security/` | ツールシステム、承認、方針、汚染追跡、ガバナンス |
| L3 | `core/agent/`, `core/affect/`, `media/` | ターン実行、感情検出、マルチモーダル処理 |
| L4 | composition-facing な `runtime/services/`, `runtime/diagnostics/`, `runtime/observability/` | 合成ルート、依存注入、テレメトリ |
| L5 | `transport/`, `cli/`, `platform/`, `plugins/`, `ui/`, `onboard/` | ゲートウェイ、チャネル、CLI、デスクトッププラグイン |

これは外部向けの地図です。内部の正本アーキテクチャはもう少し精密で、`runtime/services/` の一部は共有ターンや control-plane の振る舞いを持つ application service、一部は provider、memory、session、surface を結線する composition service です。外部読者向けの規則は単純です。継続性を担う状態はトランスポートより下に置き、surface 固有コードを正本にしません。

## レイヤリングが守るもの

レイヤリングが守る性質は一つです。**継続性の状態 (L1) は、それを読んでいる表面を知らない。**

- 記憶、人格、セッションは `transport/` や `cli/` を import しません。Discord メッセージの形や HTTP リクエストの形に偶然結びつきません。
- 中核のエージェントロジック (L3) は、セキュリティ / ツールレイヤー (L2) を飛び越えてチャネルと直接話しません。
- トランスポート (L5) は、新しいチャネル、デスクトップパネル、新しいゲートウェイルートなどに変わっても、それが表示する状態を触らずに変えられます。

この境界が穴だらけだと、ランタイムは一回のリファクタで Discord 型の記憶やゲートウェイ風味の人格に近づきます。レイヤリングは [共有ターンパイプライン](../turn-pipeline/) を誠実に保つための仕組みです。

## 合成ルート

すべては `src/runtime/services/` で結線されます。このモジュールは次を行います。

- 認証、セキュリティ方針、記憶、レートリミッターを初期化する
- プロバイダーを構築する。プロバイダーは信頼性レイヤーと OAuth 復旧レイヤーで包まれる
- ツール登録を組み立てる
- すべてのツール呼び出しに渡す `ExecutionContext` を構築する

合成ルートは、すべての部品を同時に知る唯一の場所です。他のモジュールは狭い断面だけを見ます。

## 主要な trait 境界

モジュール境界で特に重要なのは三つの trait です。

- **`Memory`** — `MemoryWriter` (`append_event`)、`MemoryReader` (`recall_scoped`, `resolve_slot`)、`MemoryGovernance` (`health_check`, `forget_slot`) を合わせた supertrait。バックエンドは PostgreSQL（既定）または Markdown 代替。
- **`Tool`** — `name()`、`description()`、`parameters_schema()`、`execute(args, ctx)`。`core/tools/registry.rs` に登録される。
- **`Provider`** — 必須メソッドは `chat_with_system()`。`ReliableProvider`（retry + circuit-breaker）と `OAuthRecoveryProvider` で包まれる。実装には Anthropic、OpenAI、OpenRouter、Ollama、Gemini / Gemini Vertex、MiniMax が含まれます。

この三つはレイヤーをまたぐ通信の主要契約です。変更を追っていてここにたどり着いたなら、正しい境界を見ています。

## 実験的なコード

実験的または退役済みのコードは、アクティブなモジュールツリーから export され、アーキテクチャチェックで守られていない限り、本番のレイヤリング契約の一部ではありません。現在のコンパニオン・ランタイムは上のレイヤーが所有します。古い planner、simulation、evolution の表面はプロダクトを担う入口ではありません。
