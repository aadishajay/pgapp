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

/// An action run automatically, server-side, immediately before a
/// component fetches its data — e.g. an `http_request` call that
/// refreshes a collection right before the report reading it renders,
/// so a page never shows stale data without someone manually clicking
/// a "refresh" button first. Reuses the same `ServerAction` registry
/// and `ActionContext` as a regular `action` component; only the
/// trigger differs (page load vs. a click). A failure here is
/// non-fatal — see `server::render_component`'s Report branch — the
/// component still renders with whatever data already exists, plus an
/// inline warning.
///
/// Hard invariant, by design: `before_load` only ever fires from the
/// read-only `GET /:workspace/:app/:page` render path (`server::show` →
/// `render_component` — its only caller). It must never be wired into
/// a POST/mutating handler (create/update/delete, `run_action`, a form
/// submission's own action, etc.) — those already have an explicit,
/// user-initiated trigger and don't need (or want) an implicit one
/// that fires on every innocuous page view, including from link
/// prefetchers and crawlers that only ever issue GET.
#[derive(Debug, Clone)]
pub struct PreAction {
    pub name: String,
    pub config: serde_json::Value,
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

/// What a `ComponentDef::Button` click actually does — see its doc for
/// why this is a separate variant from `Link`/`Action` rather than
/// reusing either directly.
#[derive(Debug, Clone)]
pub enum ButtonBehavior {
    /// `-> page <Name> (<param>: <source_field>, ...)`: navigates to
    /// another page, forwarding `<source_field>`'s *current* value —
    /// looked up in the page's own query-string context at render time,
    /// the same way `LinkColumn::extra_params` looks a row column up in
    /// the current report row — under the new `<param>` name. Empty
    /// `extra_params` is a plain redirect with no forwarded parameters.
    Redirect { target_page: String, extra_params: Vec<(String, String)> },
    /// `calls <name> (...)`: runs a registered server-side action
    /// module on click, identical to `ComponentDef::Action`.
    RunAction { name: String, config: serde_json::Value },
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

/// A read-only column a `Report` adds to its own entity-backed row
/// output — `sql` is a scalar SQL expression evaluated in the same
/// `SELECT` as the entity's own columns, aliased to `name` and cast to
/// text like every other column (see `server::select_columns`). It may
/// reference the current row via `t.<field>` (`t` is the entity table's
/// alias), including in a correlated subquery. Only meaningful on an
/// entity-backed report — a query-backed or collection-backed report
/// already runs arbitrary SQL/JSON, so add the expression to that query
/// directly instead.
#[derive(Debug, Clone)]
pub struct ComputedColumn {
    pub name: String,
    pub sql: String,
}

/// Display-only formatting for one report column, applied to the raw
/// text value at render time (`render::report_html`) — never touches
/// what's stored or what a Form submits. A value that doesn't parse the
/// way the mask expects (non-numeric text under `Currency`, say) is
/// rendered unchanged rather than erroring, since a report has no way to
/// refuse to display one bad row.
#[derive(Debug, Clone, PartialEq)]
pub enum FormatMask {
    /// `$1,234.56` — two decimals, thousands separator, `$` prefix.
    Currency,
    /// A fixed-point number with thousands separators and `decimals`
    /// digits after the point (`decimals: 0` renders a plain integer).
    Number { decimals: u32 },
    /// The raw number rounded to an integer with a trailing `%`.
    Percent,
    /// Reformats an ISO-ish `YYYY-MM-DD[ T]HH:MM:SS` text value using a
    /// strftime-like `pattern` (`%Y`, `%y`, `%m`, `%d`, `%B`, `%d` — see
    /// `format_date`). Defaults to `%Y-%m-%d` (a no-op) when unset.
    Date { pattern: String },
}

impl FormatMask {
    pub fn apply(&self, raw: &str) -> String {
        if raw.is_empty() {
            return String::new();
        }
        match self {
            FormatMask::Currency => match raw.parse::<f64>() {
                Ok(n) => {
                    let sign = if n < 0.0 { "-" } else { "" };
                    format!("{sign}${}", format_thousands(n.abs(), 2))
                }
                Err(_) => raw.to_string(),
            },
            FormatMask::Number { decimals } => match raw.parse::<f64>() {
                Ok(n) => format_thousands(n, *decimals),
                Err(_) => raw.to_string(),
            },
            FormatMask::Percent => match raw.parse::<f64>() {
                Ok(n) => format!("{}%", format_thousands(n, 0)),
                Err(_) => raw.to_string(),
            },
            FormatMask::Date { pattern } => format_date(raw, pattern).unwrap_or_else(|| raw.to_string()),
        }
    }

    /// The metadata-storage encoding of a mask — the inverse of
    /// `FormatMask::from_json`.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            FormatMask::Currency => serde_json::json!({"kind": "currency"}),
            FormatMask::Number { decimals } => serde_json::json!({"kind": "number", "decimals": decimals}),
            FormatMask::Percent => serde_json::json!({"kind": "percent"}),
            FormatMask::Date { pattern } => serde_json::json!({"kind": "date", "pattern": pattern}),
        }
    }

    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        match v.get("kind").and_then(|k| k.as_str())? {
            "currency" => Some(FormatMask::Currency),
            "number" => Some(FormatMask::Number {
                decimals: v.get("decimals").and_then(|d| d.as_u64()).unwrap_or(0) as u32,
            }),
            "percent" => Some(FormatMask::Percent),
            "date" => Some(FormatMask::Date {
                pattern: v.get("pattern").and_then(|p| p.as_str()).unwrap_or("%Y-%m-%d").to_string(),
            }),
            _ => None,
        }
    }
}

