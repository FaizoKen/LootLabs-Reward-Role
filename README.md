# Loot Labs Reward Role

Lightweight Rust backend that grants a Discord role when a member completes one [Loot Labs](https://lootlabs.gg) task. Built as a [RoleLogic](https://rolelogic.faizo.net) plugin — admins configure everything from the RoleLogic dashboard, no separate admin webpage.

> **Note**: This plugin uses the centralized Auth Gateway for member-facing Discord login. It does NOT bring its own Discord OAuth credentials.

## How it works

1. **Admin** creates a Role Link in RoleLogic → plugin auto-generates the guild's postback secret on `POST /register` (only the first role link per server triggers this; later ones reuse it).
2. **Admin** opens the role link's config in the RoleLogic dashboard:
   - **One time per Discord server**: paste the postback URL into [creators.lootlabs.gg/advanced](https://creators.lootlabs.gg/advanced). Subsequent role links in the same server show a "✓ already configured" note instead.
   - **Per role link**: paste a Loot Labs short link from [creators.lootlabs.gg/dashboard](https://creators.lootlabs.gg/dashboard), set role duration in hours (`0` = permanent), share the member-facing claim link in Discord.
3. **Member** clicks the claim link → logs in via the Auth Gateway → plugin issues a single-use `puid` token and 302-redirects to the Loot Labs short link with `&puid={token}` appended.
4. **Loot Labs** fires a postback to `/postback?k={guild_secret}&CLICK_ID={puid}` once the task is complete.
5. **Plugin** resolves the puid → registration + guild → discord_id, verifies `?k=` in constant time against the guild secret, calls RoleLogic's User Management API to grant the role with the configured expiry.
6. **Background worker** revokes roles whose duration has elapsed (permanent claims are skipped).

## Why URL-secret auth (no HMAC)

Loot Labs's postback UI exposes only the URL field plus checkbox macros (`{CLICK_ID}`, `{IP}`, `{UNIQUE_ID}`). There's no signature option. Security comes from:

- The `?k=<32-byte-random>` baked into the postback URL — one secret per Discord guild — anyone who doesn't have it can't forge a valid callback.
- The unguessable per-claim `puid` token — even with `?k=` leaked, an attacker can't trigger an unsolicited grant without a valid token issued by the plugin's claim flow.

If you suspect a leak: rotate the secret (`UPDATE guild_secrets SET secret = ... WHERE guild_id = ...`) and re-paste the URL into Loot Labs.

## Setup

```bash
cp .env.example .env
# Edit .env with your values
```

### Environment Variables

| Variable           | Required | Default          | Description                                                                                  |
| ------------------ | -------- | ---------------- | -------------------------------------------------------------------------------------------- |
| `DATABASE_URL`     | Yes      | —                | PostgreSQL connection string                                                                 |
| `SESSION_SECRET`   | Yes      | —                | MUST match the Auth Gateway's `SESSION_SECRET` byte-for-byte                                 |
| `BASE_URL`         | Yes      | —                | Public URL with path prefix, e.g. `https://plugin-rolelogic.faizo.net/lootlabs-reward-role`  |
| `AUTH_GATEWAY_URL` | Yes      | —                | Where the Auth Gateway is reachable, e.g. `http://auth-gateway:8090`                         |
| `LISTEN_ADDR`      | No       | `0.0.0.0:8088`   | Bind address                                                                                 |
| `RUST_LOG`         | No       | `…info`          | Log filter (`debug` for verbose troubleshooting)                                             |

## Run

### Docker (recommended)

```bash
docker compose up -d
```

### From source

```bash
cargo run              # development
cargo build --release  # production
```

## Endpoints

All routes are nested under `/lootlabs-reward-role`:

| Method   | Path                           | Auth                          | Description                                          |
| -------- | ------------------------------ | ----------------------------- | ---------------------------------------------------- |
| `POST`   | `/register`                    | RoleLogic token               | Ensures a guild-scoped postback secret exists        |
| `GET`    | `/config`                      | RoleLogic token               | Returns the four-section config form schema          |
| `POST`   | `/config`                      | RoleLogic token               | Persists short link + role duration                  |
| `DELETE` | `/config`                      | RoleLogic token               | Removes the registration (cascades)                  |
| `GET`    | `/postback`                    | URL secret (`?k=`)            | Loot Labs callback (one URL per guild)               |
| `GET`    | `/claim/{registration_id}`     | `rl_session` cookie           | Member claim flow → 302 to Loot Labs                 |
| `GET`    | `/health`                      | none                          | DB connectivity check                                |
| `GET`    | `/favicon.ico`                 | none                          | Static                                               |

## Database schema

Four tables:

- **`guild_secrets`** — one per Discord guild. Holds the `?k=` value baked into the shared postback URL. Generated on first `/register` for a given guild and reused for every later role link in that guild.
- **`registrations`** — one per role link. Stores the RoleLogic API token, the admin-supplied `lootlabs_short_link`, and `role_duration_hours` (NULL = permanent).
- **`claim_tokens`** — single-use `puid` tokens issued at `/claim/{id}` start. 24h TTL. `consumed_at` set on first matching postback. Cleaned up after the retention window (24h post-consumed).
- **`claims`** — successful grants. `expires_at` NULL = permanent. Partial unique index `(registration_id, discord_id) WHERE revoked_at IS NULL` prevents duplicate active rows.

## Usage

### First role link in a Discord server

1. In the RoleLogic dashboard, create a Role Link and set **Custom Plugin URL** to `https://your-domain.com/lootlabs-reward-role`.
2. RoleLogic registers the guild/role pair → plugin ensures the guild's postback secret exists.
3. Open the plugin config in RoleLogic — copy the postback URL shown in **Step 1**.
4. In Loot Labs:
   - Go to [creators.lootlabs.gg/advanced](https://creators.lootlabs.gg/advanced) → paste the postback URL → tick `{CLICK_ID}` (Loot Labs forces this) → Save. **You only do this once per Discord server.**
   - Go to [creators.lootlabs.gg/dashboard](https://creators.lootlabs.gg/dashboard) → create a Link → copy the short link (e.g. `https://links.lootlabs.gg/s?ABCDEF`).
5. Back in RoleLogic config: paste the short link, set role duration, save.
6. Share the **claim link** (also shown in the config) with your Discord members.

### Adding more role links to the same server

1. Create another Role Link in RoleLogic with the same Custom Plugin URL.
2. Open the plugin config — Step 1 will say "✓ already configured", so just create a new Loot Labs short link, paste it, set duration, save.
3. Share the new claim link.

That's it. No second trip to `creators.lootlabs.gg/advanced`.

## Operational notes

- **Role duration `0` or blank = permanent.** Permanent claims are skipped by the expiry worker.
- **One short link per role link, one postback URL per guild.** Multiple offers in the same Discord server = multiple Role Links — each gets its own short link and duration, but they all share the guild's postback URL.
- **Any Loot Labs short-link domain works.** Loot Labs hands out links on `links.lootlabs.gg`, `lootdest.org`, `loot-link.com`, and others — all share the same backend, so paste whichever your dashboard shows. The plugin doesn't validate the host beyond requiring HTTPS.
- **Claim tokens have a 24h TTL.** A member who starts a claim must complete the Loot Labs task within 24 hours.
- **Loot Labs retries failed callbacks.** The plugin always returns `200 {"ok": true}` to avoid leaking signal about validity to attackers; bad signatures, missing tokens, and consumed tokens are logged but not surfaced as errors.
- **No SQL admin UI.** Admin actions are the four RoleLogic config endpoints. Direct DB edits work fine for one-off ops (rotating a secret, retiring a stale registration).

## API Reference

- [RoleLogic Role Link API](https://docs-rolelogic.faizo.net/reference/role-link-api)
- [Loot Labs Postback API](https://help.lootlabs.gg/en/article/postback-api-1ndz3i2/)
- [Loot Labs Redirect API (anti-bypass — optional)](https://help.lootlabs.gg/en/article/lootlabs-redirect-api-anti-bypass-9o0tky/)

## License

[MIT](LICENSE)
