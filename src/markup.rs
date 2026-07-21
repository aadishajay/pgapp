//! A small, hand-rolled parser for pgapp's APEX-inspired application markup.
//!
//! Grammar (informal):
//!
//! ```text
//! file      := app | fragment
//! fragment  := (entity | page | query)*     (see src/source.rs: directory-based apps)
//!
//! app       := "app" String "{" (appprop | auth | nav | header | footer | entity | page | query | auth_scheme)* "}"
//!
//! appprop   := ("theme" | "icons" | "chart_lib") ":" Ident
//! auth      := "auth" "{" "}"
//! auth_scheme := "auth_scheme" String "{" "roles" ":" identlist "}"
//!             (a named, reusable role group — see `model::AuthScheme` —
//!             referenced from any "requires:" by name instead of a
//!             literal role; unrecognized names fall back to being
//!             treated as a literal role, so this is purely additive)
//!
//! nav       := "nav" "{" navitem* "}"
//! navitem   := "item" String ( "->" "page" Ident | "{" navitem* "}" )
//!
//! header    := "header" "{" component* "}"
//! footer    := "footer" "{" component* "}"
//!
//! query     := "query" Ident "{" "sql" ":" String "}"
//!
//! entity    := "entity" String ("from" ("query" Ident | "collection" String))? "{" field* "}"
//! field     := "field" Ident ":" Ident ("required")? ("default" Value)?
//!
//! page      := "page" String "{" (pageprop | component | query | dynaction)* "}"
//! pageprop  := "requires" ":" Ident
//!
//! dynaction := "on" Ident "of" Ident "{" daop* "}"
//! daop      := "show" Ident
//!            | "hide" Ident
//!            | "toggle" Ident "when" String
//!            | "set" Ident "to" String
//!            | "refresh" Ident
//!            | "call" Ident itemconfig? "into" Ident
//!            (the "ajax callback": runs a server-side action module
//!            without a page reload, POST /:page/c/:idx/call; "into"
//!            names an item to set from the result, or a region/query
//!            to refresh afterward — resolved client-side at dispatch
//!            time, see runtime.js)
//!
//! component := (report | form | editable_table | chart | text | link | region
//!               | dynamic_content | action) compsuffix*
//! compsuffix := "requires" ":" Ident | htmlattrs
//!            (either order, either/both/neither present; "requires:"
//!            here is a per-component role gate, same semantics as a
//!            page's own — see `model::ComponentDef::set_requires`)
//! htmlattrs := "attrs" "(" Ident ":" (String | Ident) ("," Ident ":" (String | Ident))* ")"
//!            ("id"/"class" reserved; any other key -> a plain attribute,
//!            its "_" rewritten to "-")
//!
//! dynamic_content := "dynamic_content" String "calls" Ident itemconfig?
//!            (Oracle APEX's "PL/SQL Dynamic Content" region: the named
//!            action module's returned string, rendered once per page
//!            load as trusted HTML — see model::ComponentDef::DynamicContent)
//!
//! calendar  := "calendar" String "of" Ident "{" calprop* "}"
//! calprop   := "date" ":" Ident | "title" ":" Ident | "link" ":" "page" Ident
//!            (Oracle APEX's Calendar region: a month-grid view of one
//!            entity's rows bucketed by "date" — see
//!            model::ComponentDef::Calendar)
//!
//! map       := "map" String "of" Ident "{" mapprop* "}"
//! mapprop   := "lat" ":" Ident | "lng" ":" Ident | "title" ":" Ident
//!            | "link" ":" "page" Ident
//!            (Oracle APEX's Map region: a dependency-free inline-SVG
//!            scatter of one entity's rows by lat/lng — see
//!            model::ComponentDef::Map)
//!
//! faceted_search := "faceted_search" String "of" Ident "{" facetprop* "}"
//! facetprop := "facet" Ident "as" ("checkbox_list" | "range" | "date_range")
//!            (Oracle APEX's Faceted Search: a panel of facets filtering
//!            a sibling Report bound to the same entity, on the same
//!            page — found by entity match, same convention as a
//!            Report's companion Form. Entity-backed reports only — see
//!            model::ComponentDef::FacetedSearch)
//!
//! action    := "action" String "calls" Ident itemconfig?
//!
//! button    := "button" String ( "->" "page" Ident ( "(" paramlist ")" )?
//!                                | "calls" Ident itemconfig? )
//!            (the first form redirects, forwarding the current page's
//!            own query-string values under new names, same shape as a
//!            report's "link" property above; the second runs a
//!            server-side action module, same as "action")
//!
//! report    := "report" String "of" Ident "{" reportprop* "}"
//! reportprop := "columns" ":" identlist
//!             | "source" ":" "query" Ident
//!             | "link" ":" Ident "->" "page" Ident ( "(" paramlist ")" )?
//!             | "page_size" ":" Number
//!             | "before_load" ":" Ident itemconfig?
//!             | "computed" Ident ":" String
//!             (a scalar SQL expression, entity-backed reports only —
//!             see `model::ComputedColumn`; `t.` refers to the entity's
//!             own row, e.g. `computed total: "(select sum(x) from y
//!             where y.bill_key = t.bill_key)"`)
//!             | "format" Ident ":" formatmask
//!             | "aggregate" Ident ":" ("sum" | "avg" | "count" | "min" | "max")
//!             (Interactive Report's per-column footer aggregate,
//!             computed over the whole filtered result set — see
//!             model::AggregateFn; the column must be one of `columns`)
//!             | "break_on" ":" Ident
//!             (Interactive Report's Control Break — consecutive rows
//!             sharing this column's value show it once; table display
//!             mode only, see model::ComponentDef::Report::break_on)
//!             | "highlight" "(" "when" ":" String "," "color" ":" String ")"
//!             (Interactive Report's row highlight — `when` is a scalar
//!             SQL boolean expression, entity-backed reports only, same
//!             `t.<field>` convention as `computed`'s SQL; repeatable,
//!             first matching rule per row wins — see model::HighlightRule)
//!             | "heading" Ident ":" String
//!             (Classic Report's column heading override — a column not
//!             listed here just shows its own name)
//!             | "align" Ident ":" ("left" | "center" | "right")
//!             (Classic Report's column alignment; default left)
//!             | "display" ":" ("table" | "cards" | "list")
//!             (default "table"; Oracle APEX's "Cards" region folded in
//!             as a display mode — see model::REPORT_DISPLAY_MODES)
//! formatmask := "currency" | "percent"
//!             | "number" ( "(" Number ")" )?
//!             | "date" ( "(" String ")" )?
//!
//! form      := "form" String "of" Ident "{" formprop* "}"
//! formprop  := "fields" ":" identlist
//!            | itemprop
//!
//! editable_table := "editable_table" String "of" Ident "{" etprop* "}"
//! etprop    := "columns" ":" identlist
//!            | itemprop
//!
//! itemprop  := "item" Ident ("as" fielditem)? htmlattrs?
//!            (at least one of "as .../"attrs (...)" required — a bare
//!            "item name" with neither sets nothing and is rejected)
//!
//! chart     := "chart" String "from" "query" Ident "{" chartprop* "}"
//! chartprop := "type" ":" Ident       ("bar" | "line" | "area" | "pie" | "donut" | "scatter")
//!            | "x" ":" Ident
//!            | "y" ":" Ident
//!
//! text      := "text" String
//! link      := "link" String "->" "page" Ident
//! region    := "region" String "from" "query" Ident ( "{" "columns" ":" identlist "}" )?
//!
//! fielditem  := Ident itemconfig?
//! itemconfig := "(" arglist ")" | "from" "query" Ident
//! arglist    := String ("," String)*        (-> config = {"choices": [...]})
//!             | namedarg ("," namedarg)*    (-> config = {key: value, ...})
//! namedarg   := Ident ":" (String | Ident)
//!
//! identlist  := Ident ("," Ident)*
//! paramlist  := parammap ("," parammap)*
//! parammap   := Ident ":" Ident
//! value      := Ident | Number
//! ```
//!
//! `Ident` tokens are restricted to `[A-Za-z_][A-Za-z0-9_]*`, which means
//! every entity/field/page/query name that reaches the metadata layer is
//! already safe to splice into SQL as an identifier. Page names
//! themselves are string literals (so they can be arbitrary display
//! text), but anything that *targets* a page — `nav` items, `link`
//! report properties, `link` components — takes an `Ident`, so link
//! targets are restricted to the same safe charset. A query's `sql` is a
//! raw string, opaque to this parser — see `meta::compile_named_query`
//! for how its `:name` bind markers get turned into safe positional
//! parameters.
//!
//! A `fielditem`'s `Ident` (its "kind") isn't a fixed keyword set: it's
//! whatever's registered in `src/item_types.rs` at the time the app is
//! synced (checked there, not here — the parser doesn't know the
//! registry). `itemconfig` is deliberately generic (a plain JSON blob)
//! so a brand new item type never needs a grammar change: it just reads
//! whatever config keys it defines out of that blob itself.
//!
//! A page is simply an ordered list of components — there's no separate
//! "page kind" anymore. `Report` + `Form` on the same page is the usual
//! CRUD pattern (a paginated list plus a linked edit form); an
//! `EditableTable` stands alone; any number of `Chart`s make a
//! dashboard; `Text`/`Link`/`Region` compose freely with all of the
//! above.
//!
//! App-level settings live in the file too, not in environment
//! variables: `theme:`/`icons:`/`chart_lib:` pick the pluggable
//! theme/icon-pack/chart-library directories, and an `auth { }` block
//! turns on login (see `server::auth`; the block is empty today and
//! reserved for future options). A page's `requires: <role>` then
//! restricts that page to users holding the role ('admin' passes every
//! check). Users themselves are *never* declared in markup — passwords
//! don't belong in a source file; they're managed at runtime via the
//! built-in /users page.

use anyhow::{bail, Context, Result};

use crate::model::{
    AppDef, AuthScheme, ButtonBehavior, ComponentDef, ComputedColumn, DaOp, EntityDef, FieldDef, FieldItem,
    FieldType, FormatMask, HtmlAttrs, LinkColumn, NavItem, PageDef, PreAction, QueryDef, CHART_TYPES,
};

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Str(String),
    Symbol(char),
    Arrow,
}

