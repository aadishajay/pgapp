-- PL/pgSQL functions for the pgapp Showcase demo.
--
-- Run this BEFORE the app's first sync / `/admin/reload` (same
-- convention as examples/helpdesk_functions.sql — a `dynamic_content`
-- or `call_function` component's named query is `select
-- <fn>(...)`, and pgapp resolves that query's shape by asking
-- Postgres to describe it at sync time, so the function has to exist
-- first). Tables referenced here are created by the app's own sync, so
-- run examples/showcase_seed.sql (which needs those tables) only
-- after:
--
--   export DATABASE_URL=postgres://user:pass@host:5432/<dbname>
--   psql "$DATABASE_URL" -v schema=<workspace_schema> -f examples/showcase_functions.sql
--   pgapp run examples/showcase.pgapp --workspace <slug>
--   psql "$DATABASE_URL" -v schema=<workspace_schema> -f examples/showcase_seed.sql

set search_path to :"schema", public;

-- Backs the Home page's `dynamic_content` component — Oracle APEX's
-- "PL/SQL Dynamic Content" region. Returned as trusted, unescaped HTML
-- (see render::dynamic_content_html), so this is the one place in the
-- whole app free to use rich markup a `text` component's escaping
-- would otherwise block. Every link below is a bare relative href
-- (page name only) rather than an absolute path, so it resolves
-- against whatever workspace/app slug this ends up deployed under —
-- see render.rs's own "/{app}/{page}" convention, which this
-- deliberately mirrors without needing to know `app` itself.
create or replace function home_hero_html() returns text
language sql as $$
  select '
<div class="pgapp-card" style="margin-bottom:1.5rem;">
  <h1 class="pgapp-title" style="margin-top:0;">A tour of every pgapp component, in one app</h1>
  <p class="pgapp-text">This whole application — every page, table, chart, and form below — is defined in a single <code>.pgapp</code> markup file. No per-app Rust, no ORM migrations: entities become tables, pages become routes, and every widget is one declarative block. Pick a functional area to explore it.</p>
</div>
<div class="pgapp-cards">
  <div class="pgapp-card">
    <h3 style="margin-top:0;">Products</h3>
    <p class="pgapp-card-label">Classic report + form CRUD, star ratings, a rich-text description field, computed columns, currency formatting, aggregates, and a control break.</p>
    <a class="pgapp-link pgapp-btn pgapp-btn-primary" href="Products">Open Products</a>
    <a class="pgapp-link pgapp-btn pgapp-btn-secondary" href="ProductGallery">Gallery view</a>
  </div>
  <div class="pgapp-card">
    <h3 style="margin-top:0;">Tasks</h3>
    <p class="pgapp-card-label">Faceted search, checkbox-group tags, dynamic actions (show/hide/set/refresh), and a month calendar view.</p>
    <a class="pgapp-link pgapp-btn pgapp-btn-primary" href="Tasks">Open Tasks</a>
    <a class="pgapp-link pgapp-btn pgapp-btn-secondary" href="TaskCalendar">Calendar view</a>
  </div>
  <div class="pgapp-card">
    <h3 style="margin-top:0;">Team</h3>
    <p class="pgapp-card-label">Color pickers, a skills shuttle, a rich-text bio, and a separate inline-editable table view of the same data.</p>
    <a class="pgapp-link pgapp-btn pgapp-btn-primary" href="Team">Open Team</a>
    <a class="pgapp-link pgapp-btn pgapp-btn-secondary" href="TeamQuickEdit">Quick-edit table</a>
  </div>
  <div class="pgapp-card">
    <h3 style="margin-top:0;">Cities</h3>
    <p class="pgapp-card-label">An inline-SVG Map region plotting rows by latitude/longitude, no map-tile service required.</p>
    <a class="pgapp-link pgapp-btn pgapp-btn-primary" href="Cities">Open Cities</a>
  </div>
  <div class="pgapp-card">
    <h3 style="margin-top:0;">Dashboard</h3>
    <p class="pgapp-card-label">Every chart type — bar, line, area, pie, donut, scatter — each one line of markup over a named SQL query.</p>
    <a class="pgapp-link pgapp-btn pgapp-btn-primary" href="Dashboard">Open Dashboard</a>
  </div>
  <div class="pgapp-card">
    <h3 style="margin-top:0;">Feedback</h3>
    <p class="pgapp-card-label">A public-style submission form (star rating, popup topic picker) plus server-side actions: a raw SQL action and a PL/pgSQL one.</p>
    <a class="pgapp-link pgapp-btn pgapp-btn-primary" href="Feedback">Open Feedback</a>
  </div>
</div>
<p class="pgapp-text">Curious how any of this works? <a class="pgapp-link" href="About">Read the About page</a>, or open the <code>.pgapp</code> file this app is built from — it is genuinely the whole application.</p>
'
$$;

-- Backs an `action ... calls call_function` component on the Feedback
-- page — the PL/pgSQL-native sibling of a raw `run_query` action (see
-- src/actions/call_function.rs). Returns a plain summary message on
-- success; a real validation failure would `raise exception` instead,
-- which pgapp shows as the action's error banner rather than this one.
create or replace function showcase_daily_digest() returns text
language plpgsql as $$
declare
  n_tasks integer;
  n_open integer;
  n_feedback integer;
  avg_rating numeric;
begin
  select count(*) into n_tasks from showcase_tasks;
  select count(*) into n_open from showcase_tasks where status = 'Open';
  select count(*), avg(rating) into n_feedback, avg_rating from showcase_feedback;

  return format(
    '%s task(s) tracked (%s open); %s feedback submission(s), average rating %s.',
    n_tasks, n_open, n_feedback, coalesce(round(avg_rating, 1)::text, 'n/a')
  );
end;
$$;
