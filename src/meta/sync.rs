//! Upserts a parsed [`AppDef`] into `pgapp_meta.*` and makes sure the
//! physical data table for each entity exists.
//!
//! Syncing happens in phases because components (via `link:`/nav/
//! header/footer) can reference *other* pages by name, including ones
//! declared later in the file: phase 1 creates entities/fields/tables,
//! phase 2 creates a bare row for every page (so every page has an id),
//! and phase 3 onward resolves everything that points at a page or
//! entity by name. Components are stored as `(kind, config jsonb)` —
//! `config` embeds page/entity/query *names* directly (not ids), so
//! nothing downstream needs another join to resolve them; sync's job is
//! just to validate those names exist before writing them.

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;

use crate::actions;
use crate::item_types::{self, Registry};
use crate::model::{AppDef, ComponentDef, EntityDef, FieldDef, FieldItem, FieldType, HtmlAttrs, PreAction, QueryDef};

fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('_');
            last_was_sep = true;
        }
    }
    out.trim_matches('_').to_string()
}

/// Seed value for `pgapp_meta.app_runtime_js`, written on an app's first
/// sync only — after that the database row is the one that's served.
const DEFAULT_RUNTIME_JS: &str = include_str!("../runtime.js");

/// Upserts the app/entity/field/page/component/nav/query metadata and
/// makes sure the physical data table for each entity exists (and, when
/// it already existed, that its column types still match the declared
/// fields — see `verify_data_table`). `registry` validates every
/// `item ... as <kind>` and `action_registry` every `action ... calls
/// <name>` against the compiled-in modules, so a typo fails at sync
/// time with a clear message instead of silently rendering nothing.
/// `data_schema` is where this app's entity tables live — its
/// workspace's own schema (see `src/control.rs`); it's trusted to
/// already be a validated identifier (see `instance::valid_identifier`),
/// never end-user input.
pub async fn sync_app(
    pool: &PgPool,
    app: &AppDef,
    registry: &Registry,
    action_registry: &actions::Registry,
    data_schema: &str,
) -> Result<()> {
    // Defense in depth: data_schema gets spliced directly into DDL
    // below (Postgres has no bind-parameter form for identifiers), and
    // unlike entity/page/field names it isn't lexer-restricted — it
    // comes from a workspace's schema_name. That's validated where it's
    // chosen (src/instance.rs, src/control.rs), but this is the one
    // place every path converges.
    if !crate::instance::valid_identifier(data_schema) {
        anyhow::bail!("'{data_schema}' is not a valid schema identifier");
    }

    let app_id: i32 = sqlx::query_scalar(
        "insert into pgapp_meta.apps (name, theme, icons, chart_lib, auth_enabled, data_schema)
         values ($1, $2, $3, $4, $5, $6)
         on conflict (name) do update set
            theme = excluded.theme,
            icons = excluded.icons,
            chart_lib = excluded.chart_lib,
            auth_enabled = excluded.auth_enabled,
            data_schema = excluded.data_schema
         returning id",
    )
    .bind(&app.name)
    .bind(&app.theme)
    .bind(&app.icons)
    .bind(&app.chart_lib)
    .bind(app.auth)
    .bind(data_schema)
    .fetch_one(pool)
    .await?;

    sync_queries(pool, app_id, None, &app.queries).await?;
    sync_auth_schemes(pool, app_id, &app.auth_schemes).await?;

    // Seed the runtime JS library on first sync only; once a row exists
    // it's the database's to edit, not this binary's to overwrite.
    sqlx::query(
        "insert into pgapp_meta.app_runtime_js (app_id, content) values ($1, $2)
         on conflict (app_id) do nothing",
    )
    .bind(app_id)
    .bind(DEFAULT_RUNTIME_JS)
    .execute(pool)
    .await?;

    // Phase 1: entities, fields, physical data tables. A query-backed
    // entity (`from query <name>`) gets metadata but no table — it's
    // read-only by construction.
    let mut entity_ids: HashMap<String, i32> = HashMap::new();
    for entity in &app.entities {
        let table_name = format!("{}_{}", slug(&app.name), slug(&entity.name));

        if let Some(query_name) = &entity.source_query {
            if !app.queries.iter().any(|q| &q.name == query_name) {
                anyhow::bail!(
                    "entity '{}' is backed by unknown query '{query_name}' \
                     (query-backed entities can only use app-scoped queries)",
                    entity.name
                );
            }
        }

        let entity_id: i32 = sqlx::query_scalar(
            "insert into pgapp_meta.entities (app_id, name, table_name, source_query, source_collection)
             values ($1, $2, $3, $4, $5)
             on conflict (app_id, name) do update set
                table_name = excluded.table_name,
                source_query = excluded.source_query,
                source_collection = excluded.source_collection
             returning id",
        )
        .bind(app_id)
        .bind(&entity.name)
        .bind(&table_name)
        .bind(&entity.source_query)
        .bind(&entity.source_collection)
        .fetch_one(pool)
        .await?;

        for (ordinal, field) in entity.fields.iter().enumerate() {
            sqlx::query(
                "insert into pgapp_meta.fields
                    (entity_id, name, data_type, is_required, default_value, ordinal)
                 values ($1, $2, $3, $4, $5, $6)
                 on conflict (entity_id, name) do update set
                    data_type = excluded.data_type,
                    is_required = excluded.is_required,
                    default_value = excluded.default_value,
                    ordinal = excluded.ordinal",
            )
            .bind(entity_id)
            .bind(&field.name)
            .bind(field.ty.as_str())
            .bind(field.required)
            .bind(&field.default)
            .bind(ordinal as i32)
            .execute(pool)
            .await?;
        }

        if entity.source_query.is_none() && entity.source_collection.is_none() {
            ensure_data_table(pool, data_schema, &table_name, entity).await?;
            verify_data_table(pool, data_schema, &table_name, entity).await?;
        }
        entity_ids.insert(entity.name.clone(), entity_id);
    }

    // Phase 2: a bare row for every page, so every page has an id before
    // anything below tries to link to one by name.
    let mut page_ids: HashMap<String, i32> = HashMap::new();
    for page in &app.pages {
        let page_id: i32 = sqlx::query_scalar(
            "insert into pgapp_meta.pages (app_id, name, required_role) values ($1, $2, $3)
             on conflict (app_id, name) do update set required_role = excluded.required_role
             returning id",
        )
        .bind(app_id)
        .bind(&page.name)
        .bind(&page.required_role)
        .fetch_one(pool)
        .await?;

        page_ids.insert(page.name.clone(), page_id);
    }

    // Phase 3: page-scoped queries and components — components may
    // reference another page (link targets), an entity, or a query by
    // name, all of which now have ids/known names to validate against.
    for page in &app.pages {
        let page_id = page_ids[&page.name];

        sync_queries(pool, app_id, Some(page_id), &page.queries).await?;

        sqlx::query("delete from pgapp_meta.components where page_id = $1")
            .bind(page_id)
            .execute(pool)
            .await?;

        for (ordinal, component) in page.components.iter().enumerate() {
            let known_query = |name: &str| {
                page.queries.iter().any(|q| q.name == name) || app.queries.iter().any(|q| q.name == name)
            };
            let (kind, mut config) = build_component_config(
                component,
                app,
                &entity_ids,
                &page_ids,
                registry,
                action_registry,
                &known_query,
                &format!("page '{}'", page.name),
            )?;
            merge_html_into_config(&mut config, component_html(component));
            merge_requires_into_config(&mut config, component_requires(component));

            sqlx::query(
                "insert into pgapp_meta.components (app_id, page_id, slot, kind, ordinal, config)
                 values ($1, $2, null, $3, $4, $5)",
            )
            .bind(app_id)
            .bind(page_id)
            .bind(kind)
            .bind(ordinal as i32)
            .bind(config)
            .execute(pool)
            .await?;
        }
    }

    // Phase 4: the nav tree, which can also reference any page by name.
    sqlx::query("delete from pgapp_meta.nav_items where app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await?;
    for (ordinal, item) in app.nav.iter().enumerate() {
        sync_nav_item(pool, app_id, None, ordinal as i32, item, &page_ids).await?;
    }

    // A page removed from the markup (the App Builder's "Delete Page"
    // or "Rename Page" — a rename is a delete-of-the-old-name plus an
    // insert-of-the-new-one, since the upsert above conflicts on
    // (app_id, name) — or just hand-edited out) needs its
    // `pgapp_meta.pages` row gone too, not just absent from `page_ids`
    // above — otherwise it lingers forever, still visible to anything
    // reading `pgapp_meta` directly (the App Builder's own "Pages"
    // listing is exactly that). Cascades to
    // `pgapp_meta.components`/`.saved_views`/etc. via their own
    // `on delete cascade` (see db/schema.sql) — but `nav_items` isn't
    // one of those cascades, so this has to run *after* Phase 4 has
    // already retargeted every nav item at the fresh page ids above,
    // or deleting a renamed-away-from page id here would still violate
    // `nav_items_target_page_id_fkey` for any nav item that used to
    // point at it.
    let current_page_names: Vec<&str> = app.pages.iter().map(|p| p.name.as_str()).collect();
    sqlx::query("delete from pgapp_meta.pages where app_id = $1 and not (name = any($2))")
        .bind(app_id)
        .bind(&current_page_names)
        .execute(pool)
        .await?;

    // Phase 5: the app-wide header/footer chrome — restricted to
    // Text/Link/Region, which is all "chrome" (content with no entity
    // or pagination behind it) is meant to be.
    sync_chrome(pool, app_id, "header", &app.header, &page_ids).await?;
    sync_chrome(pool, app_id, "footer", &app.footer, &page_ids).await?;

    Ok(())
}

fn sync_nav_item<'a>(
    pool: &'a PgPool,
    app_id: i32,
    parent_id: Option<i32>,
    ordinal: i32,
    item: &'a crate::model::NavItem,
    page_ids: &'a HashMap<String, i32>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        let target_id = match &item.target_page {
            None => None,
            Some(name) => Some(*page_ids.get(name).with_context(|| {
                format!("nav item '{}' links to unknown page '{name}'", item.label)
            })?),
        };

        let nav_id: i32 = sqlx::query_scalar(
            "insert into pgapp_meta.nav_items (app_id, parent_id, label, target_page_id, ordinal)
             values ($1, $2, $3, $4, $5)
             returning id",
        )
        .bind(app_id)
        .bind(parent_id)
        .bind(&item.label)
        .bind(target_id)
        .bind(ordinal)
        .fetch_one(pool)
        .await?;

        for (child_ordinal, child) in item.children.iter().enumerate() {
            sync_nav_item(pool, app_id, Some(nav_id), child_ordinal as i32, child, page_ids).await?;
        }

        Ok(())
    })
}