/// Tokenizes `src`, returning each token alongside the 1-based line it
/// starts on — parse errors report those lines, which matters once an
/// app is split across many files.
fn lex(src: &str) -> Result<(Vec<Token>, Vec<u32>)> {
    let chars: Vec<char> = src.chars().collect();
    let mut line_at = Vec::with_capacity(chars.len() + 1);
    let mut line = 1u32;
    for &c in &chars {
        line_at.push(line);
        if c == '\n' {
            line += 1;
        }
    }
    line_at.push(line);

    let mut tokens = Vec::new();
    let mut lines = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '"' {
            let quote = i;
            i += 1;
            let mut value = String::new();
            loop {
                if i >= chars.len() {
                    bail!("unterminated string literal starting on line {}", line_at[quote]);
                }
                match chars[i] {
                    '"' => break,
                    // Only `\"` and `\\` are recognized escapes — any other
                    // backslash (e.g. a Windows path, a regex) is left
                    // exactly as written, so this is purely additive: no
                    // existing string literal (none use backslashes today)
                    // changes meaning.
                    '\\' if matches!(chars.get(i + 1), Some('"') | Some('\\')) => {
                        value.push(chars[i + 1]);
                        i += 2;
                    }
                    other => {
                        value.push(other);
                        i += 1;
                    }
                }
            }
            tokens.push(Token::Str(value));
            lines.push(line_at[quote]);
            i += 1;
        } else if c == '-' && chars.get(i + 1) == Some(&'>') {
            tokens.push(Token::Arrow);
            lines.push(line_at[i]);
            i += 2;
        } else if c == '{' || c == '}' || c == ':' || c == ',' || c == '(' || c == ')' {
            tokens.push(Token::Symbol(c));
            lines.push(line_at[i]);
            i += 1;
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect()));
            lines.push(line_at[start]);
        } else {
            bail!("unexpected character '{c}' in markup on line {}", line_at[i]);
        }
    }
    Ok((tokens, lines))
}

struct Parser {
    tokens: Vec<Token>,
    lines: Vec<u32>,
    pos: usize,
}

