use std::fmt;
use std::iter::Peekable;
use std::str::Chars;

use anyhow::{bail, Result};
use serde::Deserialize;

use crate::tokenizer::{normalize_token, tokenize};

pub type PostingSet = Vec<u32>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Query {
    Term(String),
    And(Vec<Query>),
    Or(Vec<Query>),
    Not(Box<Query>),
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Query::Term(term) => write!(f, "{term}"),
            Query::And(children) => {
                for (i, child) in children.iter().enumerate() {
                    if i > 0 {
                        write!(f, " AND ")?;
                    }
                    write_child(f, child)?;
                }
                Ok(())
            }
            Query::Or(children) => {
                for (i, child) in children.iter().enumerate() {
                    if i > 0 {
                        write!(f, " OR ")?;
                    }
                    write_child(f, child)?;
                }
                Ok(())
            }
            Query::Not(inner) => {
                write!(f, "NOT ")?;
                write_child(f, inner)
            }
        }
    }
}

fn write_child(f: &mut fmt::Formatter<'_>, child: &Query) -> fmt::Result {
    match child {
        Query::Term(_) => write!(f, "{child}"),
        _ => write!(f, "({child})"),
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum QueryJson {
    Term { term: String },
    And { and: Vec<QueryJson> },
    Or { or: Vec<QueryJson> },
    Not { not: Box<QueryJson> },
}

impl From<QueryJson> for Query {
    fn from(json: QueryJson) -> Self {
        match json {
            QueryJson::Term { term } => parse_term_string(&term),
            QueryJson::And { and } => {
                let children: Vec<Query> = and.into_iter().map(Query::from).collect();
                if children.len() == 1 {
                    children.into_iter().next().unwrap()
                } else {
                    Query::And(children)
                }
            }
            QueryJson::Or { or } => {
                let children: Vec<Query> = or.into_iter().map(Query::from).collect();
                if children.len() == 1 {
                    children.into_iter().next().unwrap()
                } else {
                    Query::Or(children)
                }
            }
            QueryJson::Not { not } => Query::Not(Box::new(Query::from(*not))),
        }
    }
}

/// Parse a term string (possibly containing separators) into a Query.
fn parse_term_string(term: &str) -> Query {
    let mut scratch = String::new();
    let tokens: Vec<String> = tokenize(term)
        .map(|t| normalize_token(t, &mut scratch).to_string())
        .collect();

    match tokens.len() {
        0 => Query::Term(String::new()),
        1 => Query::Term(tokens.into_iter().next().unwrap()),
        _ => Query::And(tokens.into_iter().map(Query::Term).collect()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Term(String),
    And,
    Or,
    Not,
    LParen,
    RParen,
    Eof,
}

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn lex(input: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }

        match c {
            '(' => {
                chars.next();
                tokens.push(Token::LParen);
            }
            ')' => {
                chars.next();
                tokens.push(Token::RParen);
            }
            _ if is_token_char(c) => {
                let word = read_word(&mut chars);
                let upper = word.to_ascii_uppercase();
                match upper.as_str() {
                    "AND" => tokens.push(Token::And),
                    "OR" => tokens.push(Token::Or),
                    "NOT" => tokens.push(Token::Not),
                    _ => {
                        let mut scratch = String::new();
                        let normalized = normalize_token(&word, &mut scratch).to_string();
                        tokens.push(Token::Term(normalized));
                    }
                }
            }
            _ => {
                // Separator: skip
                chars.next();
            }
        }
    }

    tokens.push(Token::Eof);
    Ok(tokens)
}

fn read_word(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut word = String::new();
    while let Some(&c) = chars.peek() {
        if is_token_char(c) {
            word.push(c);
            chars.next();
        } else {
            break;
        }
    }
    word
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.peek().clone();
        if !matches!(tok, Token::Eof) {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: Token) -> Result<()> {
        let got = self.advance();
        if got == expected {
            Ok(())
        } else {
            bail!("unexpected token in query: expected {expected:?}, got {got:?}");
        }
    }

    fn parse_or(&mut self) -> Result<Query> {
        let mut children = vec![self.parse_and()?];

        while matches!(self.peek(), Token::Or) {
            self.advance();
            children.push(self.parse_and()?);
        }

        if children.len() == 1 {
            Ok(children.into_iter().next().unwrap())
        } else {
            Ok(Query::Or(children))
        }
    }

    fn parse_and(&mut self) -> Result<Query> {
        let mut children = vec![self.parse_not()?];

        while !matches!(self.peek(), Token::Or | Token::RParen | Token::Eof) {
            if matches!(self.peek(), Token::And) {
                self.advance();
            } else if self.starts_operand() {
                // implicit AND
            } else {
                break;
            }
            children.push(self.parse_not()?);
        }

        if children.len() == 1 {
            Ok(children.into_iter().next().unwrap())
        } else {
            Ok(Query::And(children))
        }
    }

    fn parse_not(&mut self) -> Result<Query> {
        let mut not_count = 0;
        while matches!(self.peek(), Token::Not) {
            self.advance();
            not_count += 1;
        }

        let mut query = self.parse_atom()?;

        for _ in 0..not_count {
            query = Query::Not(Box::new(query));
        }

        Ok(query)
    }

    fn parse_atom(&mut self) -> Result<Query> {
        match self.peek().clone() {
            Token::LParen => {
                self.advance();
                let inner = self.parse_or()?;
                self.expect(Token::RParen)?;
                Ok(inner)
            }
            Token::Term(term) => {
                self.advance();
                Ok(Query::Term(term))
            }
            Token::Eof => bail!("unexpected end of query"),
            other => bail!("unexpected token in query: {other:?}"),
        }
    }

    fn starts_operand(&self) -> bool {
        matches!(
            self.peek(),
            Token::Term(_) | Token::LParen | Token::Not
        )
    }
}

/// Parse an infix query string into a Query AST.
pub fn parse_query(input: &str) -> Result<Query> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(Query::Term(String::new()));
    }

    let tokens = lex(trimmed)?;
    let mut parser = Parser::new(tokens);
    let query = parser.parse_or()?;

    if !matches!(parser.peek(), Token::Eof) {
        bail!("unexpected trailing tokens in query");
    }

    Ok(query)
}

