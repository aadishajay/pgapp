-- pgapp's own control plane: which apps this server serves, and where
-- their markup lives on disk. Deliberately a separate schema from
-- pgapp_meta (the synced runtime metadata *of* an app) and pgapp_data
-- (an app's own rows) — this table is pgapp managing itself, closer to
-- what an APEX workspace/application registry is than to anything an
-- app author's markup ever touches.
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
-- Null = the classic single-workspace flow (data tables in the global
-- pgapp_data schema, data_schema defaults accordingly). Set = this
-- app's data tables live in that workspace's own schema instead. `on
-- delete cascade`: a hard-deleted workspace drops its schema (and
-- every table in it) outright, so an app row still pointing at it
-- would only be stale bookkeeping — removing it too keeps
-- pgapp_control.apps from ever naming a workspace that no longer
-- exists.
alter table pgapp_control.apps add column if not exists workspace_id integer references pgapp_control.workspaces(id) on delete cascade;
alter table pgapp_control.apps add column if not exists data_schema text not null default 'pgapp_data';
-- The app's declared name (app "Name" { ... }), not its URL slug —
-- needed to look up its pgapp_meta.apps row (keyed by that name) when
-- hard-deleting an app, without re-parsing its markup file from disk.
alter table pgapp_control.apps add column if not exists app_name text not null default '';