/// Groups digits in `n` (rounded to `decimals` places) with `,` every
/// three places left of the point — the shared core of `Currency`,
/// `Number`, and `Percent`.
fn format_thousands(n: f64, decimals: u32) -> String {
    let neg = n < 0.0;
    let scale = 10i64.pow(decimals);
    let scaled = (n.abs() * scale as f64).round() as i64;
    let int_part = scaled / scale;
    let frac_part = scaled % scale;

    let digits: Vec<char> = int_part.to_string().chars().collect();
    let mut grouped = String::new();
    for (i, c) in digits.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(*c);
    }
    let int_str: String = grouped.chars().rev().collect();

    let sign = if neg { "-" } else { "" };
    if decimals == 0 {
        format!("{sign}{int_str}")
    } else {
        format!("{sign}{int_str}.{frac_part:0width$}", width = decimals as usize)
    }
}

/// Reformats the `YYYY-MM-DD` prefix of an ISO-ish date/timestamp string
/// using a small strftime-like subset — no `chrono` dependency, since
/// pgapp's date fields are plain `text` already (see `FieldType`) with
/// no native date type to format from. Returns `None` on anything that
/// doesn't parse as `YYYY-MM-DD`, so the caller can fall back to the raw
/// value unchanged.
fn format_date(raw: &str, pattern: &str) -> Option<String> {
    let date_part = raw.split(['T', ' ']).next()?;
    let mut parts = date_part.splitn(3, '-');
    let y: u32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
    ];
    let month_name = MONTHS.get(m.checked_sub(1)? as usize)?;

    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&format!("{y:04}")),
            Some('y') => out.push_str(&format!("{:02}", y % 100)),
            Some('m') => out.push_str(&format!("{m:02}")),
            Some('d') => out.push_str(&format!("{d:02}")),
            Some('B') => out.push_str(month_name),
            Some('b') => out.push_str(&month_name[..3]),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    Some(out)
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
    /// the same entity (see `server.rs`'s `sibling_form`). `before_load`,
    /// when set, runs that action every time this report is about to
    /// fetch its rows — typically an `http_request` refreshing the
    /// collection a collection-backed entity reads from. `computed`
    /// columns and `formats` masks only apply to the entity-backed case
    /// (no `source_query`) — see `ComputedColumn`/`FormatMask`.
    Report {
        title: String,
        entity: String,
        columns: Vec<String>,
        source_query: Option<String>,
        link_column: Option<LinkColumn>,
        page_size: i64,
        before_load: Option<PreAction>,
        computed: Vec<ComputedColumn>,
        formats: HashMap<String, FormatMask>,
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
    /// A standalone clickable button, independent of any report row —
    /// unlike `Link` (a plain page target, no parameters) or `Action`
    /// (always a server action), `Button` is either: a redirect to
    /// another page with parameters mapped in from the *current page's*
    /// own query-string context (mirroring Oracle APEX's "Redirect to
    /// Page" button behavior — see `ButtonBehavior::Redirect`), or a
    /// server action run on click (`ButtonBehavior::RunAction`,
    /// identical to `Action` — kept as a `Button` variant rather than
    /// folded into `Action` so both button shapes share one component
    /// kind and one place in the App Builder's "Add Component" list).
    Button {
        label: String,
        behavior: ButtonBehavior,
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
            | ComponentDef::Action { html, .. }
            | ComponentDef::Button { html, .. } => *html = new_html,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_formats_with_thousands_and_two_decimals() {
        assert_eq!(FormatMask::Currency.apply("1234.5"), "$1,234.50");
        assert_eq!(FormatMask::Currency.apply("0"), "$0.00");
        assert_eq!(FormatMask::Currency.apply("-42.1"), "-$42.10");
    }

    #[test]
    fn currency_falls_back_to_raw_on_unparseable_input() {
        assert_eq!(FormatMask::Currency.apply("N/A"), "N/A");
    }

    #[test]
    fn number_mask_respects_decimals_and_groups_thousands() {
        assert_eq!(FormatMask::Number { decimals: 0 }.apply("1234567"), "1,234,567");
        assert_eq!(FormatMask::Number { decimals: 2 }.apply("3.14159"), "3.14");
    }

    #[test]
    fn percent_mask_rounds_to_an_integer_with_a_percent_sign() {
        assert_eq!(FormatMask::Percent.apply("87.6"), "88%");
        assert_eq!(FormatMask::Percent.apply("100"), "100%");
    }

    #[test]
    fn date_mask_reformats_iso_dates_and_ignores_a_time_suffix() {
        let mask = FormatMask::Date { pattern: "%m/%d/%Y".to_string() };
        assert_eq!(mask.apply("2026-07-21"), "07/21/2026");
        assert_eq!(mask.apply("2026-07-21T10:30:00"), "07/21/2026");
    }

    #[test]
    fn date_mask_supports_month_names_and_falls_back_on_bad_input() {
        let mask = FormatMask::Date { pattern: "%B %d, %Y".to_string() };
        assert_eq!(mask.apply("2026-01-05"), "January 05, 2026");
        assert_eq!(mask.apply("not-a-date"), "not-a-date");
    }

    #[test]
    fn empty_value_stays_empty_under_every_mask() {
        assert_eq!(FormatMask::Currency.apply(""), "");
        assert_eq!(FormatMask::Percent.apply(""), "");
        assert_eq!(FormatMask::Date { pattern: "%Y".to_string() }.apply(""), "");
    }

    #[test]
    fn format_mask_json_roundtrips() {
        for mask in [
            FormatMask::Currency,
            FormatMask::Number { decimals: 3 },
            FormatMask::Percent,
            FormatMask::Date { pattern: "%m/%d/%Y".to_string() },
        ] {
            let json = mask.to_json();
            assert_eq!(FormatMask::from_json(&json), Some(mask));
        }
    }
}
