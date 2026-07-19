-- Seed data for the Nova Helpdesk demo app. Run against the workspace
-- schema the app was registered into:
--   psql "$DATABASE_URL" -v schema=<workspace_schema> -f examples/helpdesk_seed.sql
set search_path to :"schema", public;

truncate helpdesk_tickets restart identity;
truncate helpdesk_agents restart identity;

insert into helpdesk_agents (name, team, active) values
  ('Priya Sharma',   'Support',  true),
  ('Marcus Chen',    'Support',  true),
  ('Sofia Reyes',    'Billing',  true),
  ('Tomasz Nowak',   'Platform', true),
  ('Aisha Bello',    'Support',  true),
  ('Jordan Lee',     'Billing',  false);

insert into helpdesk_tickets (subject, status, priority, agent, urgent, satisfaction, notes, created_at) values
  ('Cannot log in after password reset',        'Resolved',    'High',   'Priya Sharma',  false, 5, 'Cleared stale session cookie.',        now() - interval '13 days'),
  ('Invoice PDF renders blank',                 'Resolved',    'Medium', 'Sofia Reyes',   false, 4, 'Fixed by re-running the export.',      now() - interval '13 days'),
  ('API rate limits too aggressive',            'In Progress', 'High',   'Tomasz Nowak',  true,  3, 'Investigating burst allowance.',       now() - interval '12 days'),
  ('Add SSO via Okta',                          'Open',        'High',   'Tomasz Nowak',  false, 3, null,                                   now() - interval '12 days'),
  ('Typo on pricing page',                      'Resolved',    'Low',    'Marcus Chen',   false, 5, null,                                   now() - interval '12 days'),
  ('Webhook retries duplicate events',          'In Progress', 'High',   'Tomasz Nowak',  true,  2, 'Idempotency keys missing.',            now() - interval '11 days'),
  ('Dark mode flickers on load',                'Open',        'Low',    'Marcus Chen',   false, 3, null,                                   now() - interval '11 days'),
  ('Refund not showing in statement',           'Resolved',    'High',   'Sofia Reyes',   true,  4, 'Refund posted after 3 business days.', now() - interval '11 days'),
  ('CSV import drops UTF-8 rows',               'In Progress', 'Medium', 'Priya Sharma',  false, 3, 'BOM handling bug confirmed.',          now() - interval '10 days'),
  ('Mobile app crashes on iOS 19',              'Open',        'High',   'Aisha Bello',   true,  1, 'Crash log attached by customer.',      now() - interval '10 days'),
  ('Two-factor codes arrive late',              'Open',        'Medium', 'Priya Sharma',  false, 2, null,                                   now() - interval '9 days'),
  ('Update billing address',                    'Resolved',    'Low',    'Sofia Reyes',   false, 5, null,                                   now() - interval '9 days'),
  ('Export to Excel misaligns columns',         'Resolved',    'Medium', 'Marcus Chen',   false, 4, null,                                   now() - interval '9 days'),
  ('Custom domain SSL renewal failed',          'In Progress', 'High',   'Tomasz Nowak',  true,  2, 'ACME challenge misconfigured.',        now() - interval '8 days'),
  ('Notification emails go to spam',            'Open',        'Medium', 'Aisha Bello',   false, 3, 'DMARC alignment issue suspected.',     now() - interval '8 days'),
  ('Team member cannot be removed',             'Resolved',    'Medium', 'Priya Sharma',  false, 4, null,                                   now() - interval '8 days'),
  ('Dashboard loads slowly with 10k rows',      'In Progress', 'High',   'Tomasz Nowak',  false, 3, 'Adding keyset pagination.',            now() - interval '7 days'),
  ('Wrong currency shown for EU accounts',      'Open',        'High',   'Sofia Reyes',   true,  2, null,                                   now() - interval '7 days'),
  ('Feature request: saved filters',            'Open',        'Low',    'Marcus Chen',   false, 4, null,                                   now() - interval '7 days'),
  ('Password policy too strict',                'Resolved',    'Low',    'Priya Sharma',  false, 3, null,                                   now() - interval '6 days'),
  ('Audit log missing API events',              'In Progress', 'Medium', 'Tomasz Nowak',  false, 3, null,                                   now() - interval '6 days'),
  ('Duplicate charge on annual plan',           'Resolved',    'High',   'Sofia Reyes',   true,  5, 'Refunded and confirmed with bank.',    now() - interval '6 days'),
  ('Search ignores accented characters',        'Open',        'Medium', 'Marcus Chen',   false, 3, null,                                   now() - interval '5 days'),
  ('Trial extension request',                   'Resolved',    'Low',    'Aisha Bello',   false, 5, null,                                   now() - interval '5 days'),
  ('GraphQL endpoint returns 502 intermittently','In Progress','High',   'Tomasz Nowak',  true,  2, 'Correlates with deploy windows.',      now() - interval '5 days'),
  ('Cannot upload avatar over 2MB',             'Open',        'Low',    'Aisha Bello',   false, 3, null,                                   now() - interval '4 days'),
  ('Receipt email shows wrong company name',    'Resolved',    'Medium', 'Sofia Reyes',   false, 4, null,                                   now() - interval '4 days'),
  ('Keyboard shortcuts conflict with browser',  'Open',        'Low',    'Marcus Chen',   false, 3, null,                                   now() - interval '4 days'),
  ('Data export stuck at 99%',                  'In Progress', 'Medium', 'Priya Sharma',  false, 2, 'Large attachment table suspected.',    now() - interval '3 days'),
  ('Add Danish language support',               'Open',        'Low',    'Aisha Bello',   false, 3, null,                                   now() - interval '3 days'),
  ('Charged after cancellation',                'In Progress', 'High',   'Sofia Reyes',   true,  1, 'Escalated to billing engineering.',    now() - interval '3 days'),
  ('Onboarding checklist never completes',      'Open',        'Medium', 'Priya Sharma',  false, 3, null,                                   now() - interval '2 days'),
  ('Slack integration posts twice',             'Open',        'Medium', 'Tomasz Nowak',  false, 2, null,                                   now() - interval '2 days'),
  ('Broken image on landing page',              'Resolved',    'Low',    'Marcus Chen',   false, 5, null,                                   now() - interval '2 days'),
  ('Rename workspace fails silently',           'Open',        'Medium', 'Priya Sharma',  false, 3, null,                                   now() - interval '1 day'),
  ('VAT number rejected as invalid',            'Open',        'High',   'Sofia Reyes',   false, 2, null,                                   now() - interval '1 day'),
  ('Session expires during long form',          'In Progress', 'Medium', 'Aisha Bello',   false, 3, null,                                   now() - interval '1 day'),
  ('Add API key scoping',                       'Open',        'High',   'Tomasz Nowak',  false, 4, null,                                   now() - interval '10 hours'),
  ('Printing a report cuts off columns',        'Open',        'Low',    'Marcus Chen',   false, 3, null,                                   now() - interval '6 hours'),
  ('Thank you note - great support!',           'Resolved',    'Low',    'Aisha Bello',   false, 5, 'Customer very happy.',                 now() - interval '2 hours');
