//! A small, hand-rolled parser for pgapp's APEX-inspired application markup.
//!
//! Grammar (informal):
//!
//! ```text
//! app       := "app" String "{" (nav | header | footer | entity | page | query)* "}"
//!
//! nav       := "nav" "{" navitem* "}"
//! navitem   := "item" String ( "->" "page" Ident | "{" navitem* "}" )
//!
//! header    := "header" "{" item* "}"
//! footer    := "footer" "{" item* "}"
//!
//! query     := "query" Ident "{" "sql" ":" String "}"
//!
//! entity    := "entity" String "{" field* "}"
//! field     := "field" Ident ":" Ident ("required")? ("default" Value)?
//!
//! page      := "page" String "as" pagekind "{" pageprop* "}"
//! pagekind  := "list" "of" Ident | "detail" "of" Ident | "static"
//! pageprop  := "columns" ":" identlist
//!            | "form" ":" identlist
//!            | "source" ":" "query" Ident
//!            | "link" ":" Ident "->" "page" Ident ( "(" paramlist ")" )?
//!            | "items" "{" item* "}"
//!            | "item" Ident "as" itemtype
//!            | query
//! itemtype  := "text" | "readonly" | "checkbox"
//!            | "radio" choicesource
//!            | "popup" choicesource
//! choicesource := "(" stringlist ")" | "from" "query" Ident
//! item      := "text" String
//!            | "link" String "->" "page" Ident
//!            | "region" String "from" "query" Ident
//!
//! identlist  := Ident ("," Ident)*
//! stringlist := String ("," String)*
//! paramlist  := parammap ("," parammap)*
//! parammap   := Ident ":" Ident
//! value      := Ident | Number
//! ```
//!
//! `Ident` tokens are restricted to `[A-Za-z_][A-Za-z0-9_]*`, which means
//! every entity/field/page/query name that reaches the metadata layer is
//! already safe to splice into SQL as an identifier. Page names
//! themselves are string literals (so they can be arbitrary display
//! text), but anything that *targets* a page — `nav` items, `link` page
//! properties, `link` page items — takes an `Ident`, so link targets are
//! restricted to the same safe charset. A query's `sql` is a raw string,
//! opaque to this parser — see `meta::compile_named_query` for how its
//! `:name` bind markers get turned into safe positional parameters.

use anyhow::{bail, Context, Result};

use crate::model::{
    AppDef, ChoiceSource, EntityDef, FieldDef, FieldItemType, FieldType, LinkColumn, NavItem,
    PageDef, PageItem, PageKind, QueryDef,
};

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Str(String),
    Symbol(char),
    Arrow,
}

