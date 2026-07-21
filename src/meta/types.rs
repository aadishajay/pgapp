//! The runtime model: what `load_app` reloads from `pgapp_meta` and
//! `server.rs`/`render.rs` actually work with. Nothing here is built
//! directly from the parsed markup (`model::AppDef`) — it's rebuilt
//! from the database every time the process starts, which is the whole
//! point of "the database is the source of truth."

use std::collections::{BTreeMap, HashMap};

use crate::model::{AggregateFn, ComputedColumn, FieldItem, FormatMask, HighlightRule, HtmlAttrs, PreAction};

/// A named query, compiled at load time: `sql` already uses positional
/// `$N::TYPE` parameters, and `bind_names[i]` is the bind context key
/// that fills `$(i + 1)`. `TYPE` isn't hardcoded to `text` — see
/// `compile_named_query` — it's whatever Postgres itself infers the
/// bind's type to be from the query's own WHERE/comparison context,
/// asked fresh via `Describe` every time the app loads (startup, or
/// `/admin/reload`), so it can never go stale the way a hand-written
/// cast can when a column's type changes underneath it.
#[derive(Debug, Clone)]
pub struct RuntimeQuery {
    pub sql: String,
    pub bind_names: Vec<String>,
}

/// The exact shape a named query's SQL is always run inside of (see
/// `server::query_engine::run_named_query`) — wrapping in `to_jsonb`
/// is what lets the generic layer decode any result shape, regardless
/// of what columns a query selects or what Postgres types they are.
/// `compile_named_query` asks Postgres to describe this same wrapped
/// shape (not the bare inner SQL) so its bind-type inference matches
/// reality exactly: the wrapper is just a projection and never changes
/// how a placeholder's type is resolved, but keeping both call sites
/// on one function means they can't quietly drift apart if this shape
/// ever changes.
pub fn wrap_to_jsonb(sql: &str) -> String {
    format!("select to_jsonb(t) as j from ({sql}) as t")
}