pub trait BlockIndex {
    fn term_ids(&self, token: &str) -> Result<PostingSet>;
    fn num_lines(&self) -> usize;
}

pub fn eval(q: &Query, block: &dyn BlockIndex) -> Result<PostingSet> {
    match q {
        Query::Term(term) if term.is_empty() => Ok(Vec::new()),
        Query::Term(term) => block.term_ids(term),
        Query::And(children) => {
            let mut iter = children.iter();
            let first = iter
                .next()
                .map(|c| eval(c, block))
                .transpose()?
                .unwrap_or_default();
            let mut result = first;
            for child in iter {
                let rhs = eval(child, block)?;
                result = intersect(&result, &rhs);
            }
            Ok(result)
        }
        Query::Or(children) => {
            let mut result = Vec::new();
            for child in children {
                let ids = eval(child, block)?;
                result = union(&result, &ids);
            }
            Ok(result)
        }
        Query::Not(inner) => {
            let positive = eval(inner, block)?;
            let universe: Vec<u32> = (0..block.num_lines() as u32).collect();
            Ok(difference(&universe, &positive))
        }
    }
}

fn intersect(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                result.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    result
}

fn union(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut result = Vec::with_capacity(a.len() + b.len());
    let mut i = 0;
    let mut j = 0;
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                result.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                result.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                result.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    result.extend_from_slice(&a[i..]);
    result.extend_from_slice(&b[j..]);
    result
}

