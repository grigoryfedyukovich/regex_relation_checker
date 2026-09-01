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
    /// Crate-private: an `Expr` is only ever meant to be built by the
    /// parser (or, internally, by the CEGAR/normalization passes that
    /// rewrite an already-parsed tree). Keeping this internal, together
    /// with `ExprKind` being `#[non_exhaustive]`, means an external caller
    /// can inspect a parsed `Expr` (match on `.kind`, walk `.span`) but
    /// can't hand-construct one bypassing the parser's validation -- e.g.
    /// `ExprKind::Alt(vec![])`, which nothing downstream (`Nfa::from_expr`,
    /// the CEGAR driver's `normalize`, every backend's own AST walk) is
    /// written to expect, since the parser itself never produces one.
    pub(crate) fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
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