/// Runtime view of a field, as reloaded from `pgapp_meta` (not from the
/// markup file) — this is what the server uses to build SQL.
#[derive(Debug, Clone)]
pub struct RuntimeField {
    pub name: String,
    pub data_type: crate::model::FieldType,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeEntity {
    pub name: String,
    pub table_name: String,
    pub fields: Vec<RuntimeField>,
    /// Non-null = read-only, query-backed entity (no physical table);
    /// see `model::EntityDef::source_query`.
    pub source_query: Option<String>,
    /// Non-null = read-only, collection-backed entity; see
    /// `model::EntityDef::source_collection`.
    pub source_collection: Option<String>,
}

impl RuntimeEntity {
    pub fn field(&self, name: &str) -> Option<&RuntimeField> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// `{id, class, attrs: [[key, value], ...]}` — the App Builder's
/// structured component editor's wire format for [`HtmlAttrs`] (see
/// `RuntimeComponent::to_json`). An array of pairs rather than an
/// object for `attrs` since it's edited as an ordered, repeatable list
/// of rows client-side, same reasoning as `formats`/`item_types` below.
fn html_attrs_json(html: &HtmlAttrs) -> serde_json::Value {
    serde_json::json!({
        "id": html.id,
        "class": html.class,
        "attrs": html.attrs,
    })
}

#[derive(Debug, Clone)]
pub struct LinkColumn {
    pub field: String,
    pub target_page: String,
    pub extra_params: Vec<(String, String)>,
}

/// Runtime counterpart of [`crate::model::ButtonBehavior`].
#[derive(Debug, Clone)]
pub enum ButtonBehavior {
    Redirect { target_page: String, extra_params: Vec<(String, String)> },
    RunAction { name: String, config: serde_json::Value },
}

/// One independently-rendered piece of a page (or of the app-wide
/// header/footer chrome) — the runtime counterpart of
/// [`crate::model::ComponentDef`], with entity names already resolved
/// to the full [`RuntimeEntity`] they describe.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum RuntimeComponent {
    Report {
        title: String,
        entity: RuntimeEntity,
        columns: Vec<String>,
        source_query: Option<String>,
        link_column: Option<LinkColumn>,
        page_size: i64,
        /// Runs automatically, server-side, immediately before this
        /// report fetches its rows on every request — see
        /// `model::PreAction`.
        before_load: Option<PreAction>,
        /// Extra read-only columns spliced into the entity-backed
        /// `SELECT` — see `model::ComputedColumn`. Empty when
        /// `source_query` is set.
        computed: Vec<ComputedColumn>,
        /// Display formatting applied to a column's raw text value at
        /// render time — see `model::FormatMask`.
        formats: HashMap<String, FormatMask>,
        /// Interactive Report's per-column footer aggregates — see
        /// `model::AggregateFn`.
        aggregates: HashMap<String, AggregateFn>,
        /// Interactive Report's Control Break column — see
        /// `model::ComponentDef::Report::break_on`.
        break_on: Option<String>,
        /// Interactive Report's row highlight rules — see
        /// `model::HighlightRule`.
        highlights: Vec<HighlightRule>,
        /// One of `model::REPORT_DISPLAY_MODES` — `"table"` (default),
        /// `"cards"`, or `"list"`.
        display: String,
        /// Role (or auth scheme name) required to see/write through
        /// this one component, on top of whatever the page itself
        /// requires — see `server::auth::authorize`.
        requires: Option<String>,
        html: HtmlAttrs,
    },
    Form {
        title: String,
        entity: RuntimeEntity,
        fields: Vec<String>,
        item_types: HashMap<String, FieldItem>,
        field_html: HashMap<String, HtmlAttrs>,
        requires: Option<String>,
        html: HtmlAttrs,
    },
    EditableTable {
        title: String,
        entity: RuntimeEntity,
        columns: Vec<String>,
        item_types: HashMap<String, FieldItem>,
        field_html: HashMap<String, HtmlAttrs>,
        requires: Option<String>,
        html: HtmlAttrs,
    },
    Chart {
        title: String,
        query: String,
        chart_type: String,
        x: String,
        y: String,
        requires: Option<String>,
        html: HtmlAttrs,
    },
    Text {
        text: String,
        requires: Option<String>,
        html: HtmlAttrs,
    },
    Link {
        label: String,
        target_page: String,
        requires: Option<String>,
        html: HtmlAttrs,
    },
    /// `columns` narrows the displayed columns to a subset of the
    /// query's result columns, in the given order; empty means "show
    /// every column the query returns."
    Region {
        label: String,
        query: String,
        columns: Vec<String>,
        requires: Option<String>,
        html: HtmlAttrs,
    },
    /// Oracle APEX's "PL/SQL Dynamic Content" region — see
    /// `model::ComponentDef::DynamicContent`.
    DynamicContent {
        label: String,
        name: String,
        config: serde_json::Value,
        requires: Option<String>,
        html: HtmlAttrs,
    },
    /// A button running a registered server-side action module.
    Action {
        label: String,
        name: String,
        config: serde_json::Value,
        requires: Option<String>,
        html: HtmlAttrs,
    },
    /// A standalone button — see `model::ComponentDef::Button`.
    Button {
        label: String,
        behavior: ButtonBehavior,
        requires: Option<String>,
        html: HtmlAttrs,
    },
    /// A client-side dynamic action; `config` is the full
    /// `{event, item, ops}` blob, emitted verbatim into the page's
    /// dynamic-actions JSON for the runtime.js dispatcher. No `html` —
    /// there's no wrapper tag to attach it to.
    DynamicAction {
        config: serde_json::Value,
    },
    /// Oracle APEX's Calendar region — see `model::ComponentDef::Calendar`.
    Calendar {
        title: String,
        entity: RuntimeEntity,
        date_field: String,
        title_field: String,
        link_page: Option<String>,
        requires: Option<String>,
        html: HtmlAttrs,
    },
    /// Oracle APEX's Map region — see `model::ComponentDef::Map`.
    Map {
        title: String,
        entity: RuntimeEntity,
        lat_field: String,
        lng_field: String,
        title_field: String,
        link_page: Option<String>,
        requires: Option<String>,
        html: HtmlAttrs,
    },
}

impl RuntimeComponent {
    /// This component's kind name, exactly as the markup grammar spells
    /// it (`report`, `form`, `editable_table`, `chart`, `text`, `link`,
    /// `region`, `action`, `button`, `dynamic_action`) — the App
    /// Builder structured editor's discriminant, both for picking which
    /// client-side form-spec to render and (echoed back on submit) for
    /// server.rs to know which kind's markup generator ran.
    pub fn kind(&self) -> &'static str {
        match self {
            RuntimeComponent::Report { .. } => "report",
            RuntimeComponent::Form { .. } => "form",
            RuntimeComponent::EditableTable { .. } => "editable_table",
            RuntimeComponent::Chart { .. } => "chart",
            RuntimeComponent::Text { .. } => "text",
            RuntimeComponent::Link { .. } => "link",
            RuntimeComponent::Region { .. } => "region",
            RuntimeComponent::DynamicContent { .. } => "dynamic_content",
            RuntimeComponent::Action { .. } => "action",
            RuntimeComponent::Button { .. } => "button",
            RuntimeComponent::DynamicAction { .. } => "dynamic_action",
            RuntimeComponent::Calendar { .. } => "calendar",
            RuntimeComponent::Map { .. } => "map",
        }
    }

