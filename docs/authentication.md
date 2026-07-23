[← Back to README](../README.md)

# Authentication & authorization

- [Opting in](#opting-in)
- [Component-level `requires:` and named auth schemes](#component-level-requires-and-named-auth-schemes)

## Opting in

Opt in with an `auth { }` block:

- Every page requires a signed-in user; only `/:workspace/:app/login` and static
  assets stay public.
- First run bootstraps the admin (`POST /:workspace/:app/setup`, one-time); after
  that, admins manage accounts on the built-in `/:workspace/:app/users` page.
  Users are never declared in markup.
- Passwords are argon2id hashes in `pgapp_meta.users`; sessions are
  server-side rows in `pgapp_meta.sessions` (an HttpOnly, `SameSite=Lax`
  cookie holds only a random token — revoking a session means deleting
  its row). pgapp itself never terminates TLS (see
  [Architecture](./architecture.md)) — put a reverse proxy in front for
  production, and the session cookie picks up `Secure` automatically
  once that proxy forwards `X-Forwarded-Proto: https`.
- A user holds any number of `roles` (free-form strings, comma-separated
  on the Add-user form). `requires: <role>` gates a page (and its
  create/update/delete routes) to a user holding that role, or admin.

Without `auth { }`, an app stays fully public
(`examples/todo.pgapp`); `examples/helpdesk.pgapp` runs behind a login
with an admin-only Agents page.

## Component-level `requires:` and named auth schemes

`requires:` isn't only a page property — any component (`button`,
`form`, `report`, `action`, ...) can carry its own trailing
`requires: <role>`, same slot as `attrs (...)` and combinable with it,
in either order:

```text
button "Approve" calls approve_bill requires: finance
form "Edit rates" of rates { fields: rate } requires: finance attrs (class: "wide")
```

A component a user doesn't have the role for is simply left out of the
page (same "just don't show it" precedent as a role-gated nav item) —
and its own create/update/delete/run route independently re-checks the
same role server-side, so hiding the button is never the only thing
standing between an unauthorized user and the write.

A `requires:` name can also be a reusable **auth scheme** — a named
role group declared once at app scope:

```text
auth_scheme "can_approve" {
  roles: finance, manager
}
```

`requires: can_approve` then passes for anyone holding *either* role.
This is resolved by name at check time: an app with no matching scheme
just treats the name as a literal role, so introducing schemes is
purely additive and never a breaking change to an existing
`requires:`. This is the pgapp analog of an Oracle APEX Authorization
Scheme (see [Migrating from Oracle APEX](./migration-from-apex.md)) — minus the
PL/SQL-expression and SQL-query scheme types APEX also supports, and
minus APEX's separate Access Control allow-list, neither of which
pgapp apps have needed yet.

---

Next: [Actions](./actions.md) · [Secrets](./secrets.md)