impl Parser {
    fn new(src: &str) -> Result<Self> {
        let (tokens, lines) = lex(src)?;
        Ok(Parser { tokens, lines, pos: 0 })
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// The line of the token about to be consumed (or of the last token
    /// when at end of input) — for error messages.
    fn cur_line(&self) -> u32 {
        self.lines
            .get(self.pos)
            .or_else(|| self.lines.last())
            .copied()
            .unwrap_or(1)
    }

    fn advance(&mut self) -> Result<Token> {
        let t = self
            .tokens
            .get(self.pos)
            .cloned()
            .with_context(|| format!("unexpected end of markup (line {})", self.cur_line()))?;
        self.pos += 1;
        Ok(t)
    }

    fn expect_symbol(&mut self, c: char) -> Result<()> {
        let line = self.cur_line();
        match self.advance()? {
            Token::Symbol(s) if s == c => Ok(()),
            other => bail!("expected '{c}', found {other:?} (line {line})"),
        }
    }

    fn expect_arrow(&mut self) -> Result<()> {
        let line = self.cur_line();
        match self.advance()? {
            Token::Arrow => Ok(()),
            other => bail!("expected '->', found {other:?} (line {line})"),
        }
    }

    fn expect_keyword(&mut self, word: &str) -> Result<()> {
        let line = self.cur_line();
        match self.advance()? {
            Token::Ident(s) if s == word => Ok(()),
            other => bail!("expected keyword '{word}', found {other:?} (line {line})"),
        }
    }

    fn expect_ident(&mut self) -> Result<String> {
        let line = self.cur_line();
        match self.advance()? {
            Token::Ident(s) => Ok(s),
            other => bail!("expected identifier, found {other:?} (line {line})"),
        }
    }

    fn expect_string(&mut self) -> Result<String> {
        let line = self.cur_line();
        match self.advance()? {
            Token::Str(s) => Ok(s),
            other => bail!("expected string literal, found {other:?} (line {line})"),
        }
    }

    fn at_symbol(&self, c: char) -> bool {
        matches!(self.peek(), Some(Token::Symbol(s)) if *s == c)
    }

    fn at_keyword(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Ident(s)) if s == word)
    }

    /// Parses a `-> page <Ident>` link target, common to nav items,
    /// report `link:` properties, and `link` components.
    fn parse_page_target(&mut self) -> Result<String> {
        self.expect_arrow()?;
        self.expect_keyword("page")?;
        self.expect_ident()
    }

    /// Parses `("theme" | "icons" | "chart_lib") ":" Ident`.
    fn parse_app_prop(&mut self) -> Result<String> {
        self.advance()?; // the property keyword, already matched
        self.expect_symbol(':')?;
        self.expect_ident()
    }

    fn parse_app(&mut self) -> Result<AppDef> {
        self.expect_keyword("app")?;
        let name = self.expect_string()?;
        self.expect_symbol('{')?;

        let mut theme = None;
        let mut icons = None;
        let mut chart_lib = None;
        let mut auth = false;
        let mut entities = Vec::new();
        let mut pages = Vec::new();
        let mut nav = Vec::new();
        let mut header = Vec::new();
        let mut footer = Vec::new();
        let mut queries = Vec::new();
        let mut auth_schemes = Vec::new();
        while !self.at_symbol('}') {
            if self.at_keyword("entity") {
                entities.push(self.parse_entity()?);
            } else if self.at_keyword("page") {
                pages.push(self.parse_page()?);
            } else if self.at_keyword("nav") {
                nav = self.parse_nav()?;
            } else if self.at_keyword("header") {
                header = self.parse_component_block("header")?;
            } else if self.at_keyword("footer") {
                footer = self.parse_component_block("footer")?;
            } else if self.at_keyword("query") {
                queries.push(self.parse_query()?);
            } else if self.at_keyword("auth_scheme") {
                auth_schemes.push(self.parse_auth_scheme()?);
            } else if self.at_keyword("theme") {
                theme = Some(self.parse_app_prop()?);
            } else if self.at_keyword("icons") {
                icons = Some(self.parse_app_prop()?);
            } else if self.at_keyword("chart_lib") {
                chart_lib = Some(self.parse_app_prop()?);
            } else if self.at_keyword("auth") {
                self.advance()?;
                self.expect_symbol('{')?;
                self.expect_symbol('}')?;
                auth = true;
            } else {
                bail!(
                    "expected 'entity', 'page', 'nav', 'header', 'footer', 'query', 'auth', \
                     'auth_scheme', 'theme', 'icons', or 'chart_lib', found {:?} (line {})",
                    self.peek(),
                    self.cur_line()
                );
            }
        }
        self.expect_symbol('}')?;

        Ok(AppDef {
            name,
            theme,
            icons,
            chart_lib,
            auth,
            entities,
            pages,
            nav,
            header,
            footer,
            queries,
            auth_schemes,
        })
    }

    /// Parses `"auth_scheme" String "{" "roles" ":" identlist "}"`.
    fn parse_auth_scheme(&mut self) -> Result<AuthScheme> {
        self.expect_keyword("auth_scheme")?;
        let name = self.expect_string()?;
        self.expect_symbol('{')?;
        let mut roles = Vec::new();
        while !self.at_symbol('}') {
            let prop = self.expect_ident()?;
            self.expect_symbol(':')?;
            match prop.as_str() {
                "roles" => roles = self.parse_ident_list()?,
                other => bail!("unknown auth_scheme property '{other}' (line {})", self.cur_line()),
            }
        }
        self.expect_symbol('}')?;
        if roles.is_empty() {
            bail!("auth_scheme '{name}' declares no roles");
        }
        Ok(AuthScheme { name, roles })
    }

    /// Parses `"query" Ident "{" "sql" ":" String "}"`.
    fn parse_query(&mut self) -> Result<QueryDef> {
        self.expect_keyword("query")?;
        let name = self.expect_ident()?;
        self.expect_symbol('{')?;
        self.expect_keyword("sql")?;
        self.expect_symbol(':')?;
        let sql = self.expect_string()?;
        self.expect_symbol('}')?;
        Ok(QueryDef { name, sql })
    }

    fn parse_nav(&mut self) -> Result<Vec<NavItem>> {
        self.expect_keyword("nav")?;
        self.expect_symbol('{')?;
        let mut items = Vec::new();
        while !self.at_symbol('}') {
            items.push(self.parse_nav_item()?);
        }
        self.expect_symbol('}')?;
        Ok(items)
    }

    fn parse_nav_item(&mut self) -> Result<NavItem> {
        self.expect_keyword("item")?;
        let label = self.expect_string()?;

        if self.at_symbol('{') {
            self.advance()?;
            let mut children = Vec::new();
            while !self.at_symbol('}') {
                children.push(self.parse_nav_item()?);
            }
            self.expect_symbol('}')?;
            Ok(NavItem {
                label,
                target_page: None,
                children,
            })
        } else {
            let target_page = self.parse_page_target()?;
            Ok(NavItem {
                label,
                target_page: Some(target_page),
                children: Vec::new(),
            })
        }
    }

    /// Parses `"header" "{" component* "}"` / `"footer" "{" component* "}"`.
    fn parse_component_block(&mut self, keyword: &str) -> Result<Vec<ComponentDef>> {
        self.expect_keyword(keyword)?;
        self.expect_symbol('{')?;
        let mut items = Vec::new();
        while !self.at_symbol('}') {
            items.push(self.parse_component()?);
        }
        self.expect_symbol('}')?;
        Ok(items)
    }

    fn parse_entity(&mut self) -> Result<EntityDef> {
        self.expect_keyword("entity")?;
        let name = self.expect_string()?;

        let mut source_query = None;
        let mut source_collection = None;
        if self.at_keyword("from") {
            self.advance()?;
            if self.at_keyword("query") {
                self.advance()?;
                source_query = Some(self.expect_ident()?);
            } else {
                self.expect_keyword("collection")?;
                source_collection = Some(self.expect_string()?);
            }
        }

        self.expect_symbol('{')?;
        let mut fields = Vec::new();
        while !self.at_symbol('}') {
            fields.push(self.parse_field()?);
        }
        self.expect_symbol('}')?;

        Ok(EntityDef { name, fields, source_query, source_collection })
    }

    fn parse_field(&mut self) -> Result<FieldDef> {
        self.expect_keyword("field")?;
        let name = self.expect_ident()?;
        self.expect_symbol(':')?;
        let ty_name = self.expect_ident()?;
        let ty = FieldType::parse(&ty_name)
            .with_context(|| format!("unknown field type '{ty_name}' on field '{name}'"))?;

        let mut required = false;
        let mut default = None;
        loop {
            if self.at_keyword("required") {
                self.advance()?;
                required = true;
            } else if self.at_keyword("default") {
                self.advance()?;
                default = Some(self.expect_ident()?);
            } else {
                break;
            }
        }

        Ok(FieldDef {
            name,
            ty,
            required,
            default,
        })
    }

    fn parse_page(&mut self) -> Result<PageDef> {
        self.expect_keyword("page")?;
        let name = self.expect_string()?;
        self.expect_symbol('{')?;

        let mut components = Vec::new();
        let mut queries = Vec::new();
        let mut required_role = None;
        while !self.at_symbol('}') {
            if self.at_keyword("query") {
                queries.push(self.parse_query()?);
            } else if self.at_keyword("requires") {
                self.advance()?;
                self.expect_symbol(':')?;
                required_role = Some(self.expect_ident()?);
            } else if self.at_keyword("on") {
                components.push(self.parse_dynamic_action()?);
            } else {
                components.push(self.parse_component()?);
            }
        }
        self.expect_symbol('}')?;

        Ok(PageDef {
            name,
            components,
            queries,
            required_role,
        })
    }

    /// Same walk as `parse_page`, but records each top-level component's
    /// start line instead of building `ComponentDef`s — see
    /// `page_component_start_lines`'s doc for why this exists as its own
    /// near-duplicate rather than adding an output parameter to
    /// `parse_page` itself (used by every other caller, which has no use
    /// for line numbers).
    fn parse_page_component_lines(&mut self) -> Result<(Vec<u32>, u32)> {
        self.expect_keyword("page")?;
        self.expect_string()?;
        self.expect_symbol('{')?;

        let mut lines = Vec::new();
        while !self.at_symbol('}') {
            if self.at_keyword("query") {
                self.parse_query()?;
            } else if self.at_keyword("requires") {
                self.advance()?;
                self.expect_symbol(':')?;
                self.expect_ident()?;
            } else if self.at_keyword("on") {
                lines.push(self.cur_line());
                self.parse_dynamic_action()?;
            } else {
                lines.push(self.cur_line());
                self.parse_component()?;
            }
        }
        let closing_line = self.cur_line();
        self.expect_symbol('}')?;
        Ok((lines, closing_line))
    }

    /// Parses one component, then an optional trailing `attrs (...)`
    /// suffix (see `parse_html_attrs`) shared by every kind — so a new
    /// component variant never has to reimplement id/class/attribute
    /// support itself.
    fn parse_component(&mut self) -> Result<ComponentDef> {
        let mut def = self.parse_component_kind()?;
        loop {
            if self.at_keyword("attrs") {
                self.advance()?;
                def.set_html(self.parse_html_attrs()?);
            } else if self.at_keyword("requires") {
                self.advance()?;
                self.expect_symbol(':')?;
                def.set_requires(Some(self.expect_ident()?));
            } else {
                break;
            }
        }
        Ok(def)
    }

    fn parse_component_kind(&mut self) -> Result<ComponentDef> {
        if self.at_keyword("report") {
            self.parse_report()
        } else if self.at_keyword("form") {
            self.parse_form()
        } else if self.at_keyword("editable_table") {
            self.parse_editable_table()
        } else if self.at_keyword("chart") {
            self.parse_chart()
        } else if self.at_keyword("text") {
            self.advance()?;
            Ok(ComponentDef::Text {
                text: self.expect_string()?,
                requires: None,
                html: HtmlAttrs::default(),
            })
        } else if self.at_keyword("link") {
            self.advance()?;
            let label = self.expect_string()?;
            let target_page = self.parse_page_target()?;
            Ok(ComponentDef::Link {
                label,
                target_page,
                requires: None,
                html: HtmlAttrs::default(),
            })
        } else if self.at_keyword("region") {
            self.advance()?;
            let label = self.expect_string()?;
            self.expect_keyword("from")?;
            self.expect_keyword("query")?;
            let query = self.expect_ident()?;
            let columns = if self.at_symbol('{') {
                self.advance()?;
                let mut columns = Vec::new();
                while !self.at_symbol('}') {
                    let prop = self.expect_ident()?;
                    self.expect_symbol(':')?;
                    match prop.as_str() {
                        "columns" => columns = self.parse_ident_list()?,
                        other => bail!("unknown region property '{other}' (line {})", self.cur_line()),
                    }
                }
                self.expect_symbol('}')?;
                columns
            } else {
                Vec::new()
            };
            Ok(ComponentDef::Region {
                label,
                query,
                columns,
                requires: None,
                html: HtmlAttrs::default(),
            })
        } else if self.at_keyword("dynamic_content") {
            self.advance()?;
            let label = self.expect_string()?;
            self.expect_keyword("calls")?;
            let name = self.expect_ident()?;
            let config = if self.at_symbol('(') {
                self.parse_item_config()?
            } else {
                serde_json::json!({})
            };
            Ok(ComponentDef::DynamicContent {
                label,
                name,
                config,
                requires: None,
                html: HtmlAttrs::default(),
            })
        } else if self.at_keyword("action") {
            self.advance()?;
            let label = self.expect_string()?;
            self.expect_keyword("calls")?;
            let name = self.expect_ident()?;
            let config = if self.at_symbol('(') {
                self.parse_item_config()?
            } else {
                serde_json::json!({})
            };
            Ok(ComponentDef::Action {
                label,
                name,
                config,
                requires: None,
                html: HtmlAttrs::default(),
            })
        } else if self.at_keyword("calendar") {
            self.parse_calendar()
        } else if self.at_keyword("map") {
            self.parse_map()
        } else if self.at_keyword("faceted_search") {
            self.parse_faceted_search()
        } else if self.at_keyword("button") {
            self.advance()?;
            let label = self.expect_string()?;
            let behavior = if matches!(self.peek(), Some(Token::Arrow)) {
                let target_page = self.parse_page_target()?;
                let extra_params = if self.at_symbol('(') { self.parse_param_list()? } else { Vec::new() };
                ButtonBehavior::Redirect { target_page, extra_params }
            } else {
                self.expect_keyword("calls")?;
                let name = self.expect_ident()?;
                let config = if self.at_symbol('(') {
                    self.parse_item_config()?
                } else {
                    serde_json::json!({})
                };
                ButtonBehavior::RunAction { name, config }
            };
            Ok(ComponentDef::Button {
                label,
                behavior,
                requires: None,
                html: HtmlAttrs::default(),
            })
        } else {
            bail!(
                "expected a component ('report', 'form', 'editable_table', 'chart', 'text', \
                 'link', 'region', 'dynamic_content', 'action', 'button', 'calendar', or 'map'), found {:?} (line {})",
                self.peek(),
                self.cur_line()
            );
        }
    }

    /// Parses `"(" Ident ":" value ("," Ident ":" value)* ")"` after the
    /// `attrs` keyword. `id`/`class` are reserved; every other key
    /// becomes a plain HTML attribute on the component's wrapper tag,
    /// with `_` rewritten to `-` (identifiers can't contain hyphens
    /// directly, so `data_foo: "bar"` renders as `data-foo="bar"`).
    fn parse_html_attrs(&mut self) -> Result<HtmlAttrs> {
        self.expect_symbol('(')?;
        let mut html = HtmlAttrs::default();
        loop {
            let key = self.expect_ident()?;
            self.expect_symbol(':')?;
            let value = self.expect_config_value()?;
            match key.as_str() {
                "id" => html.id = Some(value),
                "class" => html.class = Some(value),
                other => html.attrs.push((other.replace('_', "-"), value)),
            }
            if self.at_symbol(',') {
                self.advance()?;
            } else {
                break;
            }
        }
        self.expect_symbol(')')?;
        Ok(html)
    }

    /// Parses `"on" Ident "of" Ident "{" daop* "}"` — a client-side
    /// dynamic action (see `model::DaOp` and the runtime.js dispatcher).
    fn parse_dynamic_action(&mut self) -> Result<ComponentDef> {
        self.expect_keyword("on")?;
        let event = self.expect_ident()?;
        self.expect_keyword("of")?;
        let item = self.expect_ident()?;
        self.expect_symbol('{')?;

        let mut ops = Vec::new();
        while !self.at_symbol('}') {
            if self.at_keyword("show") {
                self.advance()?;
                ops.push(DaOp::Show(self.expect_ident()?));
            } else if self.at_keyword("hide") {
                self.advance()?;
                ops.push(DaOp::Hide(self.expect_ident()?));
            } else if self.at_keyword("toggle") {
                self.advance()?;
                let target = self.expect_ident()?;
                self.expect_keyword("when")?;
                let when = self.expect_string()?;
                ops.push(DaOp::Toggle { item: target, when });
            } else if self.at_keyword("set") {
                self.advance()?;
                let target = self.expect_ident()?;
                self.expect_keyword("to")?;
                let expr = self.expect_string()?;
                ops.push(DaOp::Set { item: target, expr });
            } else if self.at_keyword("refresh") {
                self.advance()?;
                ops.push(DaOp::Refresh(self.expect_ident()?));
            } else if self.at_keyword("call") {
                self.advance()?;
                let action = self.expect_ident()?;
                let config = if self.at_symbol('(') { self.parse_item_config()? } else { serde_json::json!({}) };
                self.expect_keyword("into")?;
                let target = self.expect_ident()?;
                ops.push(DaOp::Call { action, config, target });
            } else {
                bail!(
                    "expected 'show', 'hide', 'toggle', 'set', 'refresh', or 'call' inside an \
                     'on ... of ...' block, found {:?} (line {})",
                    self.peek(),
                    self.cur_line()
                );
            }
        }
        self.expect_symbol('}')?;

        Ok(ComponentDef::DynamicAction { event, item, ops })
    }

    fn parse_report(&mut self) -> Result<ComponentDef> {
        self.expect_keyword("report")?;
        let title = self.expect_string()?;
        self.expect_keyword("of")?;
        let entity = self.expect_ident()?;
        self.expect_symbol('{')?;

        let mut columns = Vec::new();
        let mut source_query = None;
        let mut link_column = None;
        let mut page_size: i64 = 20;
        let mut before_load = None;
        let mut computed = Vec::new();
        let mut formats = std::collections::HashMap::new();
        let mut aggregates = std::collections::HashMap::new();
        let mut break_on = None;
        let mut highlights = Vec::new();
        let mut headings = std::collections::HashMap::new();
        let mut aligns = std::collections::HashMap::new();
        let mut display = "table".to_string();
        while !self.at_symbol('}') {
            if self.at_keyword("computed") {
                self.advance()?;
                let name = self.expect_ident()?;
                self.expect_symbol(':')?;
                let sql = self.expect_string()?;
                computed.push(ComputedColumn { name, sql });
                continue;
            }
            if self.at_keyword("format") {
                self.advance()?;
                let name = self.expect_ident()?;
                self.expect_symbol(':')?;
                let mask = self.parse_format_mask()?;
                formats.insert(name, mask);
                continue;
            }
            if self.at_keyword("aggregate") {
                self.advance()?;
                let name = self.expect_ident()?;
                self.expect_symbol(':')?;
                let kind = self.expect_ident()?;
                let agg = crate::model::AggregateFn::parse(&kind)
                    .ok_or_else(|| anyhow::anyhow!("unknown aggregate '{kind}' on report '{title}' (line {})", self.cur_line()))?;
                aggregates.insert(name, agg);
                continue;
            }
            if self.at_keyword("highlight") {
                self.advance()?;
                let config = self.parse_item_config()?;
                let when = config
                    .get("when")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("highlight on report '{title}' is missing required 'when:' (line {})", self.cur_line()))?
                    .to_string();
                let color = config
                    .get("color")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("highlight on report '{title}' is missing required 'color:' (line {})", self.cur_line()))?
                    .to_string();
                highlights.push(crate::model::HighlightRule { when, color });
                continue;
            }
            if self.at_keyword("heading") {
                self.advance()?;
                let name = self.expect_ident()?;
                self.expect_symbol(':')?;
                let label = self.expect_string()?;
                headings.insert(name, label);
                continue;
            }
            if self.at_keyword("align") {
                self.advance()?;
                let name = self.expect_ident()?;
                self.expect_symbol(':')?;
                let a = self.expect_ident()?;
                if !matches!(a.as_str(), "left" | "center" | "right") {
                    anyhow::bail!(
                        "invalid align '{a}' on report '{title}' (line {}) — expected left, center, or right",
                        self.cur_line()
                    );
                }
                aligns.insert(name, a);
                continue;
            }
            let prop = self.expect_ident()?;
            self.expect_symbol(':')?;
            match prop.as_str() {
                "columns" => columns = self.parse_ident_list()?,
                "source" => {
                    self.expect_keyword("query")?;
                    source_query = Some(self.expect_ident()?);
                }
                "link" => {
                    let field = self.expect_ident()?;
                    let target_page = self.parse_page_target()?;
                    let extra_params = if self.at_symbol('(') {
                        self.parse_param_list()?
                    } else {
                        Vec::new()
                    };
                    link_column = Some(LinkColumn {
                        field,
                        target_page,
                        extra_params,
                    });
                }
                "page_size" => {
                    let n = self.expect_ident()?;
                    page_size = n
                        .parse()
                        .with_context(|| format!("invalid page_size '{n}' on report '{title}'"))?;
                }
                "before_load" => {
                    let name = self.expect_ident()?;
                    let config = if self.at_symbol('(') {
                        self.parse_item_config()?
                    } else {
                        serde_json::json!({})
                    };
                    before_load = Some(PreAction { name, config });
                }
                "display" => display = self.expect_ident()?,
                "break_on" => break_on = Some(self.expect_ident()?),
                other => bail!("unknown report property '{other}' (line {})", self.cur_line()),
            }
        }
        self.expect_symbol('}')?;

        if !crate::model::REPORT_DISPLAY_MODES.contains(&display.as_str()) {
            bail!(
                "report '{title}' has unknown display mode '{display}' (expected one of: {})",
                crate::model::REPORT_DISPLAY_MODES.join(", ")
            );
        }

        Ok(ComponentDef::Report {
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
            headings,
            aligns,
            display,
            requires: None,
            html: HtmlAttrs::default(),
        })
    }

    /// `"calendar" String "of" Ident "{" ("date" ":" Ident | "title" ":" Ident
    /// | "link" ":" "page" Ident)* "}"` — Oracle APEX's Calendar region
    /// (see `model::ComponentDef::Calendar`).
    fn parse_calendar(&mut self) -> Result<ComponentDef> {
        self.expect_keyword("calendar")?;
        let title = self.expect_string()?;
        self.expect_keyword("of")?;
        let entity = self.expect_ident()?;
        self.expect_symbol('{')?;

        let mut date_field = None;
        let mut title_field = None;
        let mut link_page = None;
        while !self.at_symbol('}') {
            let prop = self.expect_ident()?;
            self.expect_symbol(':')?;
            match prop.as_str() {
                "date" => date_field = Some(self.expect_ident()?),
                "title" => title_field = Some(self.expect_ident()?),
                "link" => {
                    self.expect_keyword("page")?;
                    link_page = Some(self.expect_ident()?);
                }
                other => bail!("unknown calendar property '{other}' (line {})", self.cur_line()),
            }
        }
        self.expect_symbol('}')?;

        let date_field = date_field
            .ok_or_else(|| anyhow::anyhow!("calendar '{title}' is missing required 'date:' property"))?;
        let title_field = title_field
            .ok_or_else(|| anyhow::anyhow!("calendar '{title}' is missing required 'title:' property"))?;

        Ok(ComponentDef::Calendar {
            title,
            entity,
            date_field,
            title_field,
            link_page,
            requires: None,
            html: HtmlAttrs::default(),
        })
    }

    /// `"map" String "of" Ident "{" ("lat" ":" Ident | "lng" ":" Ident
    /// | "title" ":" Ident | "link" ":" "page" Ident)* "}"` — Oracle
    /// APEX's Map region (see `model::ComponentDef::Map`).
    fn parse_map(&mut self) -> Result<ComponentDef> {
        self.expect_keyword("map")?;
        let title = self.expect_string()?;
        self.expect_keyword("of")?;
        let entity = self.expect_ident()?;
        self.expect_symbol('{')?;

        let mut lat_field = None;
        let mut lng_field = None;
        let mut title_field = None;
        let mut link_page = None;
        while !self.at_symbol('}') {
            let prop = self.expect_ident()?;
            self.expect_symbol(':')?;
            match prop.as_str() {
                "lat" => lat_field = Some(self.expect_ident()?),
                "lng" => lng_field = Some(self.expect_ident()?),
                "title" => title_field = Some(self.expect_ident()?),
                "link" => {
                    self.expect_keyword("page")?;
                    link_page = Some(self.expect_ident()?);
                }
                other => bail!("unknown map property '{other}' (line {})", self.cur_line()),
            }
        }
        self.expect_symbol('}')?;

        let lat_field = lat_field.ok_or_else(|| anyhow::anyhow!("map '{title}' is missing required 'lat:' property"))?;
        let lng_field = lng_field.ok_or_else(|| anyhow::anyhow!("map '{title}' is missing required 'lng:' property"))?;
        let title_field =
            title_field.ok_or_else(|| anyhow::anyhow!("map '{title}' is missing required 'title:' property"))?;

        Ok(ComponentDef::Map {
            title,
            entity,
            lat_field,
            lng_field,
            title_field,
            link_page,
            requires: None,
            html: HtmlAttrs::default(),
        })
    }

    /// `"faceted_search" String "of" Ident "{" ("facet" Ident "as"
    /// ("checkbox_list" | "range" | "date_range"))* "}"` — Oracle
    /// APEX's Faceted Search region, bound to a sibling `Report` on the
    /// same entity (see `model::ComponentDef::FacetedSearch`).
    fn parse_faceted_search(&mut self) -> Result<ComponentDef> {
        self.expect_keyword("faceted_search")?;
        let title = self.expect_string()?;
        self.expect_keyword("of")?;
        let entity = self.expect_ident()?;
        self.expect_symbol('{')?;

        let mut facets = Vec::new();
        while !self.at_symbol('}') {
            self.expect_keyword("facet")?;
            let column = self.expect_ident()?;
            self.expect_keyword("as")?;
            let kind_name = self.expect_ident()?;
            let kind = crate::model::FacetKind::parse(&kind_name).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown facet kind '{kind_name}' on faceted search '{title}' (line {}) — \
                     expected checkbox_list, range, or date_range",
                    self.cur_line()
                )
            })?;
            facets.push(crate::model::Facet { column, kind });
        }
        self.expect_symbol('}')?;

        Ok(ComponentDef::FacetedSearch {
            title,
            entity,
            facets,
            requires: None,
            html: HtmlAttrs::default(),
        })
    }

    /// `"currency" | "percent" | "number" ("(" Number ")")? | "date" ("(" String ")")?`
    /// — a report `format:` mask.
    fn parse_format_mask(&mut self) -> Result<FormatMask> {
        let line = self.cur_line();
        let kind = self.expect_ident()?;
        match kind.as_str() {
            "currency" => Ok(FormatMask::Currency),
            "percent" => Ok(FormatMask::Percent),
            "number" => {
                let decimals = if self.at_symbol('(') {
                    self.advance()?;
                    let n = self.expect_ident()?;
                    self.expect_symbol(')')?;
                    n.parse().with_context(|| format!("invalid decimal count '{n}' (line {line})"))?
                } else {
                    0
                };
                Ok(FormatMask::Number { decimals })
            }
            "date" => {
                let pattern = if self.at_symbol('(') {
                    self.advance()?;
                    let p = self.expect_string()?;
                    self.expect_symbol(')')?;
                    p
                } else {
                    "%Y-%m-%d".to_string()
                };
                Ok(FormatMask::Date { pattern })
            }
            other => bail!("unknown format mask '{other}' (line {line})"),
        }
    }

    fn parse_form(&mut self) -> Result<ComponentDef> {
        self.expect_keyword("form")?;
        let title = self.expect_string()?;
        self.expect_keyword("of")?;
        let entity = self.expect_ident()?;
        self.expect_symbol('{')?;

        let mut fields = Vec::new();
        let mut item_types = std::collections::HashMap::new();
        let mut field_html = std::collections::HashMap::new();
        while !self.at_symbol('}') {
            if self.at_keyword("item") {
                let (field, item, html) = self.parse_field_item()?;
                if let Some(item) = item {
                    item_types.insert(field.clone(), item);
                }
                if !html.is_empty() {
                    field_html.insert(field, html);
                }
                continue;
            }
            let prop = self.expect_ident()?;
            self.expect_symbol(':')?;
            match prop.as_str() {
                "fields" => fields = self.parse_ident_list()?,
                other => bail!("unknown form property '{other}' (line {})", self.cur_line()),
            }
        }
        self.expect_symbol('}')?;

        Ok(ComponentDef::Form {
            title,
            entity,
            fields,
            item_types,
            field_html,
            requires: None,
            html: HtmlAttrs::default(),
        })
    }

    fn parse_editable_table(&mut self) -> Result<ComponentDef> {
        self.expect_keyword("editable_table")?;
        let title = self.expect_string()?;
        self.expect_keyword("of")?;
        let entity = self.expect_ident()?;
        self.expect_symbol('{')?;

        let mut columns = Vec::new();
        let mut item_types = std::collections::HashMap::new();
        let mut field_html = std::collections::HashMap::new();
        while !self.at_symbol('}') {
            if self.at_keyword("item") {
                let (field, item, html) = self.parse_field_item()?;
                if let Some(item) = item {
                    item_types.insert(field.clone(), item);
                }
                if !html.is_empty() {
                    field_html.insert(field, html);
                }
                continue;
            }
            let prop = self.expect_ident()?;
            self.expect_symbol(':')?;
            match prop.as_str() {
                "columns" => columns = self.parse_ident_list()?,
                other => bail!("unknown editable_table property '{other}' (line {})", self.cur_line()),
            }
        }
        self.expect_symbol('}')?;

        Ok(ComponentDef::EditableTable {
            title,
            entity,
            columns,
            item_types,
            field_html,
            requires: None,
            html: HtmlAttrs::default(),
        })
    }

    fn parse_chart(&mut self) -> Result<ComponentDef> {
        self.expect_keyword("chart")?;
        let title = self.expect_string()?;
        self.expect_keyword("from")?;
        self.expect_keyword("query")?;
        let query = self.expect_ident()?;
        self.expect_symbol('{')?;

        let mut chart_type = "bar".to_string();
        let mut x = String::new();
        let mut y = String::new();
        while !self.at_symbol('}') {
            let prop = self.expect_ident()?;
            self.expect_symbol(':')?;
            match prop.as_str() {
                "type" => chart_type = self.expect_ident()?,
                "x" => x = self.expect_ident()?,
                "y" => y = self.expect_ident()?,
                other => bail!("unknown chart property '{other}' (line {})", self.cur_line()),
            }
        }
        self.expect_symbol('}')?;

        if x.is_empty() || y.is_empty() {
            bail!("chart '{title}' requires both 'x' and 'y' properties");
        }
        if !CHART_TYPES.contains(&chart_type.as_str()) {
            bail!(
                "chart '{title}' has unknown type '{chart_type}' (expected one of: {})",
                CHART_TYPES.join(", ")
            );
        }

        Ok(ComponentDef::Chart {
            title,
            query,
            chart_type,
            x,
            y,
            requires: None,
            html: HtmlAttrs::default(),
        })
    }

    /// Parses `"item" Ident "as" Ident itemconfig?` — the kind is any
    /// identifier (validated against the item type registry later, in
    /// `meta::sync_app`, not here) and the config is a generic blob (see
    /// `parse_item_config`).
    /// `"as" kind` and a trailing `attrs (...)` are each optional, but at
    /// least one must be present — otherwise the `item` line does
    /// nothing at all and is almost certainly a mistake. Letting `as`
    /// be skipped means `item name attrs (...)` can set `id`/`class`/
    /// attributes on a field's wrapper without also forcing an item
    /// type override.
    fn parse_field_item(&mut self) -> Result<(String, Option<FieldItem>, HtmlAttrs)> {
        self.expect_keyword("item")?;
        let field = self.expect_ident()?;

        let field_item = if self.at_keyword("as") {
            self.advance()?;
            let kind = self.expect_ident()?;
            let config = if self.at_keyword("from") {
                self.advance()?;
                self.expect_keyword("query")?;
                let query_name = self.expect_ident()?;
                serde_json::json!({ "query": query_name })
            } else if self.at_symbol('(') {
                self.parse_item_config()?
            } else {
                serde_json::json!({})
            };
            Some(FieldItem { kind, config })
        } else {
            None
        };

        let html = if self.at_keyword("attrs") {
            self.advance()?;
            self.parse_html_attrs()?
        } else {
            HtmlAttrs::default()
        };

        if field_item.is_none() && html.is_empty() {
            bail!(
                "item '{field}' needs 'as <kind>' and/or 'attrs (...)' (line {})",
                self.cur_line()
            );
        }

        Ok((field, field_item, html))
    }

    /// Parses `"(" arg ("," arg)* ")"`. Every arg in the list must be the
    /// same shape: all bare strings become `{"choices": [...]}` (the
    /// radio/popup static-list shorthand); all `key: value` pairs become
    /// `{key: value, ...}` for anything else (e.g. a slider's
    /// `min`/`max`/`step`).
    fn parse_item_config(&mut self) -> Result<serde_json::Value> {
        self.expect_symbol('(')?;
        let named = matches!(self.peek(), Some(Token::Ident(_)));

        let config = if named {
            let mut map = serde_json::Map::new();
            loop {
                let key = self.expect_ident()?;
                self.expect_symbol(':')?;
                let value = self.expect_config_value()?;
                map.insert(key, serde_json::Value::String(value));
                if self.at_symbol(',') {
                    self.advance()?;
                } else {
                    break;
                }
            }
            serde_json::Value::Object(map)
        } else {
            let mut choices = Vec::new();
            loop {
                choices.push(serde_json::Value::String(self.expect_string()?));
                if self.at_symbol(',') {
                    self.advance()?;
                } else {
                    break;
                }
            }
            serde_json::json!({ "choices": choices })
        };

        self.expect_symbol(')')?;
        Ok(config)
    }

    /// A named config value: a quoted string or a bare word/number.
    fn expect_config_value(&mut self) -> Result<String> {
        match self.advance()? {
            Token::Str(s) => Ok(s),
            Token::Ident(s) => Ok(s),
            other => bail!("expected a config value, found {other:?}"),
        }
    }

    /// Parses `"(" Ident ":" Ident ("," Ident ":" Ident)* ")"` — the
    /// optional extra parameters on a report `link:` property.
    fn parse_param_list(&mut self) -> Result<Vec<(String, String)>> {
        self.expect_symbol('(')?;
        let mut out = vec![self.parse_param_pair()?];
        while self.at_symbol(',') {
            self.advance()?;
            out.push(self.parse_param_pair()?);
        }
        self.expect_symbol(')')?;
        Ok(out)
    }

    fn parse_param_pair(&mut self) -> Result<(String, String)> {
        let field = self.expect_ident()?;
        self.expect_symbol(':')?;
        let param = self.expect_ident()?;
        Ok((field, param))
    }

    fn parse_ident_list(&mut self) -> Result<Vec<String>> {
        let mut out = vec![self.expect_ident()?];
        while self.at_symbol(',') {
            self.advance()?;
            out.push(self.expect_ident()?);
        }
        Ok(out)
    }
}