    /// Serializes this component's full, already-resolved attribute set
    /// to JSON — the App Builder's structured editor fetches this to
    /// prefill a real per-kind edit form (see the new
    /// `/:workspace/:app/admin/pages/:page/components/:idx/structured`
    /// route in `server.rs`), instead of the raw markup text
    /// `page_reorder::component_source` returns. A `HashMap` field
    /// (`formats`, `item_types`, `field_html`) becomes a name-sorted
    /// array of rows — deterministic across calls, and the natural
    /// shape for a client-side repeatable-row editor. The client-side
    /// JS never needs to *parse* this back into markup text for a
    /// round trip — it only ever *generates* fresh markup text from
    /// whatever the form currently holds (see runtime.js's per-kind
    /// `pgappGenerate*` functions) and submits that verbatim through
    /// the same raw-text `/components/.../add`|`edit` routes the
    /// existing raw editor already uses; this method only has to feed
    /// the initial prefill, never round-trip losslessly on its own.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            RuntimeComponent::Report {
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
                requires,
                html,
            } => serde_json::json!({
                "title": title,
                "entity": entity.name,
                "entity_fields": entity_fields_json(entity),
                "columns": columns,
                "source_query": source_query,
                "link_column": link_column.as_ref().map(|l| serde_json::json!({
                    "field": l.field,
                    "target_page": l.target_page,
                    "extra_params": l.extra_params,
                })),
                "page_size": page_size,
                "before_load": before_load.as_ref().map(|a| serde_json::json!({"name": a.name, "config": a.config})),
                "computed": computed.iter().map(|c| serde_json::json!({"name": c.name, "sql": c.sql})).collect::<Vec<_>>(),
                "formats": formats_json(formats),
                "aggregates": aggregates_json(aggregates),
                "break_on": break_on,
                "highlights": highlights.iter().map(|h| serde_json::json!({"when": h.when, "color": h.color})).collect::<Vec<_>>(),
                "display": display,
                "requires": requires,
                "html": html_attrs_json(html),
            }),
            RuntimeComponent::Form { title, entity, fields, item_types, field_html, requires, html } => serde_json::json!({
                "title": title,
                "entity": entity.name,
                "entity_fields": entity_fields_json(entity),
                "fields": fields,
                "item_types": item_types_json(item_types),
                "field_html": field_html_json(field_html),
                "requires": requires,
                "html": html_attrs_json(html),
            }),
            RuntimeComponent::EditableTable { title, entity, columns, item_types, field_html, requires, html } => serde_json::json!({
                "title": title,
                "entity": entity.name,
                "entity_fields": entity_fields_json(entity),
                "columns": columns,
                "item_types": item_types_json(item_types),
                "field_html": field_html_json(field_html),
                "requires": requires,
                "html": html_attrs_json(html),
            }),
            RuntimeComponent::Chart { title, query, chart_type, x, y, requires, html } => serde_json::json!({
                "title": title,
                "query": query,
                "chart_type": chart_type,
                "x": x,
                "y": y,
                "requires": requires,
                "html": html_attrs_json(html),
            }),
            RuntimeComponent::Text { text, requires, html } => serde_json::json!({
                "text": text,
                "requires": requires,
                "html": html_attrs_json(html),
            }),
            RuntimeComponent::Link { label, target_page, requires, html } => serde_json::json!({
                "label": label,
                "target_page": target_page,
                "requires": requires,
                "html": html_attrs_json(html),
            }),
            RuntimeComponent::Region { label, query, columns, requires, html } => serde_json::json!({
                "label": label,
                "query": query,
                "columns": columns,
                "requires": requires,
                "html": html_attrs_json(html),
            }),
            RuntimeComponent::DynamicContent { label, name, config, requires, html } => serde_json::json!({
                "label": label,
                "name": name,
                "config": config,
                "requires": requires,
                "html": html_attrs_json(html),
            }),
            RuntimeComponent::Action { label, name, config, requires, html } => serde_json::json!({
                "label": label,
                "name": name,
                "config": config,
                "requires": requires,
                "html": html_attrs_json(html),
            }),
            RuntimeComponent::Button { label, behavior, requires, html } => serde_json::json!({
                "label": label,
                "behavior": match behavior {
                    ButtonBehavior::Redirect { target_page, extra_params } => serde_json::json!({
                        "type": "redirect",
                        "target_page": target_page,
                        "extra_params": extra_params,
                    }),
                    ButtonBehavior::RunAction { name, config } => serde_json::json!({
                        "type": "run_action",
                        "name": name,
                        "config": config,
                    }),
                },
                "requires": requires,
                "html": html_attrs_json(html),
            }),
            RuntimeComponent::DynamicAction { config } => config.clone(),
            RuntimeComponent::Calendar {
                title,
                entity,
                date_field,
                title_field,
                link_page,
                requires,
                html,
            } => serde_json::json!({
                "title": title,
                "entity": entity.name,
                "entity_fields": entity_fields_json(entity),
                "date_field": date_field,
                "title_field": title_field,
                "link_page": link_page,
                "requires": requires,
                "html": html_attrs_json(html),
            }),
            RuntimeComponent::Map {
                title,
                entity,
                lat_field,
                lng_field,
                title_field,
                link_page,
                requires,
                html,
            } => serde_json::json!({
                "title": title,
                "entity": entity.name,
                "entity_fields": entity_fields_json(entity),
                "lat_field": lat_field,
                "lng_field": lng_field,
                "title_field": title_field,
                "link_page": link_page,
                "requires": requires,
                "html": html_attrs_json(html),
            }),
        }
    }
}

