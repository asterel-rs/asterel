// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import mdx from '@astrojs/mdx';

// Project site on GitHub Pages: https://asterel-rs.github.io/asterel/
// `site` + `base` are both required for correct canonical URLs and asset paths.
export default defineConfig({
  site: 'https://asterel-rs.github.io',
  base: '/asterel',
  integrations: [
    starlight({
      title: 'Asterel',
      description:
        'An early-stage Discord-first AI companion runtime for durable memory, persona, and relationship continuity.',
      locales: {
        root: {
          label: 'English',
          lang: 'en',
        },
        ja: {
          label: '日本語',
          lang: 'ja',
        },
      },
      defaultLocale: 'root',
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/asterel-rs/asterel',
        },
      ],
      editLink: {
        baseUrl:
          'https://github.com/asterel-rs/asterel/edit/main/docs/',
      },
      customCss: ['./src/styles/docs.css'],
      sidebar: [
        {
          label: 'Start here',
          translations: { ja: 'はじめに' },
          items: [
            { label: 'Overview', translations: { ja: '概要' }, slug: 'overview' },
            {
              label: 'Getting started',
              translations: { ja: '始め方' },
              slug: 'guide/getting-started',
            },
            {
              label: 'Run Discord',
              translations: { ja: 'Discord を動かす' },
              slug: 'guide/discord-setup',
            },
            {
              label: 'Operating locally',
              translations: { ja: 'ローカル運用' },
              slug: 'guide/operating-locally',
            },
            {
              label: 'Configuration',
              translations: { ja: '設定' },
              slug: 'guide/configuration',
            },
            {
              label: 'Troubleshooting',
              translations: { ja: 'トラブルシュート' },
              slug: 'guide/troubleshooting',
            },
          ],
        },
        {
          label: 'Concepts',
          translations: { ja: 'コンセプト' },
          items: [
            {
              label: 'Companion runtime',
              translations: { ja: 'コンパニオン・ランタイム' },
              slug: 'concepts/companion',
            },
            {
              label: 'Continuity over conversation',
              translations: { ja: '会話より継続性' },
              slug: 'concepts/continuity',
            },
            {
              label: 'Memory model',
              translations: { ja: 'メモリモデル' },
              slug: 'concepts/memory-model',
            },
            {
              label: "Asterel's silhouette",
              translations: { ja: 'Asterel の輪郭' },
              slug: 'concepts/asterel-silhouette',
            },
            {
              label: 'Character and persona',
              translations: { ja: 'キャラクターと人格' },
              slug: 'concepts/character-persona',
            },
            {
              label: 'What Asterel is not',
              translations: { ja: 'Asterel ではないもの' },
              slug: 'concepts/boundaries',
            },
          ],
        },
        {
          label: 'Operator guide',
          translations: { ja: '運用者ガイド' },
          items: [
            {
              label: 'Gateway',
              translations: { ja: 'ゲートウェイ' },
              slug: 'operator/gateway',
            },
            {
              label: 'Desktop console',
              translations: { ja: 'デスクトップコンソール' },
              slug: 'operator/desktop-console',
            },
            {
              label: 'Memory review',
              translations: { ja: '記憶レビュー' },
              slug: 'operator/memory-review',
            },
            {
              label: 'Security and governance',
              translations: { ja: 'セキュリティとガバナンス' },
              slug: 'architecture/security-governance',
            },
          ],
        },
        {
          label: 'Architecture',
          translations: { ja: 'アーキテクチャ' },
          items: [
            {
              label: 'Turn pipeline',
              translations: { ja: 'ターンパイプライン' },
              slug: 'architecture/turn-pipeline',
            },
            {
              label: 'Layered dependencies',
              translations: { ja: '依存レイヤー' },
              slug: 'architecture/layers',
            },
          ],
        },
        {
          label: 'Research packet',
          translations: { ja: '研究パケット' },
          items: [
            { label: 'Overview', slug: 'research' },
            { label: 'Claims', slug: 'research/claims' },
            { label: 'Methodology', slug: 'research/methodology' },
            { label: 'Evidence ledger', slug: 'research/evidence-ledger' },
            { label: 'Harness effectiveness', slug: 'research/harness-effectiveness' },
            { label: 'Technical report v0.1', slug: 'research/technical-report-v0-1' },
            { label: 'Public release roadmap', slug: 'research/public-release-roadmap' },
            { label: 'Benchmark roadmap', slug: 'research/benchmark-roadmap' },
            { label: 'Ablation plan', slug: 'research/ablation-plan' },
            { label: 'Experimental protocol', slug: 'research/experimental-protocol' },
            { label: 'Reproducibility', slug: 'research/reproducibility' },
            { label: 'Publication boundary', slug: 'research/publication-boundary' },
          ],
        },
        {
          label: 'Reference',
          translations: { ja: 'リファレンス' },
          items: [
            { label: 'Problem details', slug: 'reference/problems' },
            { label: 'Research references', slug: 'reference/references' },
          ],
        },
      ],
    }),
    mdx(),
  ],
});
