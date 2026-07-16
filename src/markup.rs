//! A small, hand-rolled parser for pgapp's APEX-inspired application markup.
//!
//! Grammar (informal):
//!
//! ```text
//! app       := "app" String "{" (entity | page)* "}"
//! entity    := "entity" String "{" field* "}"
//! field     := "field" Ident ":" Ident ("required")? ("default" Value)?
//! page      := "page" String "as" "list" "of" Ident "{" pageprop* "}"
//! pageprop  := Ident ":" identlist
//! identlist := Ident ("," Ident)*
//! value     := Ident | Number
//! ```
//!
//! `Ident` tokens are restricted to `[A-Za-z_][A-Za-z0-9_]*`, which means
//! every entity/field/page name that reaches the metadata layer is already
//! safe to splice into SQL as an identifier.

use anyhow::{bail, Context, Result};

use crate::model::{AppDef, EntityDef, FieldDef, FieldType, PageDef};

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Str(String),
    Symbol(char),
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

    fn parse_app(&mut self) -> Result<AppDef> {
        self.expect_keyword("app")?;
        let name = self.expect_string()?;
        self.expect_symbol('{')?;

        let mut entities = Vec::new();
        let mut pages = Vec::new();
        while !self.at_symbol('}') {
            if self.at_keyword("entity") {
                entities.push(self.parse_entity()?);
            } else if self.at_keyword("page") {
                pages.push(self.parse_page()?);
            } else {
                bail!("expected 'entity' or 'page' block, found {:?}", self.peek());
            }
        }
        self.expect_symbol('}')?;

        Ok(AppDef {
            name,
            entities,
            pages,
        })
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
        self.expect_keyword("list")?;
        self.expect_keyword("of")?;
        let entity = self.expect_ident()?;
        self.expect_symbol('{')?;

        let mut columns = Vec::new();
        let mut form = Vec::new();
        while !self.at_symbol('}') {
            let prop = self.expect_ident()?;
            self.expect_symbol(':')?;
            let list = self.parse_ident_list()?;
            match prop.as_str() {
                "columns" => columns = list,
                "form" => form = list,
                other => bail!("unknown page property '{other}'"),
            }
        }
        self.expect_symbol('}')?;

        Ok(PageDef {
            name,
            entity,
            columns,
            form,
        })
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

    #[test]
    fn parses_todo_example() {
        let src = include_str!("../examples/todo.app");
        let app = parse_app(src).unwrap();
        assert_eq!(app.name, "Todo");
        assert_eq!(app.entities.len(), 1);
        let tasks = &app.entities[0];
        assert_eq!(tasks.name, "tasks");
        assert_eq!(tasks.fields.len(), 4);
        assert_eq!(app.pages.len(), 1);
        assert_eq!(app.pages[0].columns, vec!["title", "done", "created_at"]);
        assert_eq!(app.pages[0].form, vec!["title", "done"]);
    }
}
