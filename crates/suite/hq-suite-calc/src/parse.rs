//! Formula text to a tree.

use crate::value::CalcError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellRef {
    /// Zero-based column (A is 0).
    pub col: u32,
    /// Zero-based row (1 is 0).
    pub row: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Text(String),
    Bool(bool),
    Ref(CellRef),
    Range(CellRef, CellRef),
    Neg(Box<Expr>),
    Percent(Box<Expr>),
    Binary(char, Box<Expr>, Box<Expr>),
    /// `<>`, `<=` and `>=` are written as `n`, `l` and `g`.
    Compare(char, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ParseError(pub String);

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Text(String),
    Ident(String),
    Op(char),
    Cmp(char),
    LParen,
    RParen,
    Comma,
    Colon,
}

const MAX_FORMULA_CHARS: usize = 8192;
const MAX_DEPTH: usize = 64;

fn lex(src: &str) -> Result<Vec<Tok>, ParseError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                if i < chars.len()
                    && matches!(chars[i], 'e' | 'E')
                    && chars
                        .get(i + 1)
                        .is_some_and(|n| n.is_ascii_digit() || matches!(n, '+' | '-'))
                {
                    i += 2;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let text: String = chars[start..i].iter().collect();
                out.push(Tok::Num(
                    text.parse()
                        .map_err(|_| ParseError(format!("`{text}` is not a number")))?,
                ));
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(ParseError("a text is not closed".into())),
                        Some('"') if chars.get(i + 1) == Some(&'"') => {
                            s.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(c) => {
                            s.push(*c);
                            i += 1;
                        }
                    }
                }
                out.push(Tok::Text(s));
            }
            'a'..='z' | 'A'..='Z' | '_' | '$' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '$' | '.'))
                {
                    i += 1;
                }
                out.push(Tok::Ident(chars[start..i].iter().collect()));
            }
            '+' | '-' | '*' | '/' | '^' | '&' | '%' => {
                out.push(Tok::Op(c));
                i += 1;
            }
            '=' => {
                out.push(Tok::Cmp('='));
                i += 1;
            }
            '<' | '>' => {
                let next = chars.get(i + 1).copied();
                let (tok, len) = match (c, next) {
                    ('<', Some('>')) => ('n', 2),
                    ('<', Some('=')) => ('l', 2),
                    ('>', Some('=')) => ('g', 2),
                    _ => (c, 1),
                };
                out.push(Tok::Cmp(tok));
                i += len;
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            ',' | ';' => {
                out.push(Tok::Comma);
                i += 1;
            }
            ':' => {
                out.push(Tok::Colon);
                i += 1;
            }
            other => return Err(ParseError(format!("`{other}` is not allowed in a formula"))),
        }
    }
    Ok(out)
}