/// Replaces the named queries owned by an app (`page_id` null) or one
/// page (`page_id` set).
async fn sync_queries(
    pool: &PgPool,
    app_id: i32,
    page_id: Option<i32>,
    queries: &[QueryDef],
) -> Result<()> {
    match page_id {
        Some(pid) => {
            sqlx::query("delete from pgapp_meta.named_queries where page_id = $1")
                .bind(pid)
                .execute(pool)
                .await?;
        }
        None => {
            sqlx::query("delete from pgapp_meta.named_queries where app_id = $1 and page_id is null")
                .bind(app_id)
                .execute(pool)
                .await?;
        }
    }

    for q in queries {
        sqlx::query(
            "insert into pgapp_meta.named_queries (app_id, page_id, name, sql_text)
             values ($1, $2, $3, $4)",
        )
        .bind(app_id)
        .bind(page_id)
        .bind(&q.name)
        .bind(&q.sql)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// Replaces the app's named auth schemes — same delete-then-reinsert
/// pattern as `sync_queries`.
async fn sync_auth_schemes(pool: &PgPool, app_id: i32, schemes: &[crate::model::AuthScheme]) -> Result<()> {
    sqlx::query("delete from pgapp_meta.auth_schemes where app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await?;

    for scheme in schemes {
        sqlx::query("insert into pgapp_meta.auth_schemes (app_id, name, roles) values ($1, $2, $3)")
            .bind(app_id)
            .bind(&scheme.name)
            .bind(&scheme.roles)
            .execute(pool)
            .await?;
    }

    Ok(())
}

/// Replaces the app-wide header/footer chrome (`slot` = "header" or
/// "footer", `page_id` null). Only Text/Link/Region components are
/// allowed here — chrome has no pagination or per-request entity
/// context to give a Report/Form/EditableTable/Chart anything
/// meaningful to do.
async fn sync_chrome(
    pool: &PgPool,
    app_id: i32,
    slot: &str,
    components: &[ComponentDef],
    page_ids: &HashMap<String, i32>,
) -> Result<()> {
    sqlx::query("delete from pgapp_meta.components where app_id = $1 and slot = $2")
        .bind(app_id)
        .bind(slot)
        .execute(pool)
        .await?;

    for (ordinal, component) in components.iter().enumerate() {
        let (kind, mut config) = match component {
            ComponentDef::Text { text, .. } => ("text", serde_json::json!({ "text": text })),
            ComponentDef::Link { label, target_page, .. } => {
                if !page_ids.contains_key(target_page) {
                    anyhow::bail!("app {slot} links to unknown page '{target_page}'");
                }
                ("link", serde_json::json!({ "label": label, "target_page": target_page }))
            }
            ComponentDef::Region { label, query, columns, .. } => (
                "region",
                serde_json::json!({ "label": label, "query": query, "columns": columns }),
            ),
            other => anyhow::bail!(
                "app {slot} may only contain text/link/region components, found {}",
                component_kind_name(other)
            ),
        };
        merge_html_into_config(&mut config, component_html(component));
        merge_requires_into_config(&mut config, component_requires(component));

        sqlx::query(
            "insert into pgapp_meta.components (app_id, page_id, slot, kind, ordinal, config)
             values ($1, null, $2, $3, $4, $5)",
        )
        .bind(app_id)
        .bind(slot)
        .bind(kind)
        .bind(ordinal as i32)
        .bind(config)
        .execute(pool)
        .await?;
    }

    Ok(())
}

fn component_kind_name(c: &ComponentDef) -> &'static str {
    match c {
        ComponentDef::Report { .. } => "report",
        ComponentDef::Form { .. } => "form",
        ComponentDef::EditableTable { .. } => "editable_table",
        ComponentDef::Chart { .. } => "chart",
        ComponentDef::Text { .. } => "text",
        ComponentDef::Link { .. } => "link",
        ComponentDef::Region { .. } => "region",
        ComponentDef::DynamicContent { .. } => "dynamic_content",
        ComponentDef::Action { .. } => "action",
        ComponentDef::Button { .. } => "button",
        ComponentDef::DynamicAction { .. } => "dynamic_action",
        ComponentDef::Calendar { .. } => "calendar",
        ComponentDef::Map { .. } => "map",
    }
}

/// Every component (except `DynamicAction`, which has no wrapper tag)
/// carries an optional `html` override — `id`/`class`/extra attributes
/// from a trailing `attrs (...)` suffix in the markup. Reading it here,
/// independent of `kind`, means neither `sync_chrome` nor
/// `build_component_config`'s per-kind match has to plumb it through
/// individually.
fn component_html(c: &ComponentDef) -> &HtmlAttrs {
    static EMPTY: HtmlAttrs = HtmlAttrs { id: None, class: None, attrs: Vec::new() };
    match c {
        ComponentDef::Report { html, .. }
        | ComponentDef::Form { html, .. }
        | ComponentDef::EditableTable { html, .. }
        | ComponentDef::Chart { html, .. }
        | ComponentDef::Text { html, .. }
        | ComponentDef::Link { html, .. }
        | ComponentDef::Region { html, .. }
        | ComponentDef::DynamicContent { html, .. }
        | ComponentDef::Action { html, .. }
        | ComponentDef::Button { html, .. }
        | ComponentDef::Calendar { html, .. }
        | ComponentDef::Map { html, .. } => html,
        ComponentDef::DynamicAction { .. } => &EMPTY,
    }
}

/// Every component (except `DynamicAction`) also carries an optional
/// `requires` — a per-component role gate from a trailing
/// `requires: <role>` suffix, same generic-reading pattern as
/// `component_html`.
fn component_requires(c: &ComponentDef) -> Option<&str> {
    match c {
        ComponentDef::Report { requires, .. }
        | ComponentDef::Form { requires, .. }
        | ComponentDef::EditableTable { requires, .. }
        | ComponentDef::Chart { requires, .. }
        | ComponentDef::Text { requires, .. }
        | ComponentDef::Link { requires, .. }
        | ComponentDef::Region { requires, .. }
        | ComponentDef::DynamicContent { requires, .. }
        | ComponentDef::Action { requires, .. }
        | ComponentDef::Button { requires, .. }
        | ComponentDef::Calendar { requires, .. }
        | ComponentDef::Map { requires, .. } => requires.as_deref(),
        ComponentDef::DynamicAction { .. } => None,
    }
}

/// Splices `requires` into `config` under a reserved `"requires"` key —
/// read back generically in `meta::load`, independent of `kind`.
fn merge_requires_into_config(config: &mut serde_json::Value, requires: Option<&str>) {
    let Some(role) = requires else { return };
    config
        .as_object_mut()
        .expect("component config is always a JSON object")
        .insert("requires".to_string(), serde_json::Value::String(role.to_string()));
}

/// `HtmlAttrs` -> `{"id": ..., "class": ..., "attrs": {...}}`, the wire
/// shape `meta::load::decode_html_attrs` reads back — shared by both
/// component-level `html` and per-field `field_html`.
fn html_attrs_to_json(html: &HtmlAttrs) -> serde_json::Value {
    let attrs_obj: serde_json::Map<String, serde_json::Value> =
        html.attrs.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
    serde_json::json!({ "id": html.id, "class": html.class, "attrs": attrs_obj })
}

/// Splices `html`'s `id`/`class`/extra attributes into `config` under a
/// reserved `"html"` key — read back generically in `meta::load`,
/// independent of `kind`, the same way it's written here.
fn merge_html_into_config(config: &mut serde_json::Value, html: &HtmlAttrs) {
    if html.is_empty() {
        return;
    }
    config
        .as_object_mut()
        .expect("component config is always a JSON object")
        .insert("html".to_string(), html_attrs_to_json(html));
}

/// `{field: {"id": ..., "class": ..., "attrs": {...}}, ...}` for a
/// Form/EditableTable's `field_html` — one entry per field that used a
/// trailing `attrs (...)` on its `item` line. Validated the same way
/// `resolve_item_types` validates `item_types`: every key must be one
/// of the component's own declared fields.
fn field_html_json(
    field_html: &HashMap<String, HtmlAttrs>,
    known_fields: &[String],
    owner_label: &str,
) -> Result<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (field_name, html) in field_html {
        if !known_fields.iter().any(|f| f == field_name) {
            anyhow::bail!("{owner_label} sets attrs on unknown field '{field_name}'");
        }
        map.insert(field_name.clone(), html_attrs_to_json(html));
    }
    Ok(serde_json::Value::Object(map))
}

/// Resolves every field's item (kind + config), falling back to
/// `item_types::default_kind_for`, and validates the kind against the
/// registry and the field name against the entity — producing the
/// `{field_name: {"kind": ..., "config": ...}}` blob stored in a Form's
/// or EditableTable's component config.
fn resolve_item_types(
    entity: &EntityDef,
    field_names: &[String],
    item_types: &HashMap<String, FieldItem>,
    registry: &Registry,
    owner_label: &str,
) -> Result<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for field_name in field_names {
        let field = entity
            .fields
            .iter()
            .find(|f| &f.name == field_name)
            .with_context(|| format!("{owner_label} references unknown field '{field_name}'"))?;
        let field_item = item_types.get(field_name).cloned().unwrap_or_else(|| FieldItem {
            kind: item_types::default_kind_for(field.ty).to_string(),
            config: serde_json::json!({}),
        });

        if !registry.contains_key(field_item.kind.as_str()) {
            let known: Vec<&str> = registry.keys().copied().collect();
            anyhow::bail!(
                "{owner_label} field '{field_name}' uses unknown item type '{}' (known: {})",
                field_item.kind,
                known.join(", "),
            );
        }

        map.insert(
            field_name.clone(),
            serde_json::json!({ "kind": field_item.kind, "config": field_item.config }),
        );
    }
    Ok(serde_json::Value::Object(map))
}

