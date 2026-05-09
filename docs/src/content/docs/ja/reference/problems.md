---
title: Problem details
description: ゲートウェイが返す RFC 9457 problem type URI の安定した anchor。
---

Asterel gateway の error は [RFC 9457 Problem Details](https://www.rfc-editor.org/rfc/rfc9457.html) を使います。`type` field はこのページを指し、fragment は machine-readable な `code` field と一致します。例: `#invalid_request`。

code ごとの fragment は安定した identifier です。pre-release の間は、code ごとの詳しい説明が runtime に追いつかないことがあります。正確な挙動は response の `title`、`detail`、HTTP status、source code を優先してください。

## Common codes

### invalid_request

request の形、parameter、payload を受理できませんでした。

### unauthorized

request に有効な authentication がありません。

### forbidden

caller は authenticated ですが、requested action を実行する権限がありません。

### not_found

caller の scope 内で requested resource が見つかりませんでした。

### conflict

request が現在の runtime state と衝突しています。

### rate_limited

caller が rate limit または replay protection window を超えました。

### internal_error

runtime が予期せず失敗しました。
