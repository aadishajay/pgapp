//! A small, hand-rolled parser for pgapp's APEX-inspired application markup.
//!
//! Grammar (informal):
//!
//! ```text
//! app       := "app" String "{" (nav | entity | page)* "}"
//!
//! nav       := "nav" "{" navitem* "}"
//! navitem   := "item" String ( "->" "page" Ident | "{" navitem* "}" )
//!
//! entity    := "entity" String "{" field* "}"
//! field     := "field" Ident ":" Ident ("required")? ("default" Value)?
//!
//! page      := "page" String "as" pagekind "{" pageprop* "}"
//! pagekind  := "list" "of" Ident | "detail" "of" Ident | "static"
//! pageprop  := "columns" ":" identlist
//!            | "form" ":" identlist
//!            | "link" ":" Ident "->" "page" Ident
//!            | "items" "{" item* "}"
//! item      := "text" String | "link" String "->" "page" Ident
//!
//! identlist := Ident ("," Ident)*
//! value     := Ident | Number
//! ```
//!
//! `Ident` tokens are restricted to `[A-Za-z_][A-Za-z0-9_]*`, which means
//! every entity/field/page name that reaches the metadata layer is already
//! safe to splice into SQL as an identifier. Page names themselves are
//! string literals (so they can be arbitrary display text), but anything
//! that *targets* a page — `nav` items, `link` page properties, `link`
//! page items — takes an `Ident`, so link targets are restricted to the
//! same safe charset.

use anyhow::{bail, Context, Result};

use crate::model::{
    AppDef, EntityDef, FieldDef, FieldType, LinkColumn, NavItem, PageDef, PageItem, PageKind,
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
        } else if c == '{' || c == '}' || c == ':' || c == ',' {
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
        while !self.at_symbol('}') {
            if self.at_keyword("entity") {
                entities.push(self.parse_entity()?);
            } else if self.at_keyword("page") {
                pages.push(self.parse_page()?);
            } else if self.at_keyword("nav") {
                nav = self.parse_nav()?;
            } else {
                bail!(
                    "expected 'entity', 'page', or 'nav' block, found {:?}",
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
        })
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

            let prop = self.expect_ident()?;
            self.expect_symbol(':')?;
            match prop.as_str() {
                "columns" => columns = self.parse_ident_list()?,
                "form" => form = self.parse_ident_list()?,
                "link" => {
                    let field = self.expect_ident()?;
                    let target_page = self.parse_page_target()?;
                    link_column = Some(LinkColumn { field, target_page });
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
        })
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
        } else {
            bail!("expected 'text' or 'link' page item, found {:?}", self.peek());
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
        assert_eq!(tasks.fields.len(), 4);

        assert_eq!(app.pages.len(), 3);
        let list_page = app.pages.iter().find(|p| p.name == "Tasks").unwrap();
        assert_eq!(list_page.kind, PageKind::List);
        assert_eq!(list_page.columns, vec!["title", "done", "created_at"]);
        assert_eq!(list_page.form, vec!["title", "done"]);
        let link = list_page.link_column.as_ref().unwrap();
        assert_eq!(link.field, "title");
        assert_eq!(link.target_page, "TaskDetail");

        let detail_page = app.pages.iter().find(|p| p.name == "TaskDetail").unwrap();
        assert_eq!(detail_page.kind, PageKind::Detail);
        assert_eq!(detail_page.entity.as_deref(), Some("tasks"));

        let about_page = app.pages.iter().find(|p| p.name == "About").unwrap();
        assert_eq!(about_page.kind, PageKind::Static);
        assert!(about_page.entity.is_none());
        assert_eq!(about_page.items.len(), 2);
        assert!(matches!(&about_page.items[0], PageItem::Text(_)));
        assert!(matches!(&about_page.items[1], PageItem::Link { .. }));

        assert_eq!(app.nav.len(), 2);
        assert_eq!(app.nav[0].label, "Tasks");
        assert_eq!(app.nav[0].target_page.as_deref(), Some("Tasks"));
        assert_eq!(app.nav[1].label, "More");
        assert!(app.nav[1].target_page.is_none());
        assert_eq!(app.nav[1].children.len(), 1);
        assert_eq!(app.nav[1].children[0].label, "About");
    }
}