/// `A1`, `$B$7`, `aa12`.
pub(crate) fn cell_ref(text: &str) -> Option<CellRef> {
    let t = text.replace('$', "");
    let split = t.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = t.split_at(split);
    if letters.is_empty() || !letters.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    if row == 0 {
        return None;
    }
    let mut col = 0u32;
    for c in letters.chars() {
        col = col
            .checked_mul(26)?
            .checked_add(c.to_ascii_uppercase() as u32 - 'A' as u32 + 1)?;
    }
    Some(CellRef {
        col: col - 1,
        row: row - 1,
    })
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ParseError("the formula is nested too deeply".into()));
        }
        Ok(())
    }

    fn compare(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.concat()?;
        while let Some(Tok::Cmp(c)) = self.peek().cloned() {
            self.next();
            left = Expr::Compare(c, Box::new(left), Box::new(self.concat()?));
        }
        Ok(left)
    }

    fn concat(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.additive()?;
        while self.peek() == Some(&Tok::Op('&')) {
            self.next();
            left = Expr::Binary('&', Box::new(left), Box::new(self.additive()?));
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.term()?;
        while let Some(Tok::Op(c @ ('+' | '-'))) = self.peek().cloned() {
            self.next();
            left = Expr::Binary(c, Box::new(left), Box::new(self.term()?));
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.power()?;
        while let Some(Tok::Op(c @ ('*' | '/'))) = self.peek().cloned() {
            self.next();
            left = Expr::Binary(c, Box::new(left), Box::new(self.power()?));
        }
        Ok(left)
    }

    /// `^` is left to right and binds looser than a leading minus: `-2^2` is 4, as in a spreadsheet.
    fn power(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.unary()?;
        while self.peek() == Some(&Tok::Op('^')) {
            self.next();
            left = Expr::Binary('^', Box::new(left), Box::new(self.unary()?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        self.enter()?;
        let out = match self.peek() {
            Some(Tok::Op('-')) => {
                self.next();
                Expr::Neg(Box::new(self.unary()?))
            }
            Some(Tok::Op('+')) => {
                self.next();
                // `=+A1` is a calculation: an empty A1 gives 0, as in a spreadsheet.
                Expr::Call("+".into(), vec![self.unary()?])
            }
            _ => self.postfix()?,
        };
        self.depth -= 1;
        Ok(out)
    }

    fn postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.primary()?;
        while self.peek() == Some(&Tok::Op('%')) {
            self.next();
            e = Expr::Percent(Box::new(e));
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::Text(s)) => Ok(Expr::Text(s)),
            Some(Tok::LParen) => {
                self.enter()?;
                let e = self.compare()?;
                self.depth -= 1;
                match self.next() {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err(ParseError("a bracket is not closed".into())),
                }
            }
            Some(Tok::Ident(name)) => {
                if self.peek() == Some(&Tok::LParen) {
                    self.next();
                    return self.call(name);
                }
                let upper = name.to_ascii_uppercase();
                if upper == "TRUE" || upper == "FALSE" {
                    return Ok(Expr::Bool(upper == "TRUE"));
                }
                let Some(from) = cell_ref(&name) else {
                    return Err(ParseError(format!("`{name}` is not a cell or a function")));
                };
                if self.peek() == Some(&Tok::Colon) {
                    self.next();
                    let Some(Tok::Ident(to)) = self.next() else {
                        return Err(ParseError("a range needs a cell after the colon".into()));
                    };
                    let to =
                        cell_ref(&to).ok_or_else(|| ParseError(format!("`{to}` is not a cell")))?;
                    return Ok(Expr::Range(from, to));
                }
                Ok(Expr::Ref(from))
            }
            Some(other) => Err(ParseError(format!("unexpected {other:?}"))),
            None => Err(ParseError("the formula ends too soon".into())),
        }
    }

    fn call(&mut self, name: String) -> Result<Expr, ParseError> {
        self.enter()?;
        let mut args = Vec::new();
        if self.peek() == Some(&Tok::RParen) {
            self.next();
        } else {
            loop {
                args.push(self.compare()?);
                match self.next() {
                    Some(Tok::Comma) => {}
                    Some(Tok::RParen) => break,
                    _ => return Err(ParseError(format!("{name}( is not closed"))),
                }
            }
        }
        self.depth -= 1;
        // Newer functions are saved with a prefix (`_xlfn.IFS`); it is the same function.
        let upper = name.to_ascii_uppercase();
        let upper = upper.strip_prefix("_XLFN.").unwrap_or(&upper).to_string();
        Ok(Expr::Call(upper, args))
    }
}

/// A formula without its leading `=`.
pub fn parse_formula(src: &str) -> Result<Expr, ParseError> {
    if src.chars().count() > MAX_FORMULA_CHARS {
        return Err(ParseError("the formula is too long".into()));
    }
    let mut p = Parser {
        toks: lex(src)?,
        at: 0,
        depth: 0,
    };
    let e = p.compare()?;
    if p.at < p.toks.len() {
        return Err(ParseError(
            "something is left over after the formula".into(),
        ));
    }
    Ok(e)
}

/// The error a formula that does not parse stands for.
pub(crate) fn parse_failure() -> CalcError {
    CalcError::Name
}
