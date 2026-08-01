use crate::charset::CharSet;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn join(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Empty,
    Literal(char),
    CharSet(CharSet),
    Concat(Vec<Expr>),
    Alt(Vec<Expr>),
    Repeat {
        expr: Box<Expr>,
        min: usize,
        max: Option<usize>,
    },
    AnchorStart,
    AnchorEnd,
}
