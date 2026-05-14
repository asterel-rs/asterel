---
title: 設定
description: "実用上の設定モデル。設定がどこにあり、どの既定値が重要で、どの調整項目を運用境界として扱うべきか。"
---

Asterel はローカルの TOML ファイルから設定を読み、その一部を環境変数で上書きします。既定のパスは次です。

```text
~/.asterel/config.toml
```

`onboard --interactive` が最初に使える設定を作り、ワークスペースを初期化します。生成されたファイルは運用者のローカル配置記録です。マシン間でそのままコピーするテンプレートとして扱わず、内容をレビューしてください。

## 重要なセクション

多くの運用者が最初に理解すべき領域は五つです。

| 領域 | 制御するもの | 重要な理由 |
|---|---|---|
| Provider | 既定プロバイダー、モデル、API key | ターンループの中で表現と推論を担うモデルを決める |
| Memory | バックエンド、保持、想起、embedding | 継続性がセッションをまたいで残るかを決める |
| Gateway | host、port、pairing、body limit | ローカル / 管理 / API の表面がランタイムへ入る経路を決める |
| Channels | Discord と二次アダプター | どの入力イベントがコンパニオン・ターンになれるかを決める |
| Security | 信頼スコア、意図分類、ツール方針 | 信頼していない入力とツールが何をできるかを決める |

README はコマンド名、ルート一覧、よく使う環境変数を把握するための主要な入口です。網羅的な設定の正本は source schema、`.env.example`、generated contract です。このページは設定をどう考えるかを説明します。

## プロバイダー設定

最低限、ランタイムにはモデルプロバイダーと認証情報が必要です。よく使う環境変数上書きは次です。

| 変数 | 用途 |
|---|---|
| `ASTEREL_API_KEY` | プロバイダー API key |
| `ASTEREL_PROVIDER` | 既定プロバイダー |
| `ASTEREL_MODEL` | 既定モデル |
| `ASTEREL_TEMPERATURE` | サンプリング温度 |

秘密情報と配置固有の値には環境変数を使います。後からレビューすべき持続的な運用者判断は TOML に残す方が読みやすいです。

## メモリ設定

既定のメモリバックエンドは PostgreSQL です。Markdown と `none` は制約のある環境やオフライン設定用にありますが、本番推奨ではありません。

```toml
[memory]
backend = "postgres"
# 実運用で認証情報を含む場合は ASTEREL_POSTGRES_URL を優先してください。
postgres_url = "postgres://asterel@localhost/asterel"
auto_save = true
hygiene_enabled = true
conversation_retention_days = 30
```

データベース URL にパスワードが含まれる場合は、コピーされる設定ファイルへ書き込むより環境変数に置いてください。

重要な既定値:

- `backend = "postgres"` が既定。
- `auto_save = true` なので、無効化しない限り会話文脈は書き込まれる。
- `hygiene_enabled = true` なので、アーカイブと保持期間の整理が動ける。
- `working_memory_capacity = 50` でセッション作業集合を制限する。
- `recall_min_confidence = 0.3` で低信頼度の想起がプロンプト文脈に入る前に落ちる。

記憶を無効化したり使い捨てバックエンドに向けたりしても、ランタイムはターンに答えられます。ただしコンパニオンとしての約束は弱くなります。それは有用なテストモードであり、プロダクト姿勢ではありません。

## ゲートウェイ設定

ゲートウェイの既定値はローカル優先です。

```toml
[gateway]
host = "127.0.0.1"
port = 3000
require_pairing = true
allow_public_bind = false
defense_mode = "enforce"
max_body_size_bytes = 65536
```

通常運用では `require_pairing = true` を維持してください。公開アドレスへの bind は配置判断であり、初期設定の近道ではありません。マシン外からゲートウェイに到達させる必要がある場合は、信頼済みエッジまたは tunnel を前段に置き、信頼モデルを保ってください。

## チャネル設定

Discord は主なプロダクト表面です。Discord アダプターは `channels_config.discord` で設定し、bot token、任意の application / guild 制限、allowed users、thinking embed 設定、拾い上げ方針を持ちます。

既定の拾い上げ方針は控えめです。

```toml
[channels_config.discord.pickup_policy]
mode = "direct_only"
max_unsummoned_replies_per_hour = 0
min_gap_seconds = 600
```

これは運用者が周辺雑談へのまばらな反応を明示的に有効化しない限り、コンパニオンが公開チャネルの会話に入らないことを意味します。これは spam 防止だけでなく、キャラクターと境界の判断です。

## セキュリティ姿勢

セキュリティ設定は層になっています。チャネル単位の自律性とツール許可リストはチャネルができることを狭められますが、全体のセキュリティ制御は常に適用されます。

外部知識と入力は信頼スコア化されます。組み込みプロファイルは Discord、webhook、A2A、ブラウザ / ツール由来コンテンツなどに別々の既定値を与えます。運用者の上書きは狭く、ソース固有にしてください。

```toml
[security.external_knowledge_trust]
enabled = true
default_score = 0.60
min_allow_score = 0.70
min_sanitize_score = 0.30
```

連携を「動かす」ためだけに信頼スコアを上げないでください。そのシグナルを生成したエッジも信頼できる場合に限ります。

## 設定チェックリスト

ローカルランタイムを有用と見なす前に確認すること:

- `onboard --interactive` が完了している。
- プロバイダーとモデルが設定されている。
- 持続する関係継続性を期待するなら PostgreSQL が設定されている。
- 主なプロダクト表面を期待するなら Discord が設定されている。
- ゲートウェイのペアリングが有効なまま。
- 公開入力は信頼済みエッジの後ろにあるか、無効化されている。
- チャネル単位の自律性を上げる場合は明示的な理由がある。
