//! The runtime model: what `load_app` reloads from `pgapp_meta` and
//! `server.rs`/`render.rs` actually work with. Nothing here is built
//! directly from the parsed markup (`model::AppDef`) — it's rebuilt
//! from the database every time the process starts, which is the whole
//! point of "the database is the source of truth."

use std::collections::{BTreeMap, HashMap};

use crate::model::FieldItem;

/// A named query, compiled once at load time: `sql` already uses
/// positional `$N::text` parameters, and `bind_names[i]` is the bind
/// context key that fills `$(i + 1)`.
#[derive(Debug, Clone)]
pub struct RuntimeQuery {
    pub sql: String,
    pub bind_names: Vec<String>,
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
    },
    Form {
        title: String,
        entity: RuntimeEntity,
        fields: Vec<String>,
        item_types: HashMap<String, FieldItem>,
    },
    EditableTable {
        title: String,
        entity: RuntimeEntity,
        columns: Vec<String>,
        item_types: HashMap<String, FieldItem>,
    },
    Chart {
        title: String,
        query: String,
        chart_type: String,
        x: String,
        y: String,
    },
    Text(String),
    Link {
        label: String,
        target_page: String,
    },
    Region {
        label: String,
        query: String,
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