pub fn parse_app(src: &str) -> Result<AppDef> {
    let mut parser = Parser::new(src)?;
    let app = parser.parse_app()?;
    if parser.pos != parser.tokens.len() {
        bail!("unexpected trailing content after app block (line {})", parser.cur_line());
    }
    Ok(app)
}

/// The 1-based start line of each of `page_name`'s top-level components
/// (in file order), plus the line its closing `}` is on — everything
/// `src/page_reorder.rs` needs to cut the page's component blocks out of
/// the file's raw text and splice them back in a different order,
/// without regenerating markup text from a parsed model (which would
/// throw away hand-written formatting/comments this parser already
/// discards). Reuses the exact same top-level dispatch `parse_app`
/// does, so a valid file is walked identically here — every non-target
/// construct is parsed and discarded purely to advance past it
/// correctly. Single-file apps only, same as `parse_app`; a directory
/// app's fragments aren't addressed by this (see `source.rs`).
pub fn page_component_start_lines(source: &str, page_name: &str) -> Result<(Vec<u32>, u32)> {
    let mut parser = Parser::new(source)?;
    parser.expect_keyword("app")?;
    parser.expect_string()?;
    parser.expect_symbol('{')?;
    while !parser.at_symbol('}') {
        if parser.at_keyword("entity") {
            parser.parse_entity()?;
        } else if parser.at_keyword("page") {
            // Peek the page's name (without consuming) to decide whether
            // this is the one we want before committing to either path.
            let save = parser.pos;
            parser.advance()?; // "page"
            let name = parser.expect_string()?;
            parser.pos = save;
            if name == page_name {
                return parser.parse_page_component_lines();
            }
            parser.parse_page()?;
        } else if parser.at_keyword("nav") {
            parser.parse_nav()?;
        } else if parser.at_keyword("header") {
            parser.parse_component_block("header")?;
        } else if parser.at_keyword("footer") {
            parser.parse_component_block("footer")?;
        } else if parser.at_keyword("query") {
            parser.parse_query()?;
        } else if parser.at_keyword("theme") || parser.at_keyword("icons") || parser.at_keyword("chart_lib") {
            parser.parse_app_prop()?;
        } else if parser.at_keyword("auth") {
            parser.advance()?;
            parser.expect_symbol('{')?;
            parser.expect_symbol('}')?;
        } else {
            bail!(
                "expected 'entity', 'page', 'nav', 'header', 'footer', 'query', 'auth', \
                 'theme', 'icons', or 'chart_lib', found {:?} (line {})",
                parser.peek(),
                parser.cur_line()
            );
        }
    }
    bail!("no page named '{page_name}' in this markup")
}

