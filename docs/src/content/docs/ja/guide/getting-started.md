---
title: 始め方
description: インストールから動くコンパニオンまでの最短導線。完全なリファレンスは repository README が正本です。
---

このページは、ローカルで有用なところまで動かす最短の導線です。細かなコマンド、ルート、設定キーの完全なリファレンスは [repository README](https://github.com/asterel-rs/asterel/blob/main/README.md) にあります。

## インストール（macOS/Linux）

```bash
curl -fsSL https://asterel-rs.github.io/asterel/install.sh | sh
asterel onboard --interactive
```

インストーラーは、利用できる場合は GitHub Releases のバイナリを使い、なければソースからビルドします。既定のインストール先は `~/.local/bin` です。シェルがまだ `asterel` を見つけられない場合は、`~/.local/bin/asterel onboard --interactive` を実行するか、`~/.local/bin` を `PATH` に追加してください。

## 最初の実行

```bash
asterel agent
asterel agent --message "Summarize my open tasks"
```

`agent` は、ローカルのプロバイダーと設定が使えることを確認するのに便利です。プロダクトに近い形で動かす場合は daemon を使います。

## ソースからビルドする場合

```bash
git clone https://github.com/asterel-rs/asterel.git
cd asterel
cargo build --release
cargo run -- onboard --interactive
cargo run -- agent
```

ソースビルドには Rust stable、`protoc` v29 以上、Git、onboarding で設定するモデルプロバイダー認証情報またはローカルプロバイダーが必要です。推奨メモリバックエンドは PostgreSQL です。制約のあるローカルテストでは Markdown や `none` も使えます。

## フルデーモンを起動する

Discord、ゲートウェイ、スケジューラー、heartbeat をまとめて動かす場合:

```bash
asterel daemon --host 127.0.0.1 --port 3000
```

これは [ターンパイプライン](../../architecture/turn-pipeline/) が想定しているモードです。Discord テキストは主なプロダクト表面です。onboarding が書いた設定と README のチャネル設定に従って、デーモンへ接続します。Discord メッセージがコンパニオン・ターンとして受理されると、共有トランスポート経路と、他の受理済みコンパニオン・ターンと同じ補強 / 検証契約を使います。

Discord を接続する前に、ローカルランタイムを確認します。

```bash
asterel config validate
asterel doctor
asterel status
```

その後、主チャネルの設定は [Discord を動かす](../discord-setup/) に進んでください。

## 次に読むもの

- **主な表面をつなぐ** — [Discord を動かす](../discord-setup/)。
- **デーモンを運用する** — [ローカル運用](../operating-locally/) と [ゲートウェイ](../../operator/gateway/)。
- **記憶とガバナンスを見る** — [記憶レビュー](../../operator/memory-review/) と [セキュリティとガバナンス](../../architecture/security-governance/)。
- **設計を理解する** — [概要](../../overview/) と [コンパニオン・ランタイム](../../concepts/companion/)。
- **詰まったとき** — [トラブルシュート](../troubleshooting/)。
