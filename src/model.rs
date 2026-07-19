use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Id,
    Text,
    Boolean,
    Integer,
    Timestamp,
}

impl FieldType {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "id" => Some(FieldType::Id),
            "text" => Some(FieldType::Text),
            "boolean" => Some(FieldType::Boolean),
            "integer" => Some(FieldType::Integer),
            "timestamp" => Some(FieldType::Timestamp),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FieldType::Id => "id",
            FieldType::Text => "text",
            FieldType::Boolean => "boolean",
            FieldType::Integer => "integer",
            FieldType::Timestamp => "timestamp",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        Self::parse(s).unwrap_or(FieldType::Text)
    }

    /// Column type used when creating the physical data table.
    pub fn sql_column_type(&self) -> &'static str {
        match self {
            FieldType::Id => "serial primary key",
            FieldType::Text => "text",
            FieldType::Boolean => "boolean",
            FieldType::Integer => "integer",
            FieldType::Timestamp => "timestamptz",
        }
    }

    /// Cast used when binding a submitted text value to this column,
    /// and when reading it back out as text (see server.rs).
    pub fn sql_cast(&self) -> &'static str {
        match self {
            FieldType::Id => "integer",
            FieldType::Text => "text",
            FieldType::Boolean => "boolean",
            FieldType::Integer => "integer",
            FieldType::Timestamp => "timestamptz",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    pub ty: FieldType,
    pub required: bool,
    pub default: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EntityDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
    /// `entity "x" from query <name>` — a read-only entity backed by a
    /// named query instead of a physical table. No table is created, no
    /// Form/EditableTable may bind to it (checked at sync), and reports
    /// over it paginate by OFFSET since arbitrary SQL has no assumed
    /// sort key. The declared fields describe the query's columns for
    /// rendering/typing purposes.
    pub source_query: Option<String>,
    /// `entity "x" from collection "name"` — a read-only entity backed
    /// by `pgapp_meta.collections` (see db/schema.sql), scoped to the
    /// current caller so one visitor's rows are never visible to
    /// another's. Unlike `source_query`, the SQL isn't author-written:
    /// server.rs compiles it, always including the caller/name filter,
    /// so there's no WHERE clause an app author could omit or bypass.
    /// Mutually exclusive with `source_query` (checked at parse time).
    pub source_collection: Option<String>,
}

/// A named, reusable SQL query. Declared at app scope (visible to every
/// page) or nested inside one `page { }` block (visible only there,
/// shadowing an app-scoped query of the same name). `sql` may contain
/// `:name` bind markers — see `meta::compile_named_query` for how those
/// get turned into safe positional parameters.
#[derive(Debug, Clone)]
pub struct QueryDef {
    pub name: String,
    pub sql: String,
}

/// Turns a report's linked column into a link to another page, passing
/// the row's id as a `?id=` query parameter (the common "click a row to
/// see its detail page" pattern) plus, optionally, other columns from
/// the same row forwarded as additional named query parameters — this
/// is how a value on one page reaches a named query on another page.
#[derive(Debug, Clone)]
pub struct LinkColumn {
    pub field: String,
    pub target_page: String,
    pub extra_params: Vec<(String, String)>,
}

/// How a form/editable-table field is presented: `kind` names a
/// registered component (see `src/item_types.rs`, e.g. "text", "radio",
/// "slider"), and `config` is whatever that component wants — a generic
/// JSON blob so new item types never need a change here or in the
/// markup grammar. Two config keys are reserved by convention (not
/// enforced here): `choices` (a fixed list, for Radio/Popup) and
/// `query` (a named query's rows instead — see
/// `server::query_engine::resolve_field_choices`).
#[derive(Debug, Clone)]
pub struct FieldItem {
    pub kind: String,
    pub config: serde_json::Value,
}

/// The chart types the built-in `inline` SVG backend (and, by
/// convention, any pluggable `chart_lib`) knows how to render. Checked
/// at parse time in `markup::parse_chart` so an unsupported type is a
/// sync-time error, not a silently blank chart.
pub const CHART_TYPES: &[&str] = &["bar", "line", "area", "pie", "donut", "scatter"];