/// The 1-based start line of every top-level `page` block (name, line,
/// in file order), plus the line the *app's own* closing `}` is on —
/// everything `src/page_reorder.rs` needs to add or remove a whole
/// page, the same reasoning as `page_component_start_lines` one level
/// up: a real walk of `parse_app`'s own dispatch, so it can never
/// disagree with the parser about where a page block starts or ends.
/// Single-file apps only, same restriction as `page_component_start_lines`.
pub fn app_page_start_lines(source: &str) -> Result<(Vec<(String, u32)>, u32)> {
    let mut parser = Parser::new(source)?;
    parser.expect_keyword("app")?;
    parser.expect_string()?;
    parser.expect_symbol('{')?;
    let mut pages = Vec::new();
    while !parser.at_symbol('}') {
        if parser.at_keyword("entity") {
            parser.parse_entity()?;
        } else if parser.at_keyword("page") {
            let line = parser.cur_line();
            let save = parser.pos;
            parser.advance()?; // "page"
            let name = parser.expect_string()?;
            parser.pos = save;
            pages.push((name, line));
            parser.parse_page()?;
        } else if parser.at_keyword("nav") {
            parser.parse_nav()?;
        } else if parser.at_keyword("header") {
            parser.parse_component_block("header")?;
        } else if parser.at_keyword("footer") {
            parser.parse_component_block("footer")?;
        } else if parser.at_keyword("query") {
            parser.parse_query()?;
        } else if parser.at_keyword("theme") || parser.at_keyword("icons") || parser.at_keyword("chart_lib") {
            parser.parse_app_prop()?;
        } else if parser.at_keyword("auth") {
            parser.advance()?;
            parser.expect_symbol('{')?;
            parser.expect_symbol('}')?;
        } else {
            bail!(
                "expected 'entity', 'page', 'nav', 'header', 'footer', 'query', 'auth', \
                 'theme', 'icons', or 'chart_lib', found {:?} (line {})",
                parser.peek(),
                parser.cur_line()
            );
        }
    }
    let closing_line = parser.cur_line();
    Ok((pages, closing_line))
}