fn difference(universe: &[u32], subset: &[u32]) -> Vec<u32> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < universe.len() && j < subset.len() {
        match universe[i].cmp(&subset[j]) {
            std::cmp::Ordering::Less => {
                result.push(universe[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    result.extend_from_slice(&universe[i..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockBlock {
        inverted: HashMap<String, Vec<u32>>,
        num_lines: usize,
    }

    impl BlockIndex for MockBlock {
        fn term_ids(&self, token: &str) -> Result<PostingSet> {
            Ok(self.inverted.get(token).cloned().unwrap_or_default())
        }

        fn num_lines(&self) -> usize {
            self.num_lines
        }
    }

    fn mock_block(inverted: &[(&str, &[u32])], num_lines: usize) -> MockBlock {
        MockBlock {
            inverted: inverted
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_vec()))
                .collect(),
            num_lines,
        }
    }

    #[test]
    fn single_term() {
        let q = parse_query("error").unwrap();
        assert_eq!(q, Query::Term("error".to_string()));
    }

    #[test]
    fn implicit_and() {
        let q = parse_query("auth error").unwrap();
        assert_eq!(
            q,
            Query::And(vec![
                Query::Term("auth".to_string()),
                Query::Term("error".to_string())
            ])
        );
    }

    #[test]
    fn or_with_implicit_and_precedence() {
        let q = parse_query("auth OR error debug").unwrap();
        assert_eq!(
            q,
            Query::Or(vec![
                Query::Term("auth".to_string()),
                Query::And(vec![
                    Query::Term("error".to_string()),
                    Query::Term("debug".to_string())
                ])
            ])
        );
    }

    #[test]
    fn explicit_and_or() {
        let q = parse_query("a AND b OR c").unwrap();
        assert_eq!(
            q,
            Query::Or(vec![
                Query::And(vec![
                    Query::Term("a".to_string()),
                    Query::Term("b".to_string())
                ]),
                Query::Term("c".to_string())
            ])
        );
    }

    #[test]
    fn not_with_parens() {
        let q = parse_query("NOT (a OR b)").unwrap();
        assert_eq!(
            q,
            Query::Not(Box::new(Query::Or(vec![
                Query::Term("a".to_string()),
                Query::Term("b".to_string())
            ])))
        );
    }

    #[test]
    fn env_equals_prod_splits_to_implicit_and() {
        let q = parse_query("env=prod").unwrap();
        assert_eq!(
            q,
            Query::And(vec![
                Query::Term("env".to_string()),
                Query::Term("prod".to_string())
            ])
        );
    }

    #[test]
    fn empty_query() {
        let q = parse_query("").unwrap();
        assert_eq!(q, Query::Term(String::new()));
    }

    #[test]
    fn case_insensitive_operators() {
        let q = parse_query("a and b or c").unwrap();
        assert_eq!(
            q,
            Query::Or(vec![
                Query::And(vec![
                    Query::Term("a".to_string()),
                    Query::Term("b".to_string())
                ]),
                Query::Term("c".to_string())
            ])
        );
    }

    #[test]
    fn eval_and() {
        let block = mock_block(&[("a", &[0, 2]), ("b", &[1, 2])], 4);
        let q = Query::And(vec![
            Query::Term("a".to_string()),
            Query::Term("b".to_string()),
        ]);
        assert_eq!(eval(&q, &block).unwrap(), vec![2]);
    }

    #[test]
    fn eval_or() {
        let block = mock_block(&[("a", &[0, 2]), ("b", &[1, 3])], 4);
        let q = Query::Or(vec![
            Query::Term("a".to_string()),
            Query::Term("b".to_string()),
        ]);
        assert_eq!(eval(&q, &block).unwrap(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn eval_not() {
        let block = mock_block(&[("a", &[0, 2])], 4);
        let q = Query::Not(Box::new(Query::Term("a".to_string())));
        assert_eq!(eval(&q, &block).unwrap(), vec![1, 3]);
    }

    #[test]
    fn json_ast_term() {
        let json: QueryJson = serde_json::from_str(r#"{"term":"error"}"#).unwrap();
        assert_eq!(Query::from(json), Query::Term("error".to_string()));
    }

    #[test]
    fn json_ast_and() {
        let json: QueryJson =
            serde_json::from_str(r#"{"and":[{"term":"a"},{"term":"b"}]}"#).unwrap();
        assert_eq!(
            Query::from(json),
            Query::And(vec![
                Query::Term("a".to_string()),
                Query::Term("b".to_string())
            ])
        );
    }

    #[test]
    fn display_roundtrip() {
        let q = parse_query("auth OR (error AND debug)").unwrap();
        assert_eq!(q.to_string(), "auth OR (error AND debug)");
    }

    #[test]
    fn unbalanced_parens_error() {
        assert!(parse_query("(a OR b").is_err());
    }
}
