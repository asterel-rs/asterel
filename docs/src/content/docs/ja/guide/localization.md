---
title: ローカライズ
description: CLI とランタイムのローカライズを英語・日本語以外へ広げる方法。
---

# ローカライズ

Asterel は現在、`locales/en.yml` と `locales/ja.yml` の locale file を同梱しています。ランタイムの locale は `ASTEREL_LANG`、設定済み locale、system `LANG` の順で選ばれ、最後は英語にフォールバックします。

## locale を追加する

1. `locales/en.yml` を `locales/<iso-639-1>.yml` にコピーする。
2. locale file 間で key を同じに保つ。
3. 運用者向けの短い表現を優先する。コマンド名や設定キーを翻訳で変えない。
4. config locale tests と、少なくとも一つの onboarding smoke test を実行する。
5. 新しい locale に運用者固有のセットアップ注意が必要なら、ドキュメントを更新する。

locale の追加は、単なる文字列置換ではなくプロダクト作業として扱ってください。error message、onboarding prompt、安全性やガバナンスに関わる文言は、その locale で自然に読めるレビューを通してから supported として出します。
