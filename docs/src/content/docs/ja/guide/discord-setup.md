---
title: Discord を動かす
description: 現在の主な Asterel プロダクト表面である、ローカルデーモン経由の Discord テキスト接続方法。
---

Discord テキストは、現在のプロダクト検証の主表面です。他のアダプターも compile され読み込まれることがありますが、Asterel が最初に完全なコンパニオン・ループを検証している場所は Discord です。拾い上げ方針、記憶に支えられた継続性、公開 / 私的な距離、応答の最終化、ターン後の書き戻しがここでつながります。

## 始める前に

必要なもの:

- `cargo run -- onboard --interactive` による onboarding 済みの設定
- 設定済みのモデルプロバイダー、またはローカルモデル
- ローカルで動いている daemon
- Discord bot token
- bot が応答を許可されているサーバーまたは DM 文脈

関係の継続性を永続させる場合、推奨メモリバックエンドは PostgreSQL です。Markdown フォールバックは制約のあるテストには便利ですが、完全なプロダクト姿勢ではありません。

## 1. Discord 設定を追加する

Discord 設定は `~/.asterel/config.toml` の `channels_config.discord` にあります。コピーされる例に秘密情報を入れず、可能なら環境変数や秘密情報管理経路を使ってください。

```toml
[channels_config.discord]
bot_token = "DISCORD_BOT_TOKEN"
# 任意: 一つのサーバーに制限する。
guild_id = "DISCORD_GUILD_ID"
# 任意: bot と話せるユーザーを制限する。
allowed_users = ["DISCORD_USER_ID"]
thinking_embed = true

[channels_config.discord.pickup_policy]
mode = "direct_only"
max_unsummoned_replies_per_hour = 0
min_gap_seconds = 600
```

既定の拾い上げ姿勢は意図的に静かです。まず direct mention と DM から始めてください。公開部屋での振る舞いに運用者が納得してから、周辺雑談へのまばらな反応を有効化します。

## 2. ローカル設定を検証する

```bash
cargo run -- config validate
cargo run -- doctor
cargo run -- channel list
```

これらのコマンドは、Discord traffic が入る前に多くの設定問題を見つけます。プロバイダー認証情報の欠落、メモリ設定の欠落、不正な TOML、無効化されたチャネルなどです。

## 3. daemon を起動する

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
```

daemon は通常のプロダクト形です。ゲートウェイルート、チャネル、スケジューラー、heartbeat、記憶、共有コンパニオン・ターンのランタイムを、一つのランタイムインスタンスの周りで動かします。

## 4. direct turn を試す

Discord で bot に mention するか、DM を送ります。受理されると、メッセージは共有コンパニオン・ターン経路を通ります。

```text
Discord event -> pickup policy -> turn enrichment -> response assembly
  -> response finalization -> reply delivery -> post-turn update
```

返信がない場合、すぐにすべての設定を緩めないでください。次の順で判断点を確認します。

1. bot token が有効である。
2. bot が対象サーバーまたは DM に存在する。
3. `guild_id` と `allowed_users` がメッセージを除外していない。
4. 拾い上げ方針がメッセージを受理している。
5. プロバイダーがターンを完了できる。
6. 応答の最終化が送信を許可している。

## 公開部屋のルール

Asterel は騒がしい room bot のように振る舞うべきではありません。公開部屋での距離はプロダクト上の約束の一部です。周辺雑談へ入るより、direct mention や明確な招待の方が安全です。私的な記憶は、有用な接地情報であっても、それだけで公開部屋に出してよいわけではありません。