/// Validates a `before_load`'s module name against the action registry
/// (same check `ComponentDef::Action` gets) and turns it into the
/// `{"name": ..., "config": ...}` shape `meta::load::decode_before_load`
/// reads back; `None` becomes JSON `null`.
fn before_load_json(
    before_load: &Option<PreAction>,
    action_registry: &actions::Registry,
    owner_label: &str,
) -> Result<serde_json::Value> {
    match before_load {
        None => Ok(serde_json::Value::Null),
        Some(pre) => {
            if !action_registry.contains_key(pre.name.as_str()) {
                let known: Vec<&str> = action_registry.keys().copied().collect();
                anyhow::bail!(
                    "{owner_label} before_load calls unknown module '{}' (known: {})",
                    pre.name,
                    known.join(", ")
                );
            }
            Ok(serde_json::json!({ "name": pre.name, "config": pre.config }))
        }
    }
}

/// Validates one page component and turns it into `(kind, config)` for
/// storage. `known_query` reports whether a name is visible (page- or
/// app-scoped) from the page this component lives on.
#[allow(clippy::too_many_arguments)]
fn build_component_config(
    component: &ComponentDef,
    app: &AppDef,
    entity_ids: &HashMap<String, i32>,
    page_ids: &HashMap<String, i32>,
    registry: &Registry,
    action_registry: &actions::Registry,
    known_query: &impl Fn(&str) -> bool,
    owner_label: &str,
) -> Result<(&'static str, serde_json::Value)> {
    match component {
        ComponentDef::Report {
            title,
            entity,
            columns,
            source_query,
            link_column,
            page_size,
            before_load,
            computed,
            formats,
            aggregates,
            break_on,
            highlights,
            display,
            ..
        } => {
            if !entity_ids.contains_key(entity) {
                anyhow::bail!("{owner_label} report '{title}' references unknown entity '{entity}'");
            }
            let entity_def = app.entity(entity).expect("checked above");
            if !highlights.is_empty() && (source_query.is_some() || entity_def.source_collection.is_some()) {
                anyhow::bail!(
                    "{owner_label} report '{title}' declares 'highlight' rules but its source isn't a plain \
                     entity table — highlight rules only apply to entity-backed reports"
                );
            }
            if !computed.is_empty() && source_query.is_some() {
                anyhow::bail!(
                    "{owner_label} report '{title}' declares 'computed' columns but also 'source: query \
                     ...' — computed columns only apply to entity-backed reports; add the expression to \
                     the query's SQL instead"
                );
            }
            for c in computed {
                if entity_def.fields.iter().any(|f| f.name == c.name) {
                    anyhow::bail!(
                        "{owner_label} report '{title}' computed column '{}' has the same name as a field of '{entity}'",
                        c.name
                    );
                }
            }
            for c in columns {
                let is_field = entity_def.fields.iter().any(|f| &f.name == c);
                let is_computed = computed.iter().any(|cc| &cc.name == c);
                if !is_field && !is_computed {
                    anyhow::bail!("{owner_label} report '{title}' column '{c}' is not a field of '{entity}' or a computed column");
                }
            }
            for col in formats.keys() {
                if !columns.contains(col) {
                    anyhow::bail!("{owner_label} report '{title}' formats column '{col}', which isn't in its 'columns:' list");
                }
            }
            for col in aggregates.keys() {
                if !columns.contains(col) {
                    anyhow::bail!("{owner_label} report '{title}' aggregates column '{col}', which isn't in its 'columns:' list");
                }
            }
            if let Some(col) = break_on {
                if !columns.contains(col) {
                    anyhow::bail!("{owner_label} report '{title}' breaks on column '{col}', which isn't in its 'columns:' list");
                }
            }
            if let Some(q) = source_query {
                if !known_query(q) {
                    anyhow::bail!("{owner_label} report '{title}' sources from unknown query '{q}'");
                }
            }
            let link_json = match link_column {
                None => serde_json::Value::Null,
                Some(lc) => {
                    if !page_ids.contains_key(&lc.target_page) {
                        anyhow::bail!(
                            "{owner_label} report '{title}' links to unknown page '{}'",
                            lc.target_page
                        );
                    }
                    serde_json::json!({
                        "field": lc.field,
                        "target_page": lc.target_page,
                        "extra_params": lc.extra_params,
                    })
                }
            };
            let before_load_json_val =
                before_load_json(before_load, action_registry, &format!("{owner_label} report '{title}'"))?;
            let computed_json: Vec<serde_json::Value> = computed
                .iter()
                .map(|c| serde_json::json!({"name": c.name, "sql": c.sql}))
                .collect();
            let formats_json: serde_json::Map<String, serde_json::Value> =
                formats.iter().map(|(col, mask)| (col.clone(), mask.to_json())).collect();
            let aggregates_json: serde_json::Map<String, serde_json::Value> = aggregates
                .iter()
                .map(|(col, agg)| (col.clone(), serde_json::Value::String(agg.as_str().to_string())))
                .collect();
            let highlights_json: Vec<serde_json::Value> = highlights
                .iter()
                .map(|h| serde_json::json!({"when": h.when, "color": h.color}))
                .collect();
            Ok((
                "report",
                serde_json::json!({
                    "title": title,
                    "entity": entity,
                    "columns": columns,
                    "source_query": source_query,
                    "link": link_json,
                    "page_size": page_size,
                    "before_load": before_load_json_val,
                    "computed": computed_json,
                    "formats": formats_json,
                    "aggregates": aggregates_json,
                    "break_on": break_on,
                    "highlights": highlights_json,
                    "display": display,
                }),
            ))
        }
        ComponentDef::Form {
            title,
            entity,
            fields,
            item_types,
            field_html,
            ..
        } => {
            if !entity_ids.contains_key(entity) {
                anyhow::bail!("{owner_label} form '{title}' references unknown entity '{entity}'");
            }
            let entity_def = app.entity(entity).expect("checked above");
            if entity_def.source_query.is_some() {
                anyhow::bail!(
                    "{owner_label} form '{title}' binds to entity '{entity}', which is \
                     query-backed and read-only — forms need a real table"
                );
            }
            if entity_def.source_collection.is_some() {
                anyhow::bail!(
                    "{owner_label} form '{title}' binds to entity '{entity}', which is \
                     collection-backed and read-only — forms need a real table"
                );
            }
            let owner = format!("{owner_label} form '{title}'");
            let resolved = resolve_item_types(entity_def, fields, item_types, registry, &owner)?;
            let field_html_resolved = field_html_json(field_html, fields, &owner)?;
            Ok((
                "form",
                serde_json::json!({
                    "title": title,
                    "entity": entity,
                    "fields": fields,
                    "item_types": resolved,
                    "field_html": field_html_resolved,
                }),
            ))
        }
        ComponentDef::EditableTable {
            title,
            entity,
            columns,
            item_types,
            field_html,
            ..
        } => {
            if !entity_ids.contains_key(entity) {
                anyhow::bail!(
                    "{owner_label} editable_table '{title}' references unknown entity '{entity}'"
                );
            }
            let entity_def = app.entity(entity).expect("checked above");
            if entity_def.source_query.is_some() {
                anyhow::bail!(
                    "{owner_label} editable_table '{title}' binds to entity '{entity}', which is \
                     query-backed and read-only — editable tables need a real table"
                );
            }
            if entity_def.source_collection.is_some() {
                anyhow::bail!(
                    "{owner_label} editable_table '{title}' binds to entity '{entity}', which is \
                     collection-backed and read-only — editable tables need a real table"
                );
            }
            let owner = format!("{owner_label} editable_table '{title}'");
            let resolved = resolve_item_types(entity_def, columns, item_types, registry, &owner)?;
            let field_html_resolved = field_html_json(field_html, columns, &owner)?;
            Ok((
                "editable_table",
                serde_json::json!({
                    "title": title,
                    "entity": entity,
                    "columns": columns,
                    "item_types": resolved,
                    "field_html": field_html_resolved,
                }),
            ))
        }
        ComponentDef::Chart {
            title,
            query,
            chart_type,
            x,
            y,
            ..
        } => {
            if !known_query(query) {
                anyhow::bail!("{owner_label} chart '{title}' references unknown query '{query}'");
            }
            Ok((
                "chart",
                serde_json::json!({
                    "title": title,
                    "query": query,
                    "chart_type": chart_type,
                    "x": x,
                    "y": y,
                }),
            ))
        }
        ComponentDef::Text { text, .. } => Ok(("text", serde_json::json!({ "text": text }))),
        ComponentDef::Link { label, target_page, .. } => {
            if !page_ids.contains_key(target_page) {
                anyhow::bail!("{owner_label} links to unknown page '{target_page}'");
            }
            Ok(("link", serde_json::json!({ "label": label, "target_page": target_page })))
        }
        ComponentDef::Region { label, query, columns, .. } => {
            if !known_query(query) {
                anyhow::bail!("{owner_label} region '{label}' references unknown query '{query}'");
            }
            Ok(("region", serde_json::json!({ "label": label, "query": query, "columns": columns })))
        }
        ComponentDef::Action { label, name, config, .. } => {
            if !action_registry.contains_key(name.as_str()) {
                let known: Vec<&str> = action_registry.keys().copied().collect();
                anyhow::bail!(
                    "{owner_label} action '{label}' calls unknown module '{name}' (known: {})",
                    known.join(", ")
                );
            }
            Ok((
                "action",
                serde_json::json!({ "label": label, "name": name, "config": config }),
            ))
        }
        ComponentDef::DynamicContent { label, name, config, .. } => {
            if !action_registry.contains_key(name.as_str()) {
                let known: Vec<&str> = action_registry.keys().copied().collect();
                anyhow::bail!(
                    "{owner_label} dynamic_content '{label}' calls unknown module '{name}' (known: {})",
                    known.join(", ")
                );
            }
            Ok((
                "dynamic_content",
                serde_json::json!({ "label": label, "name": name, "config": config }),
            ))
        }
        ComponentDef::Button { label, behavior, .. } => match behavior {
            crate::model::ButtonBehavior::Redirect { target_page, extra_params } => {
                if !page_ids.contains_key(target_page) {
                    anyhow::bail!("{owner_label} button '{label}' redirects to unknown page '{target_page}'");
                }
                Ok((
                    "button",
                    serde_json::json!({
                        "label": label,
                        "behavior": "redirect",
                        "target_page": target_page,
                        "extra_params": extra_params,
                    }),
                ))
            }
            crate::model::ButtonBehavior::RunAction { name, config } => {
                if !action_registry.contains_key(name.as_str()) {
                    let known: Vec<&str> = action_registry.keys().copied().collect();
                    anyhow::bail!(
                        "{owner_label} button '{label}' calls unknown module '{name}' (known: {})",
                        known.join(", ")
                    );
                }
                Ok((
                    "button",
                    serde_json::json!({
                        "label": label,
                        "behavior": "run_action",
                        "name": name,
                        "config": config,
                    }),
                ))
            }
        },
        ComponentDef::Calendar {
            title,
            entity,
            date_field,
            title_field,
            link_page,
            ..
        } => {
            if !entity_ids.contains_key(entity) {
                anyhow::bail!("{owner_label} calendar '{title}' references unknown entity '{entity}'");
            }
            let entity_def = app.entity(entity).expect("checked above");
            if entity_def.source_query.is_some() {
                anyhow::bail!(
                    "{owner_label} calendar '{title}' binds to entity '{entity}', which is \
                     query-backed and read-only — calendars need a real table"
                );
            }
            if entity_def.source_collection.is_some() {
                anyhow::bail!(
                    "{owner_label} calendar '{title}' binds to entity '{entity}', which is \
                     collection-backed and read-only — calendars need a real table"
                );
            }
            if !entity_def.fields.iter().any(|f| &f.name == date_field) {
                anyhow::bail!(
                    "{owner_label} calendar '{title}' date field '{date_field}' is not a field of '{entity}'"
                );
            }
            if !entity_def.fields.iter().any(|f| &f.name == title_field) {
                anyhow::bail!(
                    "{owner_label} calendar '{title}' title field '{title_field}' is not a field of '{entity}'"
                );
            }
            if let Some(p) = link_page {
                if !page_ids.contains_key(p) {
                    anyhow::bail!("{owner_label} calendar '{title}' links to unknown page '{p}'");
                }
            }
            Ok((
                "calendar",
                serde_json::json!({
                    "title": title,
                    "entity": entity,
                    "date_field": date_field,
                    "title_field": title_field,
                    "link_page": link_page,
                }),
            ))
        }
        ComponentDef::Map {
            title,
            entity,
            lat_field,
            lng_field,
            title_field,
            link_page,
            ..
        } => {
            if !entity_ids.contains_key(entity) {
                anyhow::bail!("{owner_label} map '{title}' references unknown entity '{entity}'");
            }
            let entity_def = app.entity(entity).expect("checked above");
            if entity_def.source_query.is_some() {
                anyhow::bail!(
                    "{owner_label} map '{title}' binds to entity '{entity}', which is \
                     query-backed and read-only — maps need a real table"
                );
            }
            if entity_def.source_collection.is_some() {
                anyhow::bail!(
                    "{owner_label} map '{title}' binds to entity '{entity}', which is \
                     collection-backed and read-only — maps need a real table"
                );
            }
            for (label, field) in [("lat", lat_field), ("lng", lng_field), ("title", title_field)] {
                if !entity_def.fields.iter().any(|f| &f.name == field) {
                    anyhow::bail!("{owner_label} map '{title}' {label} field '{field}' is not a field of '{entity}'");
                }
            }
            if let Some(p) = link_page {
                if !page_ids.contains_key(p) {
                    anyhow::bail!("{owner_label} map '{title}' links to unknown page '{p}'");
                }
            }
            Ok((
                "map",
                serde_json::json!({
                    "title": title,
                    "entity": entity,
                    "lat_field": lat_field,
                    "lng_field": lng_field,
                    "title_field": title_field,
                    "link_page": link_page,
                }),
            ))
        }
        ComponentDef::DynamicAction { event, item, ops } => {
            let ops_json: Vec<serde_json::Value> = ops.iter().map(|op| op.to_json()).collect();
            for op in ops {
                if let crate::model::DaOp::Refresh(query) = op {
                    if !known_query(query) {
                        anyhow::bail!(
                            "{owner_label} dynamic action on '{item}' refreshes unknown query '{query}'"
                        );
                    }
                }
                if let crate::model::DaOp::Call { action, .. } = op {
                    if !action_registry.contains_key(action.as_str()) {
                        let known: Vec<&str> = action_registry.keys().copied().collect();
                        anyhow::bail!(
                            "{owner_label} dynamic action on '{item}' calls unknown module '{action}' (known: {})",
                            known.join(", ")
                        );
                    }
                }
            }
            Ok((
                "dynamic_action",
                serde_json::json!({ "event": event, "item": item, "ops": ops_json }),
            ))
        }
    }
}