/// The top-level blocks a non-`app` file in a directory-based app may
/// contain — see [`parse_fragment`] and `src/source.rs`.
#[derive(Debug, Default)]
pub struct Fragment {
    pub entities: Vec<EntityDef>,
    pub pages: Vec<PageDef>,
    pub queries: Vec<QueryDef>,
    pub auth_schemes: Vec<AuthScheme>,
}

/// Parses a fragment file: any number of top-level `entity`, `page`,
/// `query`, and `auth_scheme` blocks, *without* an `app "..." { }`
/// wrapper. Everything app-*wide* — settings, the `auth` block itself,
/// `nav`, `header`, `footer` — belongs in the one file that declares
/// the `app` block, so a fragment declaring one of those is still an
/// error here rather than a silent merge surprise. `auth_scheme` is
/// different: like `entity`/`page`/`query`, it's a named, pluralizable
/// declaration rather than an app-wide singleton, so it merges by name
/// the same way (see `source.rs`'s `load_dir`).
pub fn parse_fragment(src: &str) -> Result<Fragment> {
    let mut parser = Parser::new(src)?;
    let mut fragment = Fragment::default();
    while parser.pos != parser.tokens.len() {
        if parser.at_keyword("entity") {
            fragment.entities.push(parser.parse_entity()?);
        } else if parser.at_keyword("page") {
            fragment.pages.push(parser.parse_page()?);
        } else if parser.at_keyword("query") {
            fragment.queries.push(parser.parse_query()?);
        } else if parser.at_keyword("auth_scheme") {
            fragment.auth_schemes.push(parser.parse_auth_scheme()?);
        } else {
            bail!(
                "expected a top-level 'entity', 'page', 'query', or 'auth_scheme' block (app \
                 settings, 'auth', 'nav', 'header', and 'footer' belong in the file with the \
                 `app` block), found {:?} (line {})",
                parser.peek(),
                parser.cur_line()
            );
        }
    }
    Ok(fragment)
}

