---
title: 運用
description: ローカル運用者向けのバックアップ、復元、シークレットローテーション、監視の runbook。
---

# 運用

Asterel は単一運用者のランタイムです。個人マシンで動かしている場合でも、ワークスペース、データベース、ローカルのシークレット保管庫は本番状態として扱います。

## バックアップ

記憶、人格、設定の整合性を保つため、次のものはまとめてバックアップします。

- `~/.asterel/config.toml` と、ワークスペース固有の設定上書き。
- 暗号化されたシークレット保管庫と、シークレット暗号化のメタデータ。
- `postgres` メモリバックエンドを使っている場合は PostgreSQL データベース。
- 代替バックエンドを使っている場合は Markdown メモリディレクトリ。
- デーモンで使っているリリース成果物、または正確な commit SHA。

PostgreSQL では、アップグレード前に logical dump を取ります。

```bash
pg_dump --format=custom --file=asterel.dump "$ASTEREL_POSTGRES_URL"
```

バックアップは保存時にも暗号化してください。プロバイダーキーを、暗号化されていないバグ報告、CI artifacts、共有ログへコピーしないでください。

## 復元

1. デーモンとチャネル worker を停止する。
2. 設定とシークレット保管庫を先に復元する。
3. PostgreSQL を空のデータベースへ復元する。

   ```bash
   pg_restore --clean --if-exists --dbname "$ASTEREL_POSTGRES_URL" asterel.dump
   ```

4. `asterel doctor` を実行し、memory、gateway、channel、observability の状態を確認する。
5. デーモンを起動し、最初の数ターンは post-turn hook と memory metrics を見る。

## シークレットローテーション

運用者の引き継ぎ、露出の疑い、プロバイダーダッシュボードでの変更、公開 ingress の設定ミスがあった場合は、シークレットをローテーションします。

1. 上流サービスで古い provider / channel / tunnel token を失効させる。
2. 設定済みのシークレットストア、または暗号化設定の経路から新しいシークレットを書き込む。シークレット暗号化が有効なときは、平文ファイルを直接編集しない。
3. デーモン、または影響するチャネル worker を再起動する。
4. 影響する provider または transport の最小 smoke test を実行する。
5. ログと metrics で認証失敗がないか確認する。

## 監視

長時間動くデーモンでは `prometheus` observability backend を使います。observer は Prometheus text exposition の snapshot を出力できます。公開する場合は localhost か信頼済み管理者だけが scrape できる endpoint に限定してください。

少なくとも次を追います。

- observer の event / error totals。
- post-turn hook status totals。
- memory lifecycle と SLO violation totals。
- signal ingestion と deduplication labels。
- channel worker heartbeat status。

デーモンが traffic を受けているのに metrics が動かない場合は、incident として扱います。ログを取り、必要ならチャネル配送を止め、post-turn update が失敗した memory call や provider call の裏で詰まっていないか確認します。
