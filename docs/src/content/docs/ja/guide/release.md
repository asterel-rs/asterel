---
title: リリースチェックリスト
description: タグ付きリリースで使うビルドプロファイル、成果物、確認項目、署名の期待値。
---

# リリースチェックリスト

タグ付きリリースは `.github/workflows/release.yml` でビルドします。対象は Linux、macOS Intel、macOS Apple Silicon、Windows です。

## ビルドプロファイル

ルートの `Cargo.toml` では `[profile.release]` を明示しています。サイズを意識した最適化、LTO、codegen unit の 1 本化、シンボル削除、`panic = "abort"` が設定されています。

リリース成果物では、Cargo の暗黙の既定値に頼らないでください。

## 成果物への署名

リリースアーカイブは publish job で Cosign の keyless blob signing により署名します。各アーカイブには、対応する `*.bundle.sigstore.json` bundle を GitHub Release にアップロードします。

bundle には、検証に必要な署名、証明書、transparency log の証明が含まれます。

検証例:

```bash
cosign verify-blob \
  --bundle asterel-x86_64-unknown-linux-gnu.tar.gz.bundle.sigstore.json \
  asterel-x86_64-unknown-linux-gnu.tar.gz
```

## タグを打つ前に

- `cargo fmt -- --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- ドキュメントを変更した場合は docs build
- `CHANGELOG.md` に対象タグのリリースノートがあることを確認する