/// Whether `src` is a full app file (starts with the `app` keyword) as
/// opposed to a fragment — how `src/source.rs` tells the two apart in
/// a directory.
pub fn starts_app_block(src: &str) -> Result<bool> {
    let (tokens, _) = lex(src)?;
    Ok(matches!(tokens.first(), Some(Token::Ident(s)) if s == "app"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_component_start_lines_finds_the_right_page_and_closing_line() {
        let src = r#"
app "Demo" {
  entity "t" { field id: id field name: text }

  page "Other" {
    text "not this one"
  }

  page "Target" {
    text "first"
    text "second"
    report "R" of t {
      columns: name
    }
  }
}
"#;
        let (lines, closing_line) = page_component_start_lines(src, "Target").unwrap();
        assert_eq!(lines.len(), 3);
        // Line 1 is the leading blank line in the raw string above, so
        // "page \"Target\" {" is line 9, its three components on 10/11/12.
        assert_eq!(lines, vec![10, 11, 12]);
        assert_eq!(closing_line, 15);

        assert!(page_component_start_lines(src, "Nonexistent").is_err());
    }

    #[test]
    fn parses_todo_example() {
        let src = include_str!("../examples/todo.pgapp");
        let app = parse_app(src).unwrap();
        assert_eq!(app.name, "Todo");

        assert_eq!(app.entities.len(), 1);
        let tasks = &app.entities[0];
        assert_eq!(tasks.name, "tasks");
        assert_eq!(tasks.fields.len(), 8);

        assert_eq!(app.header.len(), 1);
        assert!(matches!(&app.header[0], ComponentDef::Text { .. }));
        assert_eq!(app.footer.len(), 2);
        assert!(matches!(&app.footer[0], ComponentDef::Text { .. }));
        assert!(matches!(&app.footer[1], ComponentDef::Link { .. }));

        assert_eq!(app.queries.len(), 3);
        assert!(app.queries.iter().any(|q| q.name == "assignees"));
        assert!(app.queries.iter().any(|q| q.name == "open"));
        assert!(app.queries.iter().any(|q| q.name == "by_priority"));

        assert_eq!(app.pages.len(), 5);

        let tasks_page = app.pages.iter().find(|p| p.name == "Tasks").unwrap();
        assert_eq!(tasks_page.components.len(), 4); // report, form, text, region
        let report = tasks_page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Report { columns, link_column, page_size, .. } => {
                    Some((columns, link_column, page_size))
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(
            *report.0,
            vec!["title", "priority", "done", "estimate_hours", "created_at"]
        );
        let link = report.1.as_ref().unwrap();
        assert_eq!(link.field, "title");
        assert_eq!(link.target_page, "TaskDetail");
        assert_eq!(link.extra_params, vec![("priority".to_string(), "priority".to_string())]);
        assert_eq!(*report.2, 5);

        let form = tasks_page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Form { fields, item_types, .. } => Some((fields, item_types)),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            *form.0,
            vec!["title", "priority", "done", "assignee", "notes", "estimate_hours"]
        );
        let priority = form.1.get("priority").unwrap();
        assert_eq!(priority.kind, "radio");
        assert_eq!(priority.config["choices"], serde_json::json!(["Low", "Medium", "High"]));
        let assignee = form.1.get("assignee").unwrap();
        assert_eq!(assignee.kind, "popup");
        assert_eq!(assignee.config["query"], "assignees");
        let notes = form.1.get("notes").unwrap();
        assert_eq!(notes.kind, "readonly");
        let estimate = form.1.get("estimate_hours").unwrap();
        assert_eq!(estimate.kind, "slider");
        assert_eq!(estimate.config["min"], "0");
        assert_eq!(estimate.config["max"], "40");
        assert_eq!(estimate.config["step"], "1");
        assert!(form.1.get("title").is_none());
        assert!(form.1.get("done").is_none());

        assert_eq!(tasks_page.queries.len(), 1);
        assert_eq!(tasks_page.queries[0].name, "recent");
        assert!(tasks_page
            .components
            .iter()
            .any(|c| matches!(c, ComponentDef::Region { query, .. } if query == "recent")));

        let detail_page = app.pages.iter().find(|p| p.name == "TaskDetail").unwrap();
        assert_eq!(detail_page.queries.len(), 1);
        assert_eq!(detail_page.queries[0].name, "siblings");

        let open_tasks = app.pages.iter().find(|p| p.name == "OpenTasks").unwrap();
        let open_report = open_tasks
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Report { source_query, .. } => Some(source_query),
                _ => None,
            })
            .unwrap();
        assert_eq!(open_report.as_deref(), Some("open"));

        let about_page = app.pages.iter().find(|p| p.name == "About").unwrap();
        assert_eq!(about_page.components.len(), 3); // chart, text, link
        assert!(about_page
            .components
            .iter()
            .any(|c| matches!(c, ComponentDef::Chart { .. })));
        assert!(about_page
            .components
            .iter()
            .any(|c| matches!(c, ComponentDef::Text { .. })));
        assert!(about_page
            .components
            .iter()
            .any(|c| matches!(c, ComponentDef::Link { .. })));

        assert_eq!(app.nav.len(), 4);
        assert_eq!(app.nav[0].label, "Tasks");
        assert_eq!(app.nav[0].target_page.as_deref(), Some("Tasks"));
        assert_eq!(app.nav[1].label, "Open");
        assert_eq!(app.nav[2].label, "Quick edit");
        assert_eq!(app.nav[3].label, "More");
        assert!(app.nav[3].target_page.is_none());
        assert_eq!(app.nav[3].children.len(), 1);
        assert_eq!(app.nav[3].children[0].label, "About");
    }

    #[test]
    fn parses_venpay_example() {
        let src = include_str!("../examples/venpay.pgapp");
        let app = parse_app(src).unwrap();
        assert_eq!(app.name, "VenPay");
        assert_eq!(app.pages.len(), 7);
        assert_eq!(app.entities.len(), 7); // 6 real tables + the query-backed join view

        let view = app.entities.iter().find(|e| e.name == "vendor_bills_view").unwrap();
        assert_eq!(view.source_query.as_deref(), Some("vendor_bills_view"));

        let vendor_bills_page = app.pages.iter().find(|p| p.name == "VendorBills").unwrap();
        let report_entity = vendor_bills_page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Report { entity, link_column, .. } => Some((entity, link_column)),
                _ => None,
            })
            .unwrap();
        assert_eq!(report_entity.0, "vendor_bills_view");
        assert_eq!(report_entity.1.as_ref().unwrap().target_page, "BillDetails");

        let bill_details_page = app.pages.iter().find(|p| p.name == "BillDetails").unwrap();
        let button = bill_details_page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Button { label, behavior, .. } => Some((label, behavior)),
                _ => None,
            })
            .unwrap();
        assert_eq!(button.0, "Record a payment");
        assert!(matches!(button.1, ButtonBehavior::Redirect { ref target_page, .. } if target_page == "Payments"));
    }

    #[test]
    fn parses_app_builder_example() {
        let src = include_str!("../examples/app_builder.pgapp");
        let app = parse_app(src).unwrap();
        assert_eq!(app.name, "App Builder");
        assert_eq!(app.queries.len(), 5);
        assert_eq!(app.entities.len(), 4);
        assert_eq!(app.pages.len(), 4);

        let edit_page = app.pages.iter().find(|p| p.name == "EditPage").unwrap();
        assert_eq!(edit_page.components.len(), 5); // text (context), text (preview), text, region, text (add-component)
        let region = edit_page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Region { query, columns, html, .. } => Some((query, columns, html)),
                _ => None,
            })
            .unwrap();
        assert_eq!(region.0, "components_list");
        assert_eq!(*region.1, vec!["id", "kind", "ordinal"]);
        assert_eq!(region.2.class.as_deref(), Some("pgapp-draggable-rows"));
    }

    #[test]
    fn parses_named_queries_and_regions() {
        let src = r#"
            app "Demo" {
                query assignees {
                    sql: "select name as value from people"
                }

                entity "tasks" {
                    field id: id
                    field title: text required
                    field assignee: text
                }

                page "Tasks" {
                    report "Tasks" of tasks {
                        columns: title
                    }
                    form "Tasks" of tasks {
                        fields: title, assignee
                        item assignee as popup from query assignees
                    }
                    query recent {
                        sql: "select id as value, title as label from pgapp_data.demo_tasks order by id desc limit 5"
                    }
                    region "Recently added" from query recent
                }

                page "ProjectTasks" {
                    report "Project Tasks" of tasks {
                        source: query assignees
                        link: title -> page ProjectTasks (assignee: owner)
                    }
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        assert_eq!(app.queries.len(), 1);
        assert_eq!(app.queries[0].name, "assignees");

        let tasks_page = app.pages.iter().find(|p| p.name == "Tasks").unwrap();
        assert_eq!(tasks_page.queries.len(), 1);
        assert_eq!(tasks_page.queries[0].name, "recent");
        let form = tasks_page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Form { item_types, .. } => Some(item_types),
                _ => None,
            })
            .unwrap();
        let assignee = form.get("assignee").unwrap();
        assert_eq!(assignee.kind, "popup");
        assert_eq!(assignee.config["query"], "assignees");
        assert!(tasks_page
            .components
            .iter()
            .any(|c| matches!(c, ComponentDef::Region { query, .. } if query == "recent")));

        let project_tasks = app.pages.iter().find(|p| p.name == "ProjectTasks").unwrap();
        let report = project_tasks
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Report { source_query, link_column, .. } => {
                    Some((source_query, link_column))
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(report.0.as_deref(), Some("assignees"));
        let link = report.1.as_ref().unwrap();
        assert_eq!(link.extra_params, vec![("assignee".to_string(), "owner".to_string())]);
    }

    #[test]
    fn parses_app_settings_auth_and_page_roles() {
        let src = r#"
            app "Demo" {
                theme: vivid
                icons: fontawesome
                chart_lib: canvas_bars
                auth { }

                entity "t" { field id: id field n: integer }

                page "Public" {
                    text "any signed-in user can see this"
                }

                page "AdminOnly" {
                    requires: admin
                    report "Rows" of t { columns: n }
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        assert_eq!(app.theme.as_deref(), Some("vivid"));
        assert_eq!(app.icons.as_deref(), Some("fontawesome"));
        assert_eq!(app.chart_lib.as_deref(), Some("canvas_bars"));
        assert!(app.auth);

        let public = app.pages.iter().find(|p| p.name == "Public").unwrap();
        assert!(public.required_role.is_none());
        let admin_only = app.pages.iter().find(|p| p.name == "AdminOnly").unwrap();
        assert_eq!(admin_only.required_role.as_deref(), Some("admin"));
    }

    #[test]
    fn defaults_when_no_settings_declared() {
        let src = r#"
            app "Demo" {
                entity "t" { field id: id }
                page "P" { text "hi" }
            }
        "#;
        let app = parse_app(src).unwrap();
        assert!(app.theme.is_none());
        assert!(app.icons.is_none());
        assert!(app.chart_lib.is_none());
        assert!(!app.auth);
    }

    #[test]
    fn parses_query_entities_actions_and_dynamic_actions() {
        let src = r#"
            app "Demo" {
                query stats { sql: "select 'a' as label, 1 as value" }

                entity "t" { field id: id field name: text }
                entity "stats_view" from query stats {
                    field label: text
                    field value: integer
                }

                page "P" {
                    report "Stats" of stats_view { columns: label, value }
                    action "Do it" calls run_query (query: "stats")
                    on change of name {
                        set name to "'x'"
                        show name
                        hide name
                        toggle name when "true"
                        refresh stats
                    }
                }
            }
        "#;
        let app = parse_app(src).unwrap();

        let stats_view = app.entities.iter().find(|e| e.name == "stats_view").unwrap();
        assert_eq!(stats_view.source_query.as_deref(), Some("stats"));
        assert!(app.entities.iter().find(|e| e.name == "t").unwrap().source_query.is_none());

        let page = &app.pages[0];
        let action = page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Action { label, name, config, .. } => Some((label, name, config)),
                _ => None,
            })
            .unwrap();
        assert_eq!(action.0, "Do it");
        assert_eq!(action.1, "run_query");
        assert_eq!(action.2["query"], "stats");

        let da = page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::DynamicAction { event, item, ops } => Some((event, item, ops)),
                _ => None,
            })
            .unwrap();
        assert_eq!(da.0, "change");
        assert_eq!(da.1, "name");
        assert_eq!(da.2.len(), 5);
        assert!(matches!(&da.2[0], DaOp::Set { item, expr } if item == "name" && expr == "'x'"));
        assert!(matches!(&da.2[4], DaOp::Refresh(q) if q == "stats"));
    }

    #[test]
    fn parses_both_button_behaviors() {
        let src = r#"
            app "Demo" {
                entity "t" { field id: id field name: text }

                page "List" {
                    button "Create" -> page Entry (name: default_name)
                }

                page "Entry" {
                    form "Entry" of t { fields: name }
                    button "Sync" calls run_query (query: "stats")
                }
            }
        "#;
        let app = parse_app(src).unwrap();

        let list_page = app.pages.iter().find(|p| p.name == "List").unwrap();
        let redirect = list_page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Button { label, behavior, .. } => Some((label, behavior)),
                _ => None,
            })
            .unwrap();
        assert_eq!(redirect.0, "Create");
        match redirect.1 {
            ButtonBehavior::Redirect { target_page, extra_params } => {
                assert_eq!(target_page, "Entry");
                assert_eq!(extra_params, &[("name".to_string(), "default_name".to_string())]);
            }
            other => panic!("expected Redirect, got {other:?}"),
        }

        let entry_page = app.pages.iter().find(|p| p.name == "Entry").unwrap();
        let run_action = entry_page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Button { label, behavior, .. } => Some((label, behavior)),
                _ => None,
            })
            .unwrap();
        assert_eq!(run_action.0, "Sync");
        match run_action.1 {
            ButtonBehavior::RunAction { name, config } => {
                assert_eq!(name, "run_query");
                assert_eq!(config["query"], "stats");
            }
            other => panic!("expected RunAction, got {other:?}"),
        }
    }

    #[test]
    fn a_plain_redirect_button_needs_no_parameters() {
        let src = r#"
            app "Demo" {
                page "P" {
                    button "Go" -> page P
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let button = app.pages[0]
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Button { behavior, .. } => Some(behavior),
                _ => None,
            })
            .unwrap();
        assert!(matches!(button, ButtonBehavior::Redirect { extra_params, .. } if extra_params.is_empty()));
    }

    #[test]
    fn string_literals_support_escaped_quotes_and_backslashes() {
        // \" and \\ are the only two recognized escapes — enough to write
        // a literal JSON body as a config string, which has nowhere else
        // to live given the grammar's flat string-valued configs.
        let src = r#"
            app "Demo" {
                page "P" {
                    action "Ping" calls run_query (query: "q", body: "say \"hi\" and a\\backslash")
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let config = app
            .pages[0]
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Action { config, .. } => Some(config),
                _ => None,
            })
            .unwrap();
        assert_eq!(config["body"], "say \"hi\" and a\\backslash");
    }

    #[test]
    fn parses_a_collection_backed_entity() {
        let src = r#"
            app "Demo" {
                entity "search_results" from collection "search_results" {
                    field title: text
                    field price: text
                }
                entity "t" { field id: id field name: text }

                page "P" {
                    report "Results" of search_results { columns: title, price }
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let coll = app.entities.iter().find(|e| e.name == "search_results").unwrap();
        assert_eq!(coll.source_collection.as_deref(), Some("search_results"));
        assert!(coll.source_query.is_none());
        assert_eq!(coll.fields.len(), 2);

        let t = app.entities.iter().find(|e| e.name == "t").unwrap();
        assert!(t.source_collection.is_none());
        assert!(t.source_query.is_none());
    }

    #[test]
    fn parses_a_hypothetical_new_item_type_with_no_grammar_change() {
        // Proves the grammar doesn't special-case any particular kind:
        // "starfield" isn't a real component, but it parses exactly like
        // one, with a named config blob.
        let src = r#"
            app "Demo" {
                entity "t" { field id: id field n: integer }
                page "P" {
                    form "P" of t {
                        fields: n
                        item n as starfield (density: "12", twinkle: "true")
                    }
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let page = &app.pages[0];
        let form = page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Form { item_types, .. } => Some(item_types),
                _ => None,
            })
            .unwrap();
        let item = form.get("n").unwrap();
        assert_eq!(item.kind, "starfield");
        assert_eq!(item.config["density"], "12");
        assert_eq!(item.config["twinkle"], "true");
    }

    #[test]
    fn parses_every_built_in_chart_type() {
        for chart_type in CHART_TYPES {
            let src = format!(
                r#"
                app "Demo" {{
                    query q {{ sql: "select 1 as label, 2 as value" }}
                    page "P" {{
                        chart "C" from query q {{ type: {chart_type} x: label y: value }}
                    }}
                }}
                "#
            );
            let app = parse_app(&src).unwrap_or_else(|e| panic!("chart type '{chart_type}' failed to parse: {e}"));
            let chart = app.pages[0]
                .components
                .iter()
                .find_map(|c| match c {
                    ComponentDef::Chart { chart_type, .. } => Some(chart_type),
                    _ => None,
                })
                .unwrap();
            assert_eq!(chart, chart_type);
        }
    }

    #[test]
    fn rejects_an_unknown_chart_type() {
        let src = r#"
            app "Demo" {
                query q { sql: "select 1 as label, 2 as value" }
                page "P" {
                    chart "C" from query q { type: bubble x: label y: value }
                }
            }
        "#;
        let err = parse_app(src).unwrap_err().to_string();
        assert!(err.contains("unknown type 'bubble'"), "unexpected error: {err}");
    }

    #[test]
    fn parses_trailing_attrs_suffix_on_any_component() {
        let src = r#"
            app "Demo" {
                query q { sql: "select 1 as label, 2 as value" }
                page "P" {
                    text "hi" attrs (class: "lead", id: "intro", data_foo: "bar")
                    region "R" from query q attrs (id: "sidebar")
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let page = &app.pages[0];

        let text_html = page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Text { html, .. } => Some(html),
                _ => None,
            })
            .unwrap();
        assert_eq!(text_html.class.as_deref(), Some("lead"));
        assert_eq!(text_html.id.as_deref(), Some("intro"));
        assert_eq!(text_html.attrs, vec![("data-foo".to_string(), "bar".to_string())]);

        let region_html = page
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Region { html, .. } => Some(html),
                _ => None,
            })
            .unwrap();
        assert_eq!(region_html.id.as_deref(), Some("sidebar"));
        assert!(region_html.class.is_none());
    }

    #[test]
    fn parses_region_columns_filter() {
        let src = r#"
            app "Demo" {
                query q { sql: "select 1 as a, 2 as b, 3 as c" }
                page "P" {
                    region "R" from query q { columns: a, c }
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let columns = app.pages[0]
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Region { columns, .. } => Some(columns),
                _ => None,
            })
            .unwrap();
        assert_eq!(columns, &vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn parses_field_level_attrs_without_an_item_type_override() {
        let src = r#"
            app "Demo" {
                entity "agents" { field id: id field name: text required field team: text }
                page "P" {
                    editable_table "Agents" of agents {
                        columns: name, team
                        item name attrs (id: "agent-name", class: "wide", data_foo: "bar")
                    }
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let (item_types, field_html) = app.pages[0]
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::EditableTable { item_types, field_html, .. } => Some((item_types, field_html)),
                _ => None,
            })
            .unwrap();

        // No `as <kind>` was given, so no override was recorded — the
        // field still resolves its item type the normal (default) way.
        assert!(!item_types.contains_key("name"));

        let html = field_html.get("name").unwrap();
        assert_eq!(html.id.as_deref(), Some("agent-name"));
        assert_eq!(html.class.as_deref(), Some("wide"));
        assert_eq!(html.attrs, vec![("data-foo".to_string(), "bar".to_string())]);
        assert!(!field_html.contains_key("team"));
    }

    #[test]
    fn combines_item_type_override_with_field_level_attrs() {
        let src = r#"
            app "Demo" {
                entity "agents" { field id: id field name: text required field active: boolean }
                page "P" {
                    form "Edit" of agents {
                        fields: name, active
                        item active as checkbox attrs (class: "wide")
                    }
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let (item_types, field_html) = app.pages[0]
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Form { item_types, field_html, .. } => Some((item_types, field_html)),
                _ => None,
            })
            .unwrap();
        assert_eq!(item_types.get("active").unwrap().kind, "checkbox");
        assert_eq!(field_html.get("active").unwrap().class.as_deref(), Some("wide"));
    }

    #[test]
    fn rejects_a_bare_item_line_with_neither_as_nor_attrs() {
        let src = r#"
            app "Demo" {
                entity "agents" { field id: id field name: text required }
                page "P" {
                    form "Edit" of agents {
                        fields: name
                        item name
                    }
                }
            }
        "#;
        let err = parse_app(src).unwrap_err().to_string();
        assert!(err.contains("needs 'as <kind>' and/or 'attrs (...)'"), "unexpected error: {err}");
    }

    #[test]
    fn parses_computed_columns_and_format_masks() {
        let src = r#"
            app "Demo" {
                entity "bills" { field id: id field bill_key: text required field amount: integer }
                page "P" {
                    report "Bills" of bills {
                        columns: bill_key, amount, tax
                        computed tax: "(t.amount * 0.1)"
                        format amount: currency
                        format tax: number(2)
                    }
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let (columns, computed, formats) = app.pages[0]
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Report { columns, computed, formats, .. } => Some((columns, computed, formats)),
                _ => None,
            })
            .unwrap();
        assert_eq!(*columns, vec!["bill_key", "amount", "tax"]);
        assert_eq!(computed.len(), 1);
        assert_eq!(computed[0].name, "tax");
        assert_eq!(computed[0].sql, "(t.amount * 0.1)");
        assert_eq!(formats.get("amount"), Some(&FormatMask::Currency));
        assert_eq!(formats.get("tax"), Some(&FormatMask::Number { decimals: 2 }));
    }

    #[test]
    fn parses_date_format_mask_with_custom_pattern() {
        let src = r#"
            app "Demo" {
                entity "trips" { field id: id field trip_start_date: text }
                page "P" {
                    report "Trips" of trips {
                        columns: trip_start_date
                        format trip_start_date: date("%m/%d/%Y")
                    }
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let formats = app.pages[0]
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Report { formats, .. } => Some(formats),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            formats.get("trip_start_date"),
            Some(&FormatMask::Date { pattern: "%m/%d/%Y".to_string() })
        );
    }

    #[test]
    fn parses_component_level_requires_in_either_order_relative_to_attrs() {
        let src = r#"
            app "Demo" {
                auth { }
                entity "bills" { field id: id field amount: integer }
                page "P" {
                    button "Approve" calls noop requires: finance
                    button "Reject" calls noop attrs (class: "x") requires: finance
                    button "Escalate" calls noop requires: finance attrs (class: "y")
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let page = &app.pages[0];
        let requires_of = |i: usize| match &page.components[i] {
            ComponentDef::Button { requires, .. } => requires.clone(),
            _ => panic!("expected a button"),
        };
        assert_eq!(requires_of(0), Some("finance".to_string()));
        assert_eq!(requires_of(1), Some("finance".to_string()));
        assert_eq!(requires_of(2), Some("finance".to_string()));
        match &page.components[2] {
            ComponentDef::Button { html, .. } => assert_eq!(html.class.as_deref(), Some("y")),
            _ => panic!("expected a button"),
        }
    }

    #[test]
    fn parses_an_auth_scheme_declaration() {
        let src = r#"
            app "Demo" {
                auth { }
                auth_scheme "can_approve" {
                    roles: finance, manager
                }
                entity "bills" { field id: id }
                page "P" {
                    text "hi"
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        assert_eq!(app.auth_schemes.len(), 1);
        assert_eq!(app.auth_schemes[0].name, "can_approve");
        assert_eq!(app.auth_schemes[0].roles, vec!["finance".to_string(), "manager".to_string()]);
    }

    #[test]
    fn rejects_an_auth_scheme_with_no_roles() {
        let src = r#"
            app "Demo" {
                auth_scheme "empty" { }
                entity "e" { field id: id }
            }
        "#;
        let err = parse_app(src).unwrap_err().to_string();
        assert!(err.contains("declares no roles"), "unexpected error: {err}");
    }

    #[test]
    fn fragments_accept_and_merge_a_top_level_auth_scheme() {
        let fragment = parse_fragment(r#"auth_scheme "can_approve" { roles: finance, manager }"#).unwrap();
        assert_eq!(fragment.auth_schemes.len(), 1);
        assert_eq!(fragment.auth_schemes[0].name, "can_approve");
        assert_eq!(fragment.auth_schemes[0].roles, vec!["finance".to_string(), "manager".to_string()]);
    }

    #[test]
    fn fragments_reject_a_top_level_auth_block() {
        let err = parse_fragment(r#"auth { }"#).unwrap_err().to_string();
        assert!(err.contains("'auth'"), "unexpected error: {err}");
    }

    #[test]
    fn parses_the_date_item_type_with_and_without_min_max_config() {
        let src = r#"
            app "Demo" {
                entity "trips" { field id: id field start_date: text }
                page "P" {
                    form "Edit" of trips {
                        fields: start_date
                        item start_date as date (min: "2020-01-01", max: "2030-12-31")
                    }
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        let item_types = app.pages[0]
            .components
            .iter()
            .find_map(|c| match c {
                ComponentDef::Form { item_types, .. } => Some(item_types),
                _ => None,
            })
            .unwrap();
        let item = item_types.get("start_date").unwrap();
        assert_eq!(item.kind, "date");
        assert_eq!(item.config["min"], "2020-01-01");
        assert_eq!(item.config["max"], "2030-12-31");
    }
}
