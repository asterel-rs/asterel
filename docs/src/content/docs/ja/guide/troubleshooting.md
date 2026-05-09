---
title: トラブルシュート
description: ローカル運用でよくある失敗と、設定を広げる前に確認する最初のポイント。
---

Asterel の失敗の多くは境界の問題です。設定が読み込まれていない。チャネルがメッセージを受理していない。プロバイダーが答えられない。記憶が使えない。ゲートウェイが pair されていない。まず、その境界を切り分ける小さな確認から始めます。

## すばやい health pass

振る舞いの flag を変える前に、次を実行します。

```bash
cargo run -- config validate
cargo run -- doctor
cargo run -- status
cargo run -- channel list
```

docs や source をローカルで変更した場合は、対応する gate も実行します。

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo test
pnpm --dir docs build
```

## daemon は起動するが Discord が返信しない

次の順で確認してください。

1. **チャネルが有効** — `cargo run -- channel list` で Discord が設定済みとして出る。
2. **token とサーバーアクセス** — bot が対象サーバーまたは DM に存在する。
3. **スコープフィルタ** — `guild_id` と `allowed_users` がメッセージを除外していない。
4. **拾い上げ方針** — `direct_only` では公開部屋の周辺雑談は無視される。
5. **プロバイダー** — 設定されたモデルプロバイダーが基本的な agent turn を完了できる。
6. **検証器** — 応答の最終化が、危険または漏えいのある draft を止めている可能性がある。

bot が生きていることを確認するためだけに、広い ambient pickup へ切り替えないでください。まず direct mention か DM を使います。

## Gateway pairing が失敗する

ゲートウェイはローカル優先で、既定では pairing が必要です。

- daemon または gateway が想定した host / port で listen しているか確認する。
- ローカル運用では、信頼済みエッジを前段に置いた場合を除き `127.0.0.1` を使う。
- `/admin/v1/*` route を呼ぶ前に pair する。
- admin call では、返された bearer token と明示的な tenant header を送る。

```text
Authorization: Bearer <token>
X-Asterel-Tenant: <tenant-id>
```

tenant scope はローカル運用者の文脈です。公開 SaaS の隔離モデルではありません。

## PostgreSQL が使えない

PostgreSQL は推奨メモリバックエンドです。使えない場合は次を確認します。

- `ASTEREL_POSTGRES_URL` または設定済みの `postgres_url` が正しい。
- daemon process から database に到達できる。
- Markdown フォールバックは、プロダクト証拠が弱くなることを受け入れる場合だけ使う。
- `backend = "none"` は現在、真の stateless store ではなく Markdown 互換 fallback に流れる。

目的が public release や paper artifact なら、どの backend を使ったか記録してください。

## Provider key がない、または間違っている

Provider error は、たいてい次のどれかが欠けているか、噛み合っていないことを示します。

- `ASTEREL_API_KEY` または provider 固有の認証情報
- `ASTEREL_PROVIDER` と `ASTEREL_MODEL`
- ローカル設定で選ばれた auth profile
- compatible provider 用の provider base URL

プロバイダーの秘密情報は、環境変数またはランタイムの秘密情報経路に置いてください。issue report に key を貼らないでください。

## 応答が長すぎる、前のめりすぎる、私的すぎる

これは prompt だけの問題ではなく、コンパニオン品質の問題として扱います。

- そのターンが公開、スレッド、DM のどれだったか確認する。
- 拾い上げ方針と公開 / 私的な露出姿勢を確認する。
- 記憶想起が私的な事実を公開文脈へ出していないか確認する。
- 既定経路では応答の最終化を有効に保つ。
- 覚えている事実が間違っている場合は、記憶レビュー / 訂正を使う。

問題に私的な記憶や実ユーザーの transcript が含まれる可能性がある場合、raw content を公開しないでください。redact するか、security / private reporting path を使います。