async fn ensure_data_table(pool: &PgPool, data_schema: &str, table_name: &str, entity: &EntityDef) -> Result<()> {
    let cols: Vec<String> = entity.fields.iter().map(|f| column_def(f, true)).collect();

    let sql = format!(
        "create table if not exists {data_schema}.{table_name} ({})",
        cols.join(", ")
    );
    sqlx::raw_sql(&sql)
        .execute(pool)
        .await
        .with_context(|| format!("failed to create data table {data_schema}.{table_name}"))?;

    // The table may already have existed before a field was added to
    // the entity — CREATE TABLE IF NOT EXISTS doesn't add columns to an
    // existing table, so bring existing tables up to date too. Skipping
    // NOT NULL here: enforcing it on a table that may already have rows
    // is a real migration (backfill or a default) that this vertical
    // slice doesn't attempt.
    for field in &entity.fields {
        if field.ty == FieldType::Id {
            continue; // the primary key only ever comes from CREATE TABLE
        }
        let alter_sql = format!(
            "alter table {data_schema}.{table_name} add column if not exists {}",
            column_def(field, false)
        );
        sqlx::raw_sql(&alter_sql)
            .execute(pool)
            .await
            .with_context(|| {
                format!("failed to add column '{}' to {data_schema}.{table_name}", field.name)
            })?;
    }

    Ok(())
}