fn lex(src: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = src.chars().collect();
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
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            if i >= chars.len() {
                bail!("unterminated string literal");
            }
            tokens.push(Token::Str(chars[start..i].iter().collect()));
            i += 1;
        } else if c == '-' && chars.get(i + 1) == Some(&'>') {
            tokens.push(Token::Arrow);
            i += 2;
        } else if c == '{' || c == '}' || c == ':' || c == ',' || c == '(' || c == ')' {
            tokens.push(Token::Symbol(c));
            i += 1;
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect()));
        } else {
            bail!("unexpected character '{c}' in markup");
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Result<Token> {
        let t = self
            .tokens
            .get(self.pos)
            .cloned()
            .context("unexpected end of markup")?;
        self.pos += 1;
        Ok(t)
    }

    fn expect_symbol(&mut self, c: char) -> Result<()> {
        match self.advance()? {
            Token::Symbol(s) if s == c => Ok(()),
            other => bail!("expected '{c}', found {other:?}"),
        }
    }

    fn expect_arrow(&mut self) -> Result<()> {
        match self.advance()? {
            Token::Arrow => Ok(()),
            other => bail!("expected '->', found {other:?}"),
        }
    }

    fn expect_keyword(&mut self, word: &str) -> Result<()> {
        match self.advance()? {
            Token::Ident(s) if s == word => Ok(()),
            other => bail!("expected keyword '{word}', found {other:?}"),
        }
    }

    fn expect_ident(&mut self) -> Result<String> {
        match self.advance()? {
            Token::Ident(s) => Ok(s),
            other => bail!("expected identifier, found {other:?}"),
        }
    }

    fn expect_string(&mut self) -> Result<String> {
        match self.advance()? {
            Token::Str(s) => Ok(s),
            other => bail!("expected string literal, found {other:?}"),
        }
    }

    fn at_symbol(&self, c: char) -> bool {
        matches!(self.peek(), Some(Token::Symbol(s)) if *s == c)
    }

    fn at_keyword(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Ident(s)) if s == word)
    }

    /// Parses a `-> page <Ident>` link target, common to nav items, the
    /// `link:` page property, and `link` page items.
    fn parse_page_target(&mut self) -> Result<String> {
        self.expect_arrow()?;
        self.expect_keyword("page")?;
        self.expect_ident()
    }

    fn parse_app(&mut self) -> Result<AppDef> {
        self.expect_keyword("app")?;
        let name = self.expect_string()?;
        self.expect_symbol('{')?;

        let mut entities = Vec::new();
        let mut pages = Vec::new();
        let mut nav = Vec::new();
        let mut header = Vec::new();
        let mut footer = Vec::new();
        let mut queries = Vec::new();
        while !self.at_symbol('}') {
            if self.at_keyword("entity") {
                entities.push(self.parse_entity()?);
            } else if self.at_keyword("page") {
                pages.push(self.parse_page()?);
            } else if self.at_keyword("nav") {
                nav = self.parse_nav()?;
            } else if self.at_keyword("header") {
                header = self.parse_item_block("header")?;
            } else if self.at_keyword("footer") {
                footer = self.parse_item_block("footer")?;
            } else if self.at_keyword("query") {
                queries.push(self.parse_query()?);
            } else {
                bail!(
                    "expected 'entity', 'page', 'nav', 'header', 'footer', or 'query' block, found {:?}",
                    self.peek()
                );
            }
        }
        self.expect_symbol('}')?;

        Ok(AppDef {
            name,
            entities,
            pages,
            nav,
            header,
            footer,
            queries,
        })
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

    /// Parses `"header" "{" item* "}"` / `"footer" "{" item* "}"`.
    fn parse_item_block(&mut self, keyword: &str) -> Result<Vec<PageItem>> {
        self.expect_keyword(keyword)?;
        self.expect_symbol('{')?;
        let mut items = Vec::new();
        while !self.at_symbol('}') {
            items.push(self.parse_page_item()?);
        }
        self.expect_symbol('}')?;
        Ok(items)
    }

    fn parse_entity(&mut self) -> Result<EntityDef> {
        self.expect_keyword("entity")?;
        let name = self.expect_string()?;
        self.expect_symbol('{')?;

        let mut fields = Vec::new();
        while !self.at_symbol('}') {
            fields.push(self.parse_field()?);
        }
        self.expect_symbol('}')?;

        Ok(EntityDef { name, fields })
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
        self.expect_keyword("as")?;

        let (kind, entity) = if self.at_keyword("list") {
            self.advance()?;
            self.expect_keyword("of")?;
            (PageKind::List, Some(self.expect_ident()?))
        } else if self.at_keyword("detail") {
            self.advance()?;
            self.expect_keyword("of")?;
            (PageKind::Detail, Some(self.expect_ident()?))
        } else if self.at_keyword("static") {
            self.advance()?;
            (PageKind::Static, None)
        } else {
            bail!(
                "expected 'list', 'detail', or 'static' page kind, found {:?}",
                self.peek()
            );
        };

        self.expect_symbol('{')?;

        let mut columns = Vec::new();
        let mut form = Vec::new();
        let mut link_column = None;
        let mut items = Vec::new();
        let mut item_types = std::collections::HashMap::new();
        let mut queries = Vec::new();
        let mut source_query = None;
        while !self.at_symbol('}') {
            if self.at_keyword("items") {
                self.advance()?;
                self.expect_symbol('{')?;
                while !self.at_symbol('}') {
                    items.push(self.parse_page_item()?);
                }
                self.expect_symbol('}')?;
                continue;
            }
            if self.at_keyword("item") {
                let (field, item_type) = self.parse_field_item()?;
                item_types.insert(field, item_type);
                continue;
            }
            if self.at_keyword("query") {
                queries.push(self.parse_query()?);
                continue;
            }

            let prop = self.expect_ident()?;
            self.expect_symbol(':')?;
            match prop.as_str() {
                "columns" => columns = self.parse_ident_list()?,
                "form" => form = self.parse_ident_list()?,
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
                other => bail!("unknown page property '{other}'"),
            }
        }
        self.expect_symbol('}')?;

        Ok(PageDef {
            name,
            kind,
            entity,
            columns,
            form,
            link_column,
            items,
            item_types,
            queries,
            source_query,
        })
    }

    /// Parses `"item" Ident "as" itemtype`.
    fn parse_field_item(&mut self) -> Result<(String, FieldItemType)> {
        self.expect_keyword("item")?;
        let field = self.expect_ident()?;
        self.expect_keyword("as")?;

        let item_type = if self.at_keyword("text") {
            self.advance()?;
            FieldItemType::Text
        } else if self.at_keyword("readonly") {
            self.advance()?;
            FieldItemType::ReadOnly
        } else if self.at_keyword("checkbox") {
            self.advance()?;
            FieldItemType::Checkbox
        } else if self.at_keyword("radio") {
            self.advance()?;
            FieldItemType::Radio(self.parse_choice_source()?)
        } else if self.at_keyword("popup") {
            self.advance()?;
            FieldItemType::Popup(self.parse_choice_source()?)
        } else {
            bail!(
                "expected 'text', 'readonly', 'checkbox', 'radio', or 'popup' item type, found {:?}",
                self.peek()
            );
        };

        Ok((field, item_type))
    }

    /// Parses a Radio/Popup choice source: either a literal `(...)` list
    /// or `from query <Ident>`.
    fn parse_choice_source(&mut self) -> Result<ChoiceSource> {
        if self.at_keyword("from") {
            self.advance()?;
            self.expect_keyword("query")?;
            Ok(ChoiceSource::Query(self.expect_ident()?))
        } else {
            Ok(ChoiceSource::Static(self.parse_choice_list()?))
        }
    }

    fn parse_choice_list(&mut self) -> Result<Vec<String>> {
        self.expect_symbol('(')?;
        let mut out = vec![self.expect_string()?];
        while self.at_symbol(',') {
            self.advance()?;
            out.push(self.expect_string()?);
        }
        self.expect_symbol(')')?;
        Ok(out)
    }

    /// Parses `"(" Ident ":" Ident ("," Ident ":" Ident)* ")"` — the
    /// optional extra parameters on a `link:` page property.
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

    fn parse_page_item(&mut self) -> Result<PageItem> {
        if self.at_keyword("text") {
            self.advance()?;
            Ok(PageItem::Text(self.expect_string()?))
        } else if self.at_keyword("link") {
            self.advance()?;
            let label = self.expect_string()?;
            let target_page = self.parse_page_target()?;
            Ok(PageItem::Link { label, target_page })
        } else if self.at_keyword("region") {
            self.advance()?;
            let label = self.expect_string()?;
            self.expect_keyword("from")?;
            self.expect_keyword("query")?;
            let query = self.expect_ident()?;
            Ok(PageItem::Region { label, query })
        } else {
            bail!(
                "expected 'text', 'link', or 'region' page item, found {:?}",
                self.peek()
            );
        }
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
    let tokens = lex(src)?;
    let mut parser = Parser { tokens, pos: 0 };
    let app = parser.parse_app()?;
    if parser.pos != parser.tokens.len() {
        bail!("unexpected trailing content after app block");
    }
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PageItem;

    #[test]
    fn parses_todo_example() {
        let src = include_str!("../examples/todo.app");
        let app = parse_app(src).unwrap();
        assert_eq!(app.name, "Todo");

        assert_eq!(app.entities.len(), 1);
        let tasks = &app.entities[0];
        assert_eq!(tasks.name, "tasks");
        assert_eq!(tasks.fields.len(), 7);

        assert_eq!(app.header.len(), 1);
        assert!(matches!(&app.header[0], PageItem::Text(_)));
        assert_eq!(app.footer.len(), 2);
        assert!(matches!(&app.footer[0], PageItem::Text(_)));
        assert!(matches!(&app.footer[1], PageItem::Link { .. }));

        assert_eq!(app.queries.len(), 1);
        assert_eq!(app.queries[0].name, "assignees");

        assert_eq!(app.pages.len(), 4);
        let list_page = app.pages.iter().find(|p| p.name == "Tasks").unwrap();
        assert_eq!(list_page.kind, PageKind::List);
        assert_eq!(
            list_page.columns,
            vec!["title", "priority", "done", "created_at"]
        );
        assert_eq!(
            list_page.form,
            vec!["title", "priority", "done", "assignee", "notes"]
        );
        let link = list_page.link_column.as_ref().unwrap();
        assert_eq!(link.field, "title");
        assert_eq!(link.target_page, "TaskDetail");
        assert_eq!(link.extra_params, vec![("priority".to_string(), "priority".to_string())]);

        assert!(matches!(
            list_page.item_types.get("priority"),
            Some(FieldItemType::Radio(_))
        ));
        assert!(matches!(
            list_page.item_types.get("assignee"),
            Some(FieldItemType::Popup(_))
        ));
        assert!(matches!(
            list_page.item_types.get("notes"),
            Some(FieldItemType::ReadOnly)
        ));
        assert!(list_page.item_types.get("title").is_none());
        assert!(list_page.item_types.get("done").is_none());
        if let Some(FieldItemType::Radio(ChoiceSource::Static(choices))) =
            list_page.item_types.get("priority")
        {
            assert_eq!(choices, &vec!["Low", "Medium", "High"]);
        } else {
            panic!("expected a static radio choice list for 'priority'");
        }
        assert!(matches!(
            list_page.item_types.get("assignee"),
            Some(FieldItemType::Popup(ChoiceSource::Query(name))) if name == "assignees"
        ));

        assert_eq!(list_page.queries.len(), 1);
        assert_eq!(list_page.queries[0].name, "recent");
        assert!(list_page
            .items
            .iter()
            .any(|item| matches!(item, PageItem::Region { query, .. } if query == "recent")));

        let detail_page = app.pages.iter().find(|p| p.name == "TaskDetail").unwrap();
        assert_eq!(detail_page.kind, PageKind::Detail);
        assert_eq!(detail_page.entity.as_deref(), Some("tasks"));
        assert_eq!(detail_page.queries.len(), 1);
        assert_eq!(detail_page.queries[0].name, "siblings");

        let open_tasks = app.pages.iter().find(|p| p.name == "OpenTasks").unwrap();
        assert_eq!(open_tasks.kind, PageKind::List);
        assert_eq!(open_tasks.source_query.as_deref(), Some("open"));
        assert_eq!(open_tasks.queries.len(), 1);
        assert_eq!(open_tasks.queries[0].name, "open");

        let about_page = app.pages.iter().find(|p| p.name == "About").unwrap();
        assert_eq!(about_page.kind, PageKind::Static);
        assert!(about_page.entity.is_none());
        assert_eq!(about_page.items.len(), 2);
        assert!(matches!(&about_page.items[0], PageItem::Text(_)));
        assert!(matches!(&about_page.items[1], PageItem::Link { .. }));

        assert_eq!(app.nav.len(), 3);
        assert_eq!(app.nav[0].label, "Tasks");
        assert_eq!(app.nav[0].target_page.as_deref(), Some("Tasks"));
        assert_eq!(app.nav[1].label, "Open");
        assert_eq!(app.nav[2].label, "More");
        assert!(app.nav[2].target_page.is_none());
        assert_eq!(app.nav[2].children.len(), 1);
        assert_eq!(app.nav[2].children[0].label, "About");
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

                page "Tasks" as list of tasks {
                    columns: title
                    form: title, assignee
                    item assignee as popup from query assignees
                    query recent {
                        sql: "select id as value, title as label from pgapp_data.demo_tasks order by id desc limit 5"
                    }
                    items {
                        region "Recently added" from query recent
                    }
                }

                page "ProjectTasks" as list of tasks {
                    source: query assignees
                    link: title -> page ProjectTasks (assignee: owner)
                }
            }
        "#;
        let app = parse_app(src).unwrap();
        assert_eq!(app.queries.len(), 1);
        assert_eq!(app.queries[0].name, "assignees");

        let tasks_page = app.pages.iter().find(|p| p.name == "Tasks").unwrap();
        assert_eq!(tasks_page.queries.len(), 1);
        assert_eq!(tasks_page.queries[0].name, "recent");
        assert!(matches!(
            tasks_page.item_types.get("assignee"),
            Some(FieldItemType::Popup(ChoiceSource::Query(name))) if name == "assignees"
        ));
        assert!(matches!(&tasks_page.items[0], PageItem::Region { query, .. } if query == "recent"));

        let project_tasks = app.pages.iter().find(|p| p.name == "ProjectTasks").unwrap();
        assert_eq!(project_tasks.source_query.as_deref(), Some("assignees"));
        let link = project_tasks.link_column.as_ref().unwrap();
        assert_eq!(link.extra_params, vec![("assignee".to_string(), "owner".to_string())]);
    }
}
