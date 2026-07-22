-- Seed data for the pgapp Showcase demo app. Run against the workspace
-- schema the app was registered into, after the app's first sync has
-- created the tables ($DATABASE_URL isn't set by pgapp itself — export
-- it yourself first, the same connection string you gave
-- `pgapp instance init`):
--   export DATABASE_URL=postgres://user:pass@host:5432/<dbname>
--   psql "$DATABASE_URL" -v schema=<workspace_schema> -f examples/showcase_seed.sql
set search_path to :"schema", public;

truncate products restart identity;
truncate tasks restart identity;
truncate cities restart identity;
truncate feedback restart identity;
truncate team_members restart identity;

insert into products (name, category, price, rating, featured, in_stock, description) values
  ('Mechanical Keyboard',   'Electronics', '89.99',  5, true,  true,  'Hot-swappable switches, per-key RGB.'),
  ('Wireless Mouse',        'Electronics', '34.50',  4, false, true,  'Silent clicks, 2000 DPI.'),
  ('USB-C Hub',             'Electronics', '24.00',  3, false, true,  '7-in-1, HDMI + 100W passthrough.'),
  ('Standing Desk',         'Furniture',   '349.00', 5, true,  true,  'Electric height adjust, memory presets.'),
  ('Ergonomic Chair',       'Furniture',   '279.00', 4, false, true,  'Adjustable lumbar support.'),
  ('Desk Lamp',             'Furniture',   '42.99',  3, false, false, 'Dimmable, USB-powered.'),
  ('Notebook (dot grid)',   'Stationery',  '8.50',   4, false, true,  'A5, 160 pages.'),
  ('Fountain Pen',          'Stationery',  '19.99',  5, true,  true,  'Fine nib, converter included.'),
  ('Sticky Notes (6-pack)', 'Stationery',  '5.25',   3, false, true,  'Assorted colors.'),
  ('Insulated Mug',         'Kitchen',     '22.00',  4, false, true,  'Keeps drinks hot for 6 hours.');

insert into tasks (title, status, priority, due_date, urgent, assignee, tags, notes) values
  ('Write onboarding docs',        'Open',        'Medium', to_char(now() + interval '5 days', 'YYYY-MM-DD'), false, 'Priya Sharma',  'Docs',            'Cover the CLI and the markup grammar.'),
  ('Fix flaky pagination test',    'In Progress', 'High',   to_char(now() - interval '2 days', 'YYYY-MM-DD'), true,  'Marcus Chen',   'Backend',         'Fails under keyset mode only.'),
  ('Design empty states',          'Open',        'Low',    to_char(now() + interval '10 days','YYYY-MM-DD'), false, 'Sofia Reyes',   'Frontend',        null),
  ('Add dark mode toggle',         'Done',        'Medium', to_char(now() - interval '9 days', 'YYYY-MM-DD'), false, 'Sofia Reyes',   'Frontend',        'Shipped in the theme layer.'),
  ('Investigate slow report load', 'In Progress', 'High',   to_char(now() - interval '1 days', 'YYYY-MM-DD'), true,  'Marcus Chen',   'Backend,Urgent-Fix', 'Suspect missing index on created_at.'),
  ('Update README quickstart',     'Open',        'Low',    to_char(now() + interval '3 days', 'YYYY-MM-DD'), false, 'Priya Sharma',  'Docs',            null),
  ('Set up CI for examples',       'Open',        'Medium', to_char(now() - interval '4 days', 'YYYY-MM-DD'), false, 'Tomasz Nowak',  'Backend',         'Overdue — needs re-prioritizing.'),
  ('Review faceted search UX',     'Done',        'Low',    to_char(now() - interval '14 days','YYYY-MM-DD'), false, 'Sofia Reyes',   'Frontend',        null),
  ('Draft release notes',          'Open',        'Medium', to_char(now() + interval '1 days', 'YYYY-MM-DD'), false, 'Priya Sharma',  'Docs',            null),
  ('Audit color contrast',         'Open',        'Low',    to_char(now() - interval '6 days', 'YYYY-MM-DD'), false, 'Aisha Bello',   'Frontend',        'Overdue.');

insert into cities (name, country, lat, lng, population) values
  ('San Francisco', 'USA',     '37.7749',  '-122.4194', 873965),
  ('New York',      'USA',     '40.7128',  '-74.0060',  8804190),
  ('London',        'UK',      '51.5072',  '-0.1276',   8982000),
  ('Berlin',        'Germany', '52.5200',  '13.4050',   3645000),
  ('Tokyo',         'Japan',   '35.6762',  '139.6503',  13960000),
  ('Sydney',        'Australia','-33.8688','151.2093',  5312000),
  ('Bengaluru',     'India',   '12.9716',  '77.5946',   13193000);

insert into feedback (name, email, topic, rating, comments) values
  ('Alex Turner',   'alex@example.com',   'Praise',           5, 'The faceted search is exactly what I needed.'),
  ('Jamie Rivera',  'jamie@example.com',  'Bug',              2, 'Star rating did not save on first try.'),
  ('Sam Okafor',    'sam@example.com',    'Feature Request',  4, 'Would love a Kanban view for tasks.'),
  ('Morgan Diaz',   'morgan@example.com', 'General',          4, 'Docs are clear, thanks!'),
  ('Riley Chen',    'riley@example.com',  'Praise',           5, 'Setup took five minutes end to end.');

insert into team_members (name, role, department, color, bio, skills, active) values
  ('Priya Sharma',  'Tech Writer',    'Docs',        '#336791', 'Writes the docs nobody reads until they need them.',        'Writing,Docs',          true),
  ('Marcus Chen',   'Backend Eng.',   'Engineering', '#2f6f4f', 'Keeps the pagination honest.',                              'Rust,SQL,Postgres',     true),
  ('Sofia Reyes',   'Product Design', 'Design',      '#8a4baf', 'Designs the empty states everyone forgets to test.',        'Design',                true),
  ('Tomasz Nowak',  'DevOps',         'Engineering', '#c2410c', 'CI, deploys, and the occasional 3am page.',                 'DevOps,Postgres',       true),
  ('Aisha Bello',   'QA',             'Engineering', '#0e7490', 'Finds the bug before the customer does.',                  'Rust,SQL',              false);