/// The deployment check: after `ensure_data_table`, compare the
/// physical table's actual columns (via information_schema) against the
/// declared fields. A column with a *different type* than declared is a
/// hard startup error — the metadata would promise casts the table
/// can't honor, failing confusingly at request time instead. Columns
/// present in the table but absent from the entity (removed fields,
/// manual additions) are only warned about: they hold real data pgapp
/// shouldn't judge.
async fn verify_data_table(pool: &PgPool, data_schema: &str, table_name: &str, entity: &EntityDef) -> Result<()> {
    let actual: Vec<(String, String)> = sqlx::query_as(
        "select column_name, udt_name from information_schema.columns
          where table_schema = $1 and table_name = $2",
    )
    .bind(data_schema)
    .bind(table_name)
    .fetch_all(pool)
    .await?;
    let actual: HashMap<String, String> = actual.into_iter().collect();

    let expected_udt = |ty: FieldType| match ty {
        FieldType::Id | FieldType::Integer => "int4",
        FieldType::Text => "text",
        FieldType::Boolean => "bool",
        FieldType::Timestamp => "timestamptz",
    };

    let mut mismatches = Vec::new();
    for field in &entity.fields {
        match actual.get(&field.name) {
            None => mismatches.push(format!(
                "column '{}' is missing from {data_schema}.{table_name}",
                field.name
            )),
            Some(udt) if udt != expected_udt(field.ty) => mismatches.push(format!(
                "column '{}' is {udt} in {data_schema}.{table_name} but the entity declares {} (expected {})",
                field.name,
                field.ty.as_str(),
                expected_udt(field.ty),
            )),
            Some(_) => {}
        }
    }
    if !mismatches.is_empty() {
        anyhow::bail!(
            "entity '{}' does not match its existing table:\n  - {}\n\
             Fix the markup to match the table, or migrate the table \
             (pgapp adds columns but never changes or drops them).",
            entity.name,
            mismatches.join("\n  - "),
        );
    }

    for column in actual.keys() {
        if entity.fields.iter().all(|f| &f.name != column) {
            println!(
                "pgapp: warning: {data_schema}.{table_name} has column '{column}' that entity '{}' \
                 no longer declares (left untouched)",
                entity.name
            );
        }
    }

    Ok(())
}

fn column_def(field: &FieldDef, include_not_null: bool) -> String {
    let mut col = format!("{} {}", field.name, field.ty.sql_column_type());
    if field.ty != FieldType::Id {
        if include_not_null && field.required {
            col.push_str(" not null");
        }
        if let Some(default) = &field.default {
            match field.ty {
                FieldType::Boolean => col.push_str(&format!(" default {default}")),
                FieldType::Timestamp if default == "now" => col.push_str(" default now()"),
                FieldType::Integer => col.push_str(&format!(" default {default}")),
                _ => col.push_str(&format!(" default '{default}'")),
            }
        }
    }
    col
}