/// An entity's fields as `[{name, type}, ...]` — `type` is
/// `FieldType::as_str()` (`"id"`, `"text"`, `"boolean"`, `"integer"`,
/// `"timestamp"`), which the App Builder's structured editor needs to
/// compute each field's default item-type kind client-side (mirroring
/// `item_types::default_kind_for`'s tiny fixed mapping) — that's how it
/// knows whether an `item_types` row it's about to regenerate as
/// markup is actually redundant (kind == that field's own default, and
/// empty config) and can be omitted rather than emitting a needless
/// explicit `item <field> as <kind>` line for every single field.
fn entity_fields_json(entity: &RuntimeEntity) -> serde_json::Value {
    serde_json::Value::Array(
        entity
            .fields
            .iter()
            .map(|f| serde_json::json!({"name": f.name, "type": f.data_type.as_str()}))
            .collect(),
    )
}

/// `formats`'s wire format: a field-name-sorted array of `{field,
/// mask}` rows (see `to_json`'s doc for why a map becomes a sorted
/// array).
fn formats_json(formats: &HashMap<String, FormatMask>) -> serde_json::Value {
    let mut names: Vec<&String> = formats.keys().collect();
    names.sort();
    serde_json::Value::Array(
        names
            .into_iter()
            .map(|name| serde_json::json!({"field": name, "mask": formats[name].to_json()}))
            .collect(),
    )
}

fn aggregates_json(aggregates: &HashMap<String, AggregateFn>) -> serde_json::Value {
    let mut names: Vec<&String> = aggregates.keys().collect();
    names.sort();
    serde_json::Value::Array(
        names
            .into_iter()
            .map(|name| serde_json::json!({"field": name, "fn": aggregates[name].as_str()}))
            .collect(),
    )
}

/// `item_types`'s wire format: a field-name-sorted array of `{field,
/// kind, config}` rows (see `to_json`'s doc for why a map becomes a
/// sorted array).
fn item_types_json(item_types: &HashMap<String, FieldItem>) -> serde_json::Value {
    let mut names: Vec<&String> = item_types.keys().collect();
    names.sort();
    serde_json::Value::Array(
        names
            .into_iter()
            .map(|name| {
                let item = &item_types[name];
                serde_json::json!({"field": name, "kind": item.kind, "config": item.config})
            })
            .collect(),
    )
}

/// `field_html`'s wire format: a field-name-sorted array of `{field,
/// html}` rows.
fn field_html_json(field_html: &HashMap<String, HtmlAttrs>) -> serde_json::Value {
    let mut names: Vec<&String> = field_html.keys().collect();
    names.sort();
    serde_json::Value::Array(
        names
            .into_iter()
            .map(|name| serde_json::json!({"field": name, "html": html_attrs_json(&field_html[name])}))
            .collect(),
    )
}

