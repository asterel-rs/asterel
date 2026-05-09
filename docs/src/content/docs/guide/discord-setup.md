---
title: Run Discord
description: "How to connect the current primary Asterel product surface: Discord text through the local daemon."
---

Discord text is the current product-proof surface. Other adapters may compile and
load, but Discord is where Asterel first tries to prove the full companion loop:
pickup policy, memory-backed continuity, public/private distance, response
finalization, and post-turn writeback.

## Before you start

You need:

- onboarding completed with `cargo run -- onboard --interactive`;
- a configured model provider or local model;
- the daemon running locally;
- a Discord bot token;
- the server or DM context where the bot is allowed to respond.

For durable relationship continuity, PostgreSQL is the recommended memory
backend. Markdown fallback is useful for constrained testing, but it is not the
full product posture.

## 1. Add Discord config

Discord settings live under `channels_config.discord` in
`~/.asterel/config.toml`. Keep secrets out of copied examples and prefer an
environment secret path when possible.

```toml
[channels_config.discord]
bot_token = "DISCORD_BOT_TOKEN"
# Optional: restrict to one server.
guild_id = "DISCORD_GUILD_ID"
# Optional: restrict who can talk to the bot.
allowed_users = ["DISCORD_USER_ID"]
thinking_embed = true

[channels_config.discord.pickup_policy]
mode = "direct_only"
max_unsummoned_replies_per_hour = 0
min_gap_seconds = 600
```

The default pickup posture is intentionally quiet. Start with direct mentions and
DMs. Turn on sparse ambient behavior only after the operator is comfortable with
public-room behavior.

## 2. Validate local config

```bash
cargo run -- config validate
cargo run -- doctor
cargo run -- channel list
```

These commands catch most setup problems before Discord traffic is involved:
missing provider credentials, missing memory configuration, invalid TOML, or a
disabled channel.

## 3. Run the daemon

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
```

The daemon is the normal product shape. It keeps gateway routes, channels,
scheduler, heartbeat, memory, and the shared companion turn runtime around one
runtime instance.

## 4. Test a direct turn

In Discord, mention the bot or send a DM. If accepted, the message follows the
shared companion-turn path:

```text
Discord event -> pickup policy -> turn enrichment -> response assembly
  -> response finalization -> reply delivery -> post-turn update
```

If there is no reply, do not immediately loosen all settings. Check the decision
points in order:

1. the bot token is valid;
2. the bot is present in the intended server or DM;
3. `guild_id` and `allowed_users` are not excluding the message;
4. pickup policy accepts the message;
5. the provider can complete a turn;
6. response finalization allows the response to be sent.

## Public-room rule

Asterel should not behave like a noisy room bot. Public-room distance is part of
the product promise. A direct mention or clear invitation is safer than ambient
chatter, and private memory should not surface in public just because it was
useful grounding.
