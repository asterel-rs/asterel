---
title: ゲートウェイ
description: ローカル HTTP / WebSocket ゲートウェイが、単一運用者向け Asterel 配置の中で果たす役割。
---

ゲートウェイは、コンパニオン・ランタイムの周りにあるローカル HTTP / WebSocket edge です。health check、pairing、webhook、A2A 風の message、companion surface route、admin API に使います。公開 SaaS の境界ではありません。

## ローカルで起動する

```bash
cargo run -- gateway --host 127.0.0.1 --port 3000
```

通常運用では daemon を推奨します。daemon は gateway、channels、scheduler、heartbeat、runtime services をまとめて動かします。

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
```

## ローカル優先の姿勢

安全な既定値は、ローカル bind と pairing 必須です。

```toml
[gateway]
host = "127.0.0.1"
port = 3000
require_pairing = true
allow_public_bind = false
```

公開到達性が必要な場合は、信頼済み edge または tunnel を runtime の前段に置きます。ローカル管理 API を、認証なしの公開サービスにしないでください。

## Public routes と admin routes

Public routes には health、readiness、pairing、gateway OpenAPI、webhook、A2A、companion surface、WebSocket entrypoint が含まれます。Admin routes は `/admin/v1/*` の下にあり、pairing と明示的な tenant scope が必要です。

```text
Authorization: Bearer <token>
X-Asterel-Tenant: <tenant-id>
```

tenant header はローカル運用者状態を scope します。Asterel が hosted multi-tenant SaaS であるという主張ではありません。

## ゲートウェイが所有すべきでないもの

ゲートウェイは transport input を正規化し、route を公開します。記憶、人格、セッション継続性、prompt policy、応答検証の所有者になるべきではありません。それらは共有 runtime / core services に属します。そうすることで、Discord、gateway、channel turns が別々のプロダクトへ drift しないようにします。
