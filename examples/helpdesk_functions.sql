-- PL/pgSQL functions for the Nova Helpdesk demo's "call_function"
-- action (src/actions/call_function.rs) — pgapp's way of running
-- server-side logic that already lives beside the data, instead of
-- round-tripping it through Rust.
--
-- Run this BEFORE the app's first `cargo run` / `/admin/reload`, not
-- after (unlike helpdesk_seed.sql, which needs pgapp_data.* to already
-- exist). A `call_function` action's named query is `select
-- close_stale_tickets()`, and pgapp resolves that query's bind types
-- by asking Postgres to describe it at sync time — so the function
-- has to exist first, the same way any table a query joins against
-- has to exist first:
--
--   psql "$DATABASE_URL" -f examples/helpdesk_functions.sql
--   cargo run -- examples/helpdesk.pgapp
--   psql "$DATABASE_URL" -f examples/helpdesk_seed.sql   # after, once pgapp_data exists

create or replace function close_stale_tickets() returns text
language plpgsql as $$
declare
  n integer;
begin
  update pgapp_data.helpdesk_tickets
     set status = 'Resolved'
   where status = 'Open'
     and created_at < now() - interval '10 days';
  get diagnostics n = row_count;

  if n = 0 then
    -- A validation-style failure: pgapp shows this exact message as
    -- the action's error banner (see actions::clean_db_error) — the
    -- PL/pgSQL function decides what "failure" means, not Rust.
    raise exception 'No stale open tickets to close.';
  end if;

  return format('Closed %s stale ticket(s).', n);
end;
$$;
