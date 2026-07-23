[← Back to README](../README.md)

# Secrets

A fixed credential an action needs — an API key, a service account
token — that isn't user-typed and shouldn't sit in plaintext in the
markup file. Managed with the same instance-mode CLI as everything
else (see [Instance mode](./getting-started.md#instance-mode)):

```bash
pgapp secret set <name> (--workspace <slug> | --app <slug>)
pgapp secret list (--workspace <slug> | --app <slug>)
pgapp secret rm <name> (--workspace <slug> | --app <slug>)
```

`set` prompts for the value interactively rather than taking it as an
argument (`--value` exists for scripts, but — unlike the prompt — it
lands in shell history and `ps`). Referenced from markup as
`{{secret.<name>}}`, anywhere `http_request` already accepts
`{{item}}` (`url`/`body`/`headers`/`token`/`username`/`password`/
`key_value`):

```text
action "Create ticket" calls http_request (
  url: "https://api.example.com/tickets",
  auth: "bearer",
  token: "{{secret.api_token}}"
)
```

An app-scoped secret shadows a workspace-scoped one of the same name —
same precedent a page-scoped named query already sets over an
app-scoped one. Storage lives in `pgapp_control` (pgapp managing
itself, the same registry `workspace`/`app` commands use), not
`pgapp_meta` — untouched by a markup resync, so a secret survives
every `admin/reload` and app rebuild for free, exactly like the
workspace/app registry itself does.

**Encrypted, never hashed.** A hash is one-way — right for the CLI
admin password (see [Instance mode](./getting-started.md#instance-mode)), which is only ever
*compared*, but useless for a secret that has to be sent back out in
plaintext (an `Authorization` header). Secrets are AES-256-GCM
encrypted at rest instead. The key itself never touches this database
— same "never written to disk" story as `pgapp_admin`'s own Postgres
password: it's read fresh from `PGAPP_SECRET_KEY` (64 hex characters —
`openssl rand -hex 32`) by every command or request that actually
needs a value, and isn't required at all otherwise (`secret list`/`rm`,
or an app with no `{{secret...}}` references, work fine without it ever
being set).

The App Builder also has a **Secrets** panel (add/update/remove) on
each app's AppSettings page — see [App Builder](./app-builder.md#editing-an-apps-data-model-queries-navigation-and-settings).

---

Next: [Actions](./actions.md) · [Instance mode](./getting-started.md#instance-mode)