/// Optional `id`/`class`/extra-attribute overrides on a component,
/// parsed from a trailing `attrs (id: "...", class: "...", ...)` suffix
/// (see `markup::Parser::parse_html_attrs`) and spliced onto that
/// component's outer wrapper tag at render time (`render::merged_class`/
/// `render::extra_attrs`). `id`/`class` are reserved keys; any other
/// key becomes a plain HTML attribute, with `_` rewritten to `-` so
/// `data_foo: "bar"` renders as `data-foo="bar"` (the grammar's
/// identifiers can't contain hyphens directly).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HtmlAttrs {
    pub id: Option<String>,
    pub class: Option<String>,
    pub attrs: Vec<(String, String)>,
}

impl HtmlAttrs {
    pub fn is_empty(&self) -> bool {
        self.id.is_none() && self.class.is_none() && self.attrs.is_empty()
    }
}

/// One independently-rendered piece of a page, or of the app-wide
/// header/footer chrome (which reuses the same component kinds, though
/// in practice only `Text`/`Link`/`Region` make sense there — enforced
/// at sync time, see `meta::sync_app`).
///
/// A page is simply an ordered list of these — there is no longer a
/// fixed page "kind": a page can carry a `Report` and a `Form` side by
/// side (the classic list+edit CRUD pattern), an `EditableTable` on its
/// own, a dashboard of `Chart`s, or any other combination.
#[derive(Debug, Clone)]
pub enum ComponentDef {
    /// A read-only, paginated table. Rows come from the entity's data
    /// table by default, or from `source_query` when set. `link_column`
    /// makes one column a link to another page (forwarding the row's id
    /// plus any extra parameters). Edit/delete actions appear on each
    /// row automatically when the same page also has a `Form` bound to
    /// the same entity (see `server.rs`'s `sibling_form`).
    Report {
        title: String,
        entity: String,
        columns: Vec<String>,
        source_query: Option<String>,
        link_column: Option<LinkColumn>,
        page_size: i64,
        html: HtmlAttrs,
    },
    /// A create/edit form for one entity. Renders blank (create mode) by
    /// default; switches to edit mode for one row when the page is
    /// requested with `?edit_<n>=<id>` (`<n>` = this component's index
    /// on the page). `field_html` is the per-field counterpart to the
    /// component-level `html`: `item <field> attrs (...)` sets
    /// `id`/`class`/attributes on that one field's `<div class="pgapp-field">`
    /// wrapper, independent of (and combinable with) an `as <kind>` item
    /// type override.
    Form {
        title: String,
        entity: String,
        fields: Vec<String>,
        item_types: HashMap<String, FieldItem>,
        field_html: HashMap<String, HtmlAttrs>,
        html: HtmlAttrs,
    },
    /// Every row rendered inline-editable (one `<form>` per row), plus
    /// an "add new" row form — no separate list/edit split. `field_html`:
    /// see `Form`.
    EditableTable {
        title: String,
        entity: String,
        columns: Vec<String>,
        item_types: HashMap<String, FieldItem>,
        field_html: HashMap<String, HtmlAttrs>,
        html: HtmlAttrs,
    },
    /// Renders `query`'s rows as a chart; `chart_type` is one of
    /// `CHART_TYPES`, `x`/`y` name the columns used for each axis (for
    /// `pie`/`donut`, `x` is the slice label and `y` its value). See
    /// `src/chart_lib.rs` for the pluggable rendering backend.
    Chart {
        title: String,
        query: String,
        chart_type: String,
        x: String,
        y: String,
        html: HtmlAttrs,
    },
    Text {
        text: String,
        html: HtmlAttrs,
    },
    Link {
        label: String,
        target_page: String,
        html: HtmlAttrs,
    },
    /// Renders a named query's rows as a plain (non-paginated) table —
    /// sugar for a small, fixed-shape `Report` without entity/pagination
    /// machinery. `columns` narrows the displayed columns to a subset
    /// of the query's result columns, in the given order; empty means
    /// "show every column the query returns" (unlike `Report`, where an
    /// explicit list is always required).
    Region {
        label: String,
        query: String,
        columns: Vec<String>,
        html: HtmlAttrs,
    },
    /// A button that runs a server-side action module — a Rust component
    /// registered in `src/actions.rs` (pgapp's PL/SQL analog). `config`
    /// is the module's own generic blob, same pattern as item types.
    Action {
        label: String,
        name: String,
        config: serde_json::Value,
        html: HtmlAttrs,
    },
    /// A client-side dynamic action: `on <event> of <item> { ops }`.
    /// Not rendered as visible content — the page emits all of these as
    /// one JSON blob that the DB-stored runtime.js dispatcher binds, so
    /// (unlike every other variant) it carries no `html` — there's no
    /// wrapper tag to put attributes on.
    DynamicAction {
        event: String,
        item: String,
        ops: Vec<DaOp>,
    },
}

