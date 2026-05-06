-- gen_random_uuid() is built-in on PostgreSQL 13+. Safety net for older
-- managed Postgres providers that ship it via pgcrypto.
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- One row per Discord guild. Holds the `?k=` value baked into the postback
-- URL the admin pastes into Loot Labs. Loot Labs has no HMAC option, so
-- security is via URL secrecy. Per-guild (not per-role-link) so admins
-- only paste the URL once per Discord server.
CREATE TABLE guild_secrets (
    guild_id    TEXT PRIMARY KEY,
    secret      TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One row per RoleLogic role link. The postback secret lives in
-- guild_secrets, not here — postback dispatch uses the puid token, which
-- is already unique + unguessable, so registration_id doesn't need to be
-- in the URL.
CREATE TABLE registrations (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    guild_id             TEXT NOT NULL,
    role_id              TEXT NOT NULL,
    rolelogic_token      TEXT NOT NULL,
    lootlabs_short_link  TEXT,
    role_duration_hours  INTEGER,            -- NULL = permanent
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (guild_id, role_id)
);

-- Single-use claim tokens. Plugin issues one when a member starts the
-- claim flow, redirects to Loot Labs with `&puid={TOKEN}`. Loot Labs
-- echoes it back in the postback's `{CLICK_ID}` macro.
CREATE TABLE claim_tokens (
    token            TEXT PRIMARY KEY,
    registration_id  UUID NOT NULL REFERENCES registrations(id) ON DELETE CASCADE,
    discord_id       TEXT NOT NULL,
    issued_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at       TIMESTAMPTZ NOT NULL,
    consumed_at      TIMESTAMPTZ
);
CREATE INDEX claim_tokens_expiry_idx ON claim_tokens (expires_at) WHERE consumed_at IS NULL;
CREATE INDEX claim_tokens_discord_idx ON claim_tokens (discord_id);

-- Successful claims. expires_at NULL = permanent role; otherwise the
-- role-expiry worker revokes via RoleLogic API once now() passes it.
CREATE TABLE claims (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    registration_id  UUID NOT NULL REFERENCES registrations(id) ON DELETE CASCADE,
    discord_id       TEXT NOT NULL,
    granted_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at       TIMESTAMPTZ,
    revoked_at       TIMESTAMPTZ
);
CREATE INDEX claims_active_expiry_idx ON claims (expires_at)
    WHERE expires_at IS NOT NULL AND revoked_at IS NULL;
CREATE INDEX claims_discord_idx ON claims (discord_id);
CREATE UNIQUE INDEX claims_active_unique
    ON claims (registration_id, discord_id)
    WHERE revoked_at IS NULL;
