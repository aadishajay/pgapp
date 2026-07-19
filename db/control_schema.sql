-- pgapp's own control plane: which apps this server serves, and where
-- their markup lives on disk. Deliberately a separate schema from
-- pgapp_meta (the synced runtime metadata *of* an app) and a
-- workspace's own schema (an app's rows) — this table is pgapp
-- managing itself, closer to what an APEX workspace/application
-- registry is than to anything an app author's markup ever touches.
create schema if not exists pgapp_control;

-- A workspace is a Postgres schema an app's data tables live in —
-- either one pgapp created (with its own owning login role and
-- password, granting pgapp_admin USAGE/CREATE into it) or an existing
-- schema pgapp was granted access to. Every app in a workspace shares
-- that one schema for its data tables; pgapp_meta/pgapp_control stay
-- global to the whole instance regardless of workspace.
create table if not exists pgapp_control.workspaces (
    id          serial primary key,
    slug        text not null unique,
    schema_name text not null unique,
    owner_role  text,
    enabled     boolean not null default true,
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now()
);

create table if not exists pgapp_control.apps (
    id          serial primary key,
    slug        text not null unique,
    markup_path text not null,
    enabled     boolean not null default true,
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now()
);
-- Every app belongs to exactly one workspace — its data tables live in
-- that workspace's own schema (`data_schema`). `on delete cascade`: a
-- hard-deleted workspace drops its schema (and every table in it)
-- outright, so an app row still pointing at it would only be stale
-- bookkeeping — removing it too keeps pgapp_control.apps from ever
-- naming a workspace that no longer exists.
alter table pgapp_control.apps add column if not exists workspace_id integer references pgapp_control.workspaces(id) on delete cascade;
alter table pgapp_control.apps alter column workspace_id set not null;
alter table pgapp_control.apps add column if not exists data_schema text not null default 'pgapp_data';
-- The app's declared name (app "Name" { ... }), not its URL slug —
-- needed to look up its pgapp_meta.apps row (keyed by that name) when
-- hard-deleting an app, without re-parsing its markup file from disk.
alter table pgapp_control.apps add column if not exists app_name text not null default '';

-- `pgapp secret set/list/rm` (see src/secrets.rs) — a workspace- or
-- app-scoped named secret, referenced from markup as
-- `{{secret.<name>}}` (same interpolation `http_request` already does
-- for page items, just resolved from here instead of `ctx.values`).
-- Lives in pgapp_control, not pgapp_meta, for the same reason the
-- workspace/app registry does: it's pgapp managing itself, untouched
-- by an app's own markup resync, so it survives every rebuild/upgrade
-- for free — nothing here is ever derived from a `.pgapp` file.
--
-- `ciphertext`/`nonce` are AES-256-GCM output, never a hash: a secret
-- has to be sent back out in plaintext (e.g. an Authorization header),
-- so a one-way hash — right for a login password, which is only ever
-- compared — would be useless here. The key that decrypts these never
-- lives in this database; see `secrets::load_key` (read fresh from
-- `PGAPP_SECRET_KEY` at process start, same pattern
-- `PGAPP_ADMIN_DB_PASSWORD` already uses for the `pgapp_admin` role's
-- own password).
create table if not exists pgapp_control.secrets (
    id           serial primary key,
    workspace_id integer references pgapp_control.workspaces(id) on delete cascade,
    app_id       integer references pgapp_control.apps(id) on delete cascade,
    name         text not null,
    ciphertext   bytea not null,
    nonce        bytea not null,
    created_at   timestamptz not null default now(),
    updated_at   timestamptz not null default now(),
    constraint secrets_exactly_one_scope check (
        (workspace_id is not null and app_id is null) or
        (workspace_id is null and app_id is not null)
    )
);
-- Partial (not plain) unique indexes: a plain `unique (workspace_id,
-- name)` would treat every app-scoped row (workspace_id null) as
-- equal to every other on that column, wrongly colliding names across
-- unrelated apps.
create unique index if not exists secrets_workspace_name on pgapp_control.secrets (workspace_id, name) where workspace_id is not null;
create unique index if not exists secrets_app_name on pgapp_control.secrets (app_id, name) where app_id is not null;