#[derive(Debug, Clone)]
pub struct RuntimePage {
    pub name: String,
    pub components: Vec<RuntimeComponent>,
    /// Queries visible only on this page.
    pub queries: HashMap<String, RuntimeQuery>,
    /// Role a signed-in user must hold to see or write through this
    /// page ('admin' always passes); None = any signed-in user. Only
    /// consulted when the app has auth enabled.
    pub required_role: Option<String>,
}

impl RuntimePage {
    /// Looks up a query by name, preferring one scoped to this page over
    /// an app-scoped query of the same name.
    pub fn resolve_query<'a>(&'a self, app: &'a RuntimeApp, name: &str) -> Option<&'a RuntimeQuery> {
        self.queries.get(name).or_else(|| app.queries.get(name))
    }
}

/// One node in the reloaded nav tree; see [`crate::model::NavItem`] for
/// the markup-level equivalent.
#[derive(Debug, Clone)]
pub struct NavNode {
    pub label: String,
    pub target_page: Option<String>,
    pub children: Vec<NavNode>,
}

#[derive(Debug, Clone)]
pub struct RuntimeApp {
    /// The app's row id in `pgapp_meta.apps` — users and sessions are
    /// keyed by it.
    pub id: i32,
    pub name: String,
    /// Which schema this app's physical data tables live in — its
    /// workspace's own schema (see `src/control.rs`). Any bare
    /// unqualified table reference in `server.rs`'s generated SQL
    /// resolves against this via `search_path`.
    pub data_schema: String,
    /// App settings reloaded from `pgapp_meta.apps`, originally declared
    /// in the markup (`theme:` / `icons:` / `chart_lib:` / `auth { }`).
    pub theme: Option<String>,
    pub icons: Option<String>,
    pub chart_lib: Option<String>,
    pub auth_enabled: bool,
    pub pages: Vec<RuntimePage>,
    pub nav: Vec<NavNode>,
    pub header: Vec<RuntimeComponent>,
    pub footer: Vec<RuntimeComponent>,
    /// Queries visible from every page.
    pub queries: HashMap<String, RuntimeQuery>,
    /// Named role groups (`auth_scheme "name" { roles: ... }`), keyed by
    /// name, for `server::auth::authorize` to expand a `requires:` name
    /// through before falling back to treating it as a literal role.
    pub schemes: HashMap<String, Vec<String>>,
    /// This app's row id in `pgapp_control.apps` — a different table
    /// (and a different id) from `id` above (`pgapp_meta.apps`, rebuilt
    /// by every markup resync). `load_app` doesn't know this; it's set
    /// afterward by whoever's loading the app (`main.rs`'s
    /// `load_one_app`, `server::AppEntry::reload`) from the control-
    /// plane registry, and only exists to scope a `{{secret...}}`
    /// lookup (`secrets::resolve`) — 0 if never set.
    pub control_app_id: i32,
    /// This app's workspace, if any (`pgapp_control.apps.workspace_id`)
    /// — same "set after the fact" story as `control_app_id`, and the
    /// same use: a workspace-scoped secret is the fallback when no
    /// app-scoped secret of that name exists.
    pub workspace_id: Option<i32>,
}

impl RuntimeApp {
    pub fn page(&self, name: &str) -> Option<&RuntimePage> {
        self.pages.iter().find(|p| p.name == name)
    }

    /// Everything site-wide that renderers need alongside a single
    /// page: the nav tree, header/footer chrome, and the resolved rows
    /// for every `Region` component anywhere on the current request
    /// (the page's own components plus the header/footer), keyed by
    /// query name.
    pub fn chrome<'a>(&'a self, regions: &'a RegionRows) -> Chrome<'a> {
        Chrome {
            nav: &self.nav,
            header: &self.header,
            footer: &self.footer,
            regions,
        }
    }
}

/// Rows already fetched for each `Region` component that appears on the
/// current request, keyed by query name. Resolving these is an async DB
/// round trip, so `server.rs` does it up front and hands the result to
/// the (synchronous) render functions.
pub type RegionRows = HashMap<String, Vec<BTreeMap<String, Option<String>>>>;

/// Borrowed bundle of the app-wide chrome (nav/header/footer/regions),
/// passed into every page render function instead of four separate
/// parameters.
#[derive(Debug, Clone, Copy)]
pub struct Chrome<'a> {
    pub nav: &'a [NavNode],
    pub header: &'a [RuntimeComponent],
    pub footer: &'a [RuntimeComponent],
    pub regions: &'a RegionRows,
}
