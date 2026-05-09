---
title: 始め方
description: clone から動くコンパニオンまでの最短導線。完全なリファレンスは repository README が正本です。
---

このページは、ローカルで有用なところまで動かす最短の導線です。細かなコマンド、ルート、設定キーの完全なリファレンスは [repository README](https://github.com/asterel-rs/asterel/blob/main/README.md) にあります。

## 前提条件

- Rust stable — `rust-toolchain.toml` で固定
- `protoc` v29 以上
- Git
- onboarding で設定するモデルプロバイダー認証情報、またはローカルプロバイダー
- 推奨メモリバックエンドを使う場合は PostgreSQL。Markdown フォールバックは制約のある環境でのテストには使えますが、関係の継続性を検証するプロダクト姿勢は PostgreSQL です。

## ビルド

```bash
git clone https://github.com/asterel-rs/asterel.git
cd asterel
cargo build --release
```

## 最初の実行

最初の実行には順序があります。新規インストールでは `agent` を開始する前に `onboard --interactive` を完了する必要があります。これは `~/.asterel/config.toml` を書き、ワークスペースを初期化します。

```bash
# 対話式オンボーディング（新規インストールでは最初に実行）
cargo run -- onboard --interactive

# 対話式エージェントループを開始
cargo run -- agent

# 一回だけメッセージを送る
cargo run -- agent --message "Summarize my open tasks"
```

`agent` は、ローカルのプロバイダーと設定が使えることを確認するために便利です。プロダクトとしての形を確認する場合は daemon を使います。

## フルデーモンを起動する

Discord、ゲートウェイ、スケジューラー、heartbeat をまとめて動かす場合:

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
```

これは [ターンパイプライン](../../architecture/turn-pipeline/) が想定しているモードです。Discord テキストは主なプロダクト表面です。onboarding が書いた設定と README のチャネル設定に従って、デーモンへ接続します。Discord メッセージがコンパニオン・ターンとして受理されると、共有トランスポート経路と、他の受理済みコンパニオン・ターンと同じ補強 / 検証契約を使います。

Discord を接続する前に、ローカルランタイムを確認します。

```bash
cargo run -- config validate
cargo run -- doctor
cargo run -- status
```

その後、主チャネルの設定は [Discord を動かす](../discord-setup/) に進んでください。

## 次に読むもの

- **主な表面をつなぐ** — [Discord を動かす](../discord-setup/)。
- **デーモンを運用する** — [ローカル運用](../operating-locally/) と [ゲートウェイ](../../operator/gateway/)。
- **記憶とガバナンスを見る** — [記憶レビュー](../../operator/memory-review/) と [セキュリティとガバナンス](../../architecture/security-governance/)。
- **設計を理解する** — [概要](../../overview/) と [コンパニオン・ランタイム](../../concepts/companion/)。
- **詰まったとき** — [トラブルシュート](../troubleshooting/)。
