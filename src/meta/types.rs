//! The runtime model: what `load_app` reloads from `pgapp_meta` and
//! `server.rs`/`render.rs` actually work with. Nothing here is built
//! directly from the parsed markup (`model::AppDef`) — it's rebuilt
//! from the database every time the process starts, which is the whole
//! point of "the database is the source of truth."

use std::collections::{BTreeMap, HashMap};

use crate::model::{ComputedColumn, FieldItem, FormatMask, HtmlAttrs, PreAction};

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
        html: HtmlAttrs,
    },
    Form {
        title: String,
        entity: RuntimeEntity,
        fields: Vec<String>,
        item_types: HashMap<String, FieldItem>,
        field_html: HashMap<String, HtmlAttrs>,
        html: HtmlAttrs,
    },
    EditableTable {
        title: String,
        entity: RuntimeEntity,
        columns: Vec<String>,
        item_types: HashMap<String, FieldItem>,
        field_html: HashMap<String, HtmlAttrs>,
        html: HtmlAttrs,
    },
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
    /// `columns` narrows the displayed columns to a subset of the
    /// query's result columns, in the given order; empty means "show
    /// every column the query returns."
    Region {
        label: String,
        query: String,
        columns: Vec<String>,
        html: HtmlAttrs,
    },
    /// A button running a registered server-side action module.
    Action {
        label: String,
        name: String,
        config: serde_json::Value,
        html: HtmlAttrs,
    },
    /// A standalone button — see `model::ComponentDef::Button`.
    Button {
        label: String,
        behavior: ButtonBehavior,
        html: HtmlAttrs,
    },
    /// A client-side dynamic action; `config` is the full
    /// `{event, item, ops}` blob, emitted verbatim into the page's
    /// dynamic-actions JSON for the runtime.js dispatcher. No `html` —
    /// there's no wrapper tag to attach it to.
    DynamicAction {
        config: serde_json::Value,
    },
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
