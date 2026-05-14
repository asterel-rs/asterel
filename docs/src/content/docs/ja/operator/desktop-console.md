---
title: デスクトップコンソール
description: デスクトップアプリの用途。コンパニオン・ランタイムを囲む運用者レビュー、診断、ガバナンス。
---

デスクトップアプリは二次的な運用者コンソールです。ユーザーがコンパニオンと出会う主な場所ではなく、第二の runtime でもありません。admin API を通じて daemon / gateway runtime を読み、管理します。

## ローカルで起動する

まず daemon を起動します。

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
```

次にデスクトップアプリを起動します。

```bash
pnpm --dir desktop tauri dev
```

## 何に使うか

デスクトップコンソールは次に使います。

- session review と transcript inspection
- 記憶レビュー、訂正、忘却、self-amendment approval workflow
- exposure diagnostics と governance check
- runtime、channel、scheduler の health（durable cron state は PostgreSQL backed）
- auth、provider、skill、cron、tenant / operator settings

## 何に使わないか

デスクトップアプリは次になってはいけません。

- 主なユーザー向け chat product
- runtime state の分岐した所有者
- private memory を無関係な note にコピーする場所
- gateway pairing、tenant scope、memory governance を迂回する経路

役に立つ見方は、運用者のための desk であり、第二のコンパニオンではない、というものです。

## デスクトップ変更の検証

desktop source を変更した場合は、プロジェクト定義の checks を使います。

```bash
pnpm --dir desktop exec oxfmt
pnpm --dir desktop exec oxlint --react-plugin src
pnpm --dir desktop build
```

build は大きな shared vendor chunk について warning を出すことがあります。その warning だけで build failure とは扱いません。