impl ComponentDef {
    /// Overwrites this component's `html` in place from a trailing
    /// `attrs (...)` suffix (see `markup::Parser::parse_component`). A
    /// no-op on `DynamicAction`, which has no wrapper tag to attach to.
    pub(crate) fn set_html(&mut self, new_html: HtmlAttrs) {
        match self {
            ComponentDef::Report { html, .. }
            | ComponentDef::Form { html, .. }
            | ComponentDef::EditableTable { html, .. }
            | ComponentDef::Chart { html, .. }
            | ComponentDef::Text { html, .. }
            | ComponentDef::Link { html, .. }
            | ComponentDef::Region { html, .. }
            | ComponentDef::Action { html, .. } => *html = new_html,
            ComponentDef::DynamicAction { .. } => {}
        }
    }
}

/// One operation inside a dynamic action.
#[derive(Debug, Clone)]
pub enum DaOp {
    Show(String),
    Hide(String),
    /// Show `item` when the JS expression `when` is truthy, hide it
    /// otherwise.
    Toggle { item: String, when: String },
    /// Set `item` to the result of evaluating the JS expression `expr`
    /// (which may call `pgapp.getItem(...)`).
    Set { item: String, expr: String },
    /// Re-fetch one region's rows (by query name), sending the page's
    /// current item values as bind parameters.
    Refresh(String),
}

impl DaOp {
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            DaOp::Show(item) => serde_json::json!({"op": "show", "item": item}),
            DaOp::Hide(item) => serde_json::json!({"op": "hide", "item": item}),
            DaOp::Toggle { item, when } => serde_json::json!({"op": "toggle", "item": item, "when": when}),
            DaOp::Set { item, expr } => serde_json::json!({"op": "set", "item": item, "expr": expr}),
            DaOp::Refresh(query) => serde_json::json!({"op": "refresh", "query": query}),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PageDef {
    pub name: String,
    pub components: Vec<ComponentDef>,
    /// Queries visible only within this page (in addition to the app's).
    pub queries: Vec<QueryDef>,
    /// `requires: <role>` — when the app has auth enabled, only users
    /// with this role (or 'admin', which passes every check) may see or
    /// write through this page. None = any signed-in user.
    pub required_role: Option<String>,
}

/// One entry in the app's (possibly multi-level) navigation bar. A leaf
/// links to a page; a group has children and no target of its own.
#[derive(Debug, Clone)]
pub struct NavItem {
    pub label: String,
    pub target_page: Option<String>,
    pub children: Vec<NavItem>,
}

#[derive(Debug, Clone)]
pub struct AppDef {
    pub name: String,
    /// App-level settings, declared in the markup file rather than the
    /// process environment: `theme: vivid`, `icons: fontawesome`,
    /// `chart_lib: canvas_bars`. None = the built-in default
    /// (shadcn / builtin / inline).
    pub theme: Option<String>,
    pub icons: Option<String>,
    pub chart_lib: Option<String>,
    /// `auth { }` — when true, every page requires a signed-in user
    /// (see `server::auth`), and pages may further restrict by role
    /// via `requires:`.
    pub auth: bool,
    pub entities: Vec<EntityDef>,
    pub pages: Vec<PageDef>,
    pub nav: Vec<NavItem>,
    /// Shown on every page, above the nav bar / below the footer
    /// respectively. Reuses `ComponentDef` — restricted in practice (and
    /// validated at sync time) to `Text`/`Link`/`Region`.
    pub header: Vec<ComponentDef>,
    pub footer: Vec<ComponentDef>,
    /// Queries visible from every page (a page-scoped query of the same
    /// name takes precedence).
    pub queries: Vec<QueryDef>,
}

impl AppDef {
    pub fn entity(&self, name: &str) -> Option<&EntityDef> {
        self.entities.iter().find(|e| e.name == name)
    }
}
