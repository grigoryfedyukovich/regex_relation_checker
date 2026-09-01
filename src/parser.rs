use crate::ast::{Expr, ExprKind, Span};
use crate::charset::CharSet;
use crate::config::{Alphabet, Config};
use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontendErrorKind {
    Syntax,
    Unsupported,
}

#[derive(Clone, Debug, Error, Serialize)]
#[error("{message} at bytes {span_start}..{span_end}")]
pub struct FrontendError {
    pub kind: FrontendErrorKind,
    pub message: String,
    pub span_start: usize,
    pub span_end: usize,
    pub hint: Option<String>,
}

impl FrontendError {
    fn new(kind: FrontendErrorKind, message: impl Into<String>, span: Span) -> Self {
        Self {
            kind,
            message: message.into(),
            span_start: span.start,
            span_end: span.end,
            hint: None,
        }
    }

    fn syntax(message: impl Into<String>, span: Span) -> Self {
        Self::new(FrontendErrorKind::Syntax, message, span)
    }

    fn unsupported(message: impl Into<String>, span: Span) -> Self {
        Self::new(FrontendErrorKind::Unsupported, message, span)
    }

    fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    fn with_default_hint(mut self) -> Self {
        if self.hint.is_none() {
            self.hint = Some("run 'regexrel syntax' for the supported subset".to_owned());
        }
        self
    }

    pub fn span(&self) -> Span {
        Span::new(self.span_start, self.span_end)
    }
}

pub fn parse(pattern: &str, config: &Config) -> Result<Expr, FrontendError> {
    parse_inner(pattern, config).map_err(FrontendError::with_default_hint)
}

fn parse_inner(pattern: &str, config: &Config) -> Result<Expr, FrontendError> {
    let mut parser = Parser::new(pattern, config);
    let expr = parser.parse_alt()?;
    if let Some((offset, ch)) = parser.peek() {
        return Err(FrontendError::syntax(
            format!("unexpected character {ch:?}"),
            Span::new(offset, offset + ch.len_utf8()),
        ));
    }
    validate_anchors(&expr)?;
    Ok(expr)
}

struct Parser<'a> {
    input: &'a str,
    chars: Vec<(usize, char)>,
    pos: usize,
    config: &'a Config,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, config: &'a Config) -> Self {
        Self {
            input,
            chars: input.char_indices().collect(),
            pos: 0,
            config,
        }
    }

    fn peek(&self) -> Option<(usize, char)> {
        self.chars.get(self.pos).copied()
    }

    fn peek_n(&self, n: usize) -> Option<(usize, char)> {
        self.chars.get(self.pos + n).copied()
    }

    fn bump(&mut self) -> Option<(usize, char)> {
        let result = self.peek();
        if result.is_some() {
            self.pos += 1;
        }
        result
    }

    fn current_offset(&self) -> usize {
        self.peek()
            .map(|(offset, _)| offset)
            .unwrap_or(self.input.len())
    }

    fn parse_alt(&mut self) -> Result<Expr, FrontendError> {
        let start = self.current_offset();
        let mut branches = vec![self.parse_concat()?];
        while matches!(self.peek(), Some((_, '|'))) {
            self.bump();
            branches.push(self.parse_concat()?);
        }
        let end = branches.last().map(|expr| expr.span.end).unwrap_or(start);
        if branches.len() == 1 {
            Ok(branches.pop().expect("one branch"))
        } else {
            Ok(Expr::new(ExprKind::Alt(branches), Span::new(start, end)))
        }
    }

    fn parse_concat(&mut self) -> Result<Expr, FrontendError> {
        let start = self.current_offset();
        let mut parts = Vec::new();
        while let Some((_, ch)) = self.peek() {
            if matches!(ch, ')' | '|') {
                break;
            }
            parts.push(self.parse_repeat()?);
        }
        let end = parts.last().map(|expr| expr.span.end).unwrap_or(start);
        match parts.len() {
            0 => Ok(Expr::new(ExprKind::Empty, Span::new(start, start))),
            1 => Ok(parts.pop().expect("one part")),
            _ => Ok(Expr::new(ExprKind::Concat(parts), Span::new(start, end))),
        }
    }

    fn parse_repeat(&mut self) -> Result<Expr, FrontendError> {
        let atom = self.parse_atom()?;
        let Some((quantifier_start, ch)) = self.peek() else {
            return Ok(atom);
        };

        let (min, max, quantifier_end) = match ch {
            '*' => {
                self.bump();
                (0, None, quantifier_start + 1)
            }
            '+' => {
                self.bump();
                (1, None, quantifier_start + 1)
            }
            '?' => {
                self.bump();
                (0, Some(1), quantifier_start + 1)
            }
            '{' => self.parse_braced_quantifier()?,
            _ => return Ok(atom),
        };

        if let Some((offset, suffix @ ('?' | '+'))) = self.peek() {
            return Err(FrontendError::unsupported(
                format!("lazy or possessive quantifier suffix {suffix:?} is not supported"),
                Span::new(offset, offset + 1),
            )
            .with_hint("remove the suffix; language relations ignore backtracking preference"));
        }

        if matches!(self.peek(), Some((_, '*' | '+' | '?' | '{'))) {
            let (offset, repeated) = self.peek().expect("checked above");
            return Err(FrontendError::syntax(
                format!("multiple quantifiers are not allowed; found {repeated:?}"),
                Span::new(offset, offset + repeated.len_utf8()),
            ));
        }

        let span = atom.span.join(Span::new(quantifier_start, quantifier_end));
        Ok(Expr::new(
            ExprKind::Repeat {
                expr: Box::new(atom),
                min,
                max,
            },
            span,
        ))
    }

    fn parse_braced_quantifier(&mut self) -> Result<(usize, Option<usize>, usize), FrontendError> {
        let (start, _) = self.bump().expect("opening brace");
        let min = self.parse_number()?.ok_or_else(|| {
            FrontendError::syntax(
                "expected a decimal lower bound after '{'",
                Span::new(start, self.current_offset()),
            )
            .with_hint("use forms such as {2}, {2,}, or {2,5}")
        })?;

        let max = match self.peek() {
            Some((_, '}')) => Some(min),
            Some((_, ',')) => {
                self.bump();
                self.parse_number()?
            }
            Some((offset, ch)) => {
                return Err(FrontendError::syntax(
                    format!("expected ',' or '}}', found {ch:?}"),
                    Span::new(offset, offset + ch.len_utf8()),
                ));
            }
            None => {
                return Err(FrontendError::syntax(
                    "unterminated repetition",
                    Span::new(start, self.input.len()),
                ));
            }
        };

        let (end_start, end_ch) = self.bump().ok_or_else(|| {
            FrontendError::syntax(
                "unterminated repetition",
                Span::new(start, self.input.len()),
            )
        })?;
        if end_ch != '}' {
            return Err(FrontendError::syntax(
                format!("expected '}}', found {end_ch:?}"),
                Span::new(end_start, end_start + end_ch.len_utf8()),
            ));
        }

        if let Some(maximum) = max {
            if maximum < min {
                return Err(FrontendError::syntax(
                    "repetition upper bound is smaller than lower bound",
                    Span::new(start, end_start + 1),
                ));
            }
            if maximum > self.config.max_repeat {
                return Err(FrontendError::unsupported(
                    format!(
                        "repetition upper bound {maximum} exceeds configured max_repeat {}",
                        self.config.max_repeat
                    ),
                    Span::new(start, end_start + 1),
                ));
            }
        } else if min > self.config.max_repeat {
            return Err(FrontendError::unsupported(
                format!(
                    "repetition lower bound {min} exceeds configured max_repeat {}",
                    self.config.max_repeat
                ),
                Span::new(start, end_start + 1),
            ));
        }

        Ok((min, max, end_start + 1))
    }

    fn parse_number(&mut self) -> Result<Option<usize>, FrontendError> {
        let start = self.current_offset();
        let mut value = 0usize;
        let mut found = false;
        while let Some((offset, ch)) = self.peek() {
            let Some(digit) = ch.to_digit(10) else {
                break;
            };
            found = true;
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit as usize))
                .ok_or_else(|| {
                    FrontendError::syntax(
                        "repetition bound overflows the representable range",
                        Span::new(start, offset + ch.len_utf8()),
                    )
                    .with_hint("use a smaller bound no greater than the platform usize maximum")
                })?;
            self.bump();
        }
        Ok(found.then_some(value))
    }

    fn parse_atom(&mut self) -> Result<Expr, FrontendError> {
        let (start, ch) = self.bump().ok_or_else(|| {
            FrontendError::syntax(
                "expected a regex atom",
                Span::new(self.input.len(), self.input.len()),
            )
        })?;
        let end = start + ch.len_utf8();
        match ch {
            '(' => {
                if matches!(self.peek(), Some((_, '?'))) {
                    let next = self.peek().expect("checked above").0;
                    return Err(FrontendError::unsupported(
                        concat!(
                            "lookaround, inline flags, and non-capturing group prefixes ",
                            "are not supported"
                        ),
                        Span::new(start, next + 1),
                    )
                    .with_hint(
                        "use a plain capturing group; capture identity has no semantic effect",
                    ));
                }
                let inner = self.parse_alt()?;
                let Some((close_start, close)) = self.bump() else {
                    return Err(FrontendError::syntax(
                        "unterminated group",
                        Span::new(start, self.input.len()),
                    ));
                };
                if close != ')' {
                    return Err(FrontendError::syntax(
                        format!("expected ')', found {close:?}"),
                        Span::new(close_start, close_start + close.len_utf8()),
                    ));
                }
                Ok(Expr::new(inner.kind, Span::new(start, close_start + 1)))
            }
            '[' => self.parse_class(start),
            '.' => Ok(Expr::new(
                ExprKind::CharSet(CharSet::any(
                    self.config.alphabet,
                    self.config.dot_matches_newline,
                )),
                Span::new(start, end),
            )),
            '\\' => self.parse_escape(false),
            '^' => Ok(Expr::new(ExprKind::AnchorStart, Span::new(start, end))),
            '$' => Ok(Expr::new(ExprKind::AnchorEnd, Span::new(start, end))),
            '*' | '+' | '?' | '{' => Err(FrontendError::syntax(
                format!("quantifier {ch:?} has no preceding atom"),
                Span::new(start, end),
            )),
            ')' => Err(FrontendError::syntax(
                "unmatched ')'",
                Span::new(start, end),
            )),
            _ => self.literal_expr(ch, Span::new(start, end)),
        }
    }

    fn parse_escape(&mut self, in_class: bool) -> Result<Expr, FrontendError> {
        let slash_start = self
            .chars
            .get(self.pos.saturating_sub(1))
            .map(|(offset, _)| *offset)
            .unwrap_or(0);
        let Some((start, escaped)) = self.bump() else {
            return Err(FrontendError::syntax(
                "trailing backslash",
                Span::new(slash_start, self.input.len()),
            ));
        };
        let span = Span::new(slash_start, start + escaped.len_utf8());
        let charset = match escaped {
            'd' => Some(CharSet::ascii_digits()),
            'D' => Some(CharSet::ascii_digits().complement(self.config.alphabet)),
            'w' => Some(CharSet::ascii_word()),
            'W' => Some(CharSet::ascii_word().complement(self.config.alphabet)),
            's' => Some(CharSet::ascii_space()),
            'S' => Some(CharSet::ascii_space().complement(self.config.alphabet)),
            _ => None,
        };
        if let Some(set) = charset {
            return Ok(Expr::new(ExprKind::CharSet(set), span));
        }

        let literal = match escaped {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '0' => '\0',
            '1'..='9' => {
                return Err(FrontendError::unsupported(
                    "backreferences are not regular and are not supported",
                    span,
                ));
            }
            // Outside a class, `\b` is the word-boundary assertion (a
            // zero-width lookaround, not a regular-language construct, so
            // unsupported here like the other assertions below). Inside a
            // class it can only ever mean a literal character -- a boundary
            // assertion has no meaning as one member of a character set --
            // and every mainstream engine (JS, Python, PCRE, ICU, .NET,
            // Rust's `regex` crate) takes `[\b]` to mean the single
            // backspace character, U+0008, on exactly that basis.
            'b' if in_class => '\u{0008}',
            'b' if !in_class => {
                return Err(FrontendError::unsupported(
                    "word-boundary assertions are not supported",
                    span,
                ));
            }
            'A' | 'z' | 'Z' | 'G' if !in_class => {
                return Err(FrontendError::unsupported(
                    "this zero-width assertion is not supported",
                    span,
                ));
            }
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$'
            | '-' | '/' => escaped,
            _ => {
                return Err(FrontendError::unsupported(
                    format!("escape \\{escaped} is not supported"),
                    span,
                )
                .with_hint(concat!(
                    "use a literal character, an escaped metacharacter, ",
                    "or a documented shorthand class"
                )));
            }
        };
        self.literal_expr(literal, span)
    }

    fn literal_expr(&self, ch: char, span: Span) -> Result<Expr, FrontendError> {
        if self.config.alphabet == Alphabet::Ascii && (ch as u32) > 0x7f {
            return Err(FrontendError::unsupported(
                format!("non-ASCII literal {ch:?} is outside the configured ASCII alphabet"),
                span,
            )
            .with_hint("use --alphabet unicode to analyze this literal"));
        }
        Ok(Expr::new(ExprKind::Literal(ch), span))
    }

    fn parse_class(&mut self, class_start: usize) -> Result<Expr, FrontendError> {
        let negated = if matches!(self.peek(), Some((_, '^'))) {
            self.bump();
            true
        } else {
            false
        };
        let mut set = CharSet::empty();
        let mut saw_item = false;

        loop {
            let Some((offset, ch)) = self.peek() else {
                return Err(FrontendError::syntax(
                    "unterminated character class",
                    Span::new(class_start, self.input.len()),
                ));
            };
            if ch == ']' && saw_item {
                self.bump();
                let set = if negated {
                    set.complement(self.config.alphabet)
                } else {
                    set.clipped(self.config.alphabet)
                };
                return Ok(Expr::new(
                    ExprKind::CharSet(set),
                    Span::new(class_start, offset + 1),
                ));
            }

            let first = self.parse_class_atom(class_start, saw_item)?;
            saw_item = true;
            if matches!(self.peek(), Some((_, '-')))
                && !matches!(self.peek_n(1), Some((_, ']')) | None)
            {
                let hyphen_offset = self.bump().expect("hyphen").0;
                let second = self.parse_class_atom(class_start, true)?;
                let (left, right) = match (first.literal, second.literal) {
                    (Some(left), Some(right)) => (left, right),
                    _ => {
                        return Err(FrontendError::unsupported(
                            "character-class ranges require literal endpoints",
                            Span::new(hyphen_offset, second.span.end),
                        ));
                    }
                };
                if left > right {
                    return Err(FrontendError::syntax(
                        format!("reversed character range {left:?}-{right:?}"),
                        first.span.join(second.span),
                    ));
                }
                set = set.union(&CharSet::from_interval(left, right));
            } else {
                set = set.union(&first.set);
            }
        }
    }

    fn parse_class_atom(
        &mut self,
        class_start: usize,
        saw_item: bool,
    ) -> Result<ClassAtom, FrontendError> {
        let (start, ch) = self.bump().ok_or_else(|| {
            FrontendError::syntax(
                "unterminated character class",
                Span::new(class_start, self.input.len()),
            )
        })?;
        let span = Span::new(start, start + ch.len_utf8());
        if ch == ']' && !saw_item {
            return Ok(ClassAtom::literal(']', span));
        }
        if ch == '\\' {
            let escaped = self.parse_escape(true)?;
            return match escaped.kind {
                ExprKind::Literal(literal) => Ok(ClassAtom::literal(literal, escaped.span)),
                ExprKind::CharSet(set) => Ok(ClassAtom {
                    set,
                    literal: None,
                    span: escaped.span,
                }),
                _ => unreachable!("escape only returns literal or charset"),
            };
        }
        self.literal_expr(ch, span)?;
        Ok(ClassAtom::literal(ch, span))
    }
}

struct ClassAtom {
    set: CharSet,
    literal: Option<char>,
    span: Span,
}

impl ClassAtom {
    fn literal(ch: char, span: Span) -> Self {
        Self {
            set: CharSet::singleton(ch),
            literal: Some(ch),
            span,
        }
    }
}

fn validate_anchors(expr: &Expr) -> Result<(), FrontendError> {
    match &expr.kind {
        ExprKind::Concat(parts) => {
            for (index, part) in parts.iter().enumerate() {
                match &part.kind {
                    ExprKind::AnchorStart if index == 0 => {}
                    ExprKind::AnchorEnd if index + 1 == parts.len() => {}
                    ExprKind::AnchorStart => {
                        return Err(FrontendError::unsupported(
                            "'^' is only supported at the beginning of the whole regex",
                            part.span,
                        ));
                    }
                    ExprKind::AnchorEnd => {
                        return Err(FrontendError::unsupported(
                            "'$' is only supported at the end of the whole regex",
                            part.span,
                        ));
                    }
                    _ => reject_nested_anchor(part)?,
                }
            }
            Ok(())
        }
        ExprKind::AnchorStart | ExprKind::AnchorEnd => Ok(()),
        _ => reject_nested_anchor(expr),
    }
}

fn reject_nested_anchor(expr: &Expr) -> Result<(), FrontendError> {
    match &expr.kind {
        ExprKind::AnchorStart | ExprKind::AnchorEnd => Err(FrontendError::unsupported(
            "anchors are only supported at the outer full-expression boundary",
            expr.span,
        )),
        ExprKind::Concat(parts) | ExprKind::Alt(parts) => {
            for part in parts {
                reject_nested_anchor(part)?;
            }
            Ok(())
        }
        ExprKind::Repeat { expr, .. } => reject_nested_anchor(expr),
        _ => Ok(()),
    }
}

pub const SYNTAX_HELP: &str = r#"Supported regex subset (full-string semantics)

Atoms:
  literal characters        abc
  grouping                  (ab|cd)
  alternation               a|b
  wildcard                  .        (newline excluded unless configured)
  character classes         [abc] [a-z] [^0-9]
  escapes                    \\ \n \r \t \0 \d \D \w \W \s \S
  boundary anchors           ^...$   (optional under full-string semantics)

Quantifiers:
  *, +, ?, {m}, {m,}, {m,n}

A `{` that doesn't form one of those three forms (reversed bounds, a
non-numeric body, unterminated) is a syntax error, not a literal `{` --
unlike JS, Python, and Rust's `regex` crate. Escape it (\{) for a literal.

Not supported:
  backreferences, lookaround, inline flags, lazy/possessive quantifiers,
  word-boundary assertions (\b outside a class), Unicode property escapes,
  or PCRE conditionals.

Inside a class, \b has no meaning as a boundary assertion and instead
matches the literal backspace character U+0008 -- e.g. [\b] -- the same
reading every mainstream engine (JS, Python, PCRE, ICU, .NET, Rust's
`regex` crate) gives it, for the same reason.

ASCII mode gives \d/\w/\s their ASCII meanings. Unicode mode expands the
alphabet for literals, dot, and negated classes, but those shorthand classes
remain ASCII-defined in v0.1.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_running_example() {
        let expr = parse("[a-z]{2,}", &Config::default()).unwrap();
        assert!(matches!(
            expr.kind,
            ExprKind::Repeat {
                min: 2,
                max: None,
                ..
            }
        ));
    }

    #[test]
    fn parses_empty_regex_as_epsilon() {
        let expr = parse("", &Config::default()).unwrap();
        assert!(matches!(expr.kind, ExprKind::Empty));
        assert_eq!(expr.span, Span::new(0, 0));
    }

    #[test]
    fn parses_empty_alternation_branches() {
        for pattern in ["a|", "|a", "(|a)"] {
            parse(pattern, &Config::default()).unwrap();
        }
    }

    #[test]
    fn parses_literal_and_shorthand_class_members() {
        for pattern in [r"\.", r"\+", r"\n", r"\d", r"\W", r"[\d_-]"] {
            parse(pattern, &Config::default()).unwrap();
        }
    }

    #[test]
    fn accepts_literal_closing_bracket_as_first_class_item() {
        parse("[]a]", &Config::default()).unwrap();
    }

    #[test]
    fn accepts_literal_hyphen_at_class_edges() {
        parse("[-a]", &Config::default()).unwrap();
        parse("[a-]", &Config::default()).unwrap();
    }

    #[test]
    fn rejects_reversed_character_range() {
        let error = parse("[z-a]", &Config::default()).unwrap_err();
        assert_eq!(error.kind, FrontendErrorKind::Syntax);
        assert!(error.message.contains("reversed character range"));
    }

    #[test]
    fn rejects_shorthand_as_range_endpoint() {
        let error = parse(r"[a-\d]", &Config::default()).unwrap_err();
        assert_eq!(error.kind, FrontendErrorKind::Unsupported);
        assert!(error.message.contains("literal endpoints"));
    }

    #[test]
    fn rejects_unterminated_character_class() {
        let error = parse("[abc", &Config::default()).unwrap_err();
        assert_eq!(error.kind, FrontendErrorKind::Syntax);
        assert_eq!(error.span_start, 0);
    }

    #[test]
    fn parses_all_counted_repetition_forms() {
        let exact = parse("a{3}", &Config::default()).unwrap();
        assert!(matches!(
            exact.kind,
            ExprKind::Repeat {
                min: 3,
                max: Some(3),
                ..
            }
        ));

        let open = parse("a{3,}", &Config::default()).unwrap();
        assert!(matches!(
            open.kind,
            ExprKind::Repeat {
                min: 3,
                max: None,
                ..
            }
        ));

        let bounded = parse("a{3,5}", &Config::default()).unwrap();
        assert!(matches!(
            bounded.kind,
            ExprKind::Repeat {
                min: 3,
                max: Some(5),
                ..
            }
        ));
    }

    #[test]
    fn rejects_reversed_repetition_bounds() {
        let error = parse("a{4,2}", &Config::default()).unwrap_err();
        assert_eq!(error.kind, FrontendErrorKind::Syntax);
        assert!(error.message.contains("upper bound is smaller"));
    }

    #[test]
    fn repetition_bound_overflow_has_a_specific_diagnostic() {
        let pattern = format!("a{{{}}}", "9".repeat(100));
        let error = parse(&pattern, &Config::default()).unwrap_err();
        assert_eq!(error.kind, FrontendErrorKind::Syntax);
        assert!(error.message.contains("overflows the representable range"));
        assert!(error.hint.unwrap().contains("smaller bound"));
    }

    #[test]
    fn repetition_limit_is_reported_as_unsupported() {
        let config = Config {
            max_repeat: 3,
            ..Config::default()
        };
        let error = parse("a{4}", &config).unwrap_err();
        assert_eq!(error.kind, FrontendErrorKind::Unsupported);
        assert!(error.message.contains("max_repeat 3"));
    }

    #[test]
    fn rejects_quantifier_without_atom() {
        for pattern in ["*a", "+", "?", "{2}"] {
            let error = parse(pattern, &Config::default()).unwrap_err();
            assert_eq!(error.kind, FrontendErrorKind::Syntax, "{pattern}");
        }
    }

    #[test]
    fn rejects_multiple_quantifiers() {
        let error = parse("a**", &Config::default()).unwrap_err();
        assert_eq!(error.kind, FrontendErrorKind::Syntax);
        assert!(error.message.contains("multiple quantifiers"));
    }

    #[test]
    fn rejects_lazy_and_possessive_quantifiers() {
        for pattern in ["a*?", "a++", "a{2,3}?"] {
            let error = parse(pattern, &Config::default()).unwrap_err();
            assert_eq!(error.kind, FrontendErrorKind::Unsupported, "{pattern}");
        }
    }

    #[test]
    fn rejects_backreferences_as_unsupported() {
        let error = parse(r"(a)\1", &Config::default()).unwrap_err();
        assert_eq!(error.kind, FrontendErrorKind::Unsupported);
    }

    #[test]
    fn rejects_lookaround_and_prefixed_groups() {
        for pattern in ["(?=a)", "(?:a)", "(?i:a)"] {
            let error = parse(pattern, &Config::default()).unwrap_err();
            assert_eq!(error.kind, FrontendErrorKind::Unsupported, "{pattern}");
        }
    }

    #[test]
    fn rejects_word_boundary_and_unknown_escape() {
        let boundary = parse(r"\bword\b", &Config::default()).unwrap_err();
        assert_eq!(boundary.kind, FrontendErrorKind::Unsupported);

        let unknown = parse(r"\q", &Config::default()).unwrap_err();
        assert_eq!(unknown.kind, FrontendErrorKind::Unsupported);
        assert!(unknown.hint.is_some());
    }

    /// `\b` outside a class is the (unsupported) word-boundary assertion,
    /// but every mainstream engine (JS, Python, PCRE, ICU, .NET, Rust's
    /// `regex` crate) takes `[\b]` to mean the literal backspace character,
    /// U+0008, precisely because a zero-width assertion has no meaning as
    /// one member of a character set. This parser used to reject `\b`
    /// unconditionally, even inside a class, falling through to the generic
    /// "unsupported escape" error instead of treating it as a literal.
    #[test]
    fn class_backspace_escape_is_a_literal_not_unsupported() {
        let expr = parse(r"[\b]", &Config::default()).unwrap();
        let ExprKind::CharSet(set) = &expr.kind else {
            panic!("expected a CharSet, got {expr:?}");
        };
        assert!(set.contains('\u{0008}'));
        assert_eq!(
            set.intervals().len(),
            1,
            "expected exactly one member: U+0008"
        );

        // Outside a class, the same escape is still the unsupported
        // word-boundary assertion -- this fix only changes the in-class
        // reading, not the bare one.
        let boundary = parse(r"\b", &Config::default()).unwrap_err();
        assert_eq!(boundary.kind, FrontendErrorKind::Unsupported);
    }

    #[test]
    fn allows_outer_anchors() {
        parse("^ab$", &Config::default()).unwrap();
        parse("^", &Config::default()).unwrap();
        parse("$", &Config::default()).unwrap();
    }

    #[test]
    fn rejects_nested_or_repeated_anchors() {
        for pattern in ["a(^b)", "a$b", "^^a", "a$$", "(^a|b)"] {
            let error = parse(pattern, &Config::default()).unwrap_err();
            assert_eq!(error.kind, FrontendErrorKind::Unsupported, "{pattern}");
        }
    }

    #[test]
    fn reports_unmatched_parentheses() {
        let open = parse("(ab", &Config::default()).unwrap_err();
        assert_eq!(open.kind, FrontendErrorKind::Syntax);
        assert!(open.message.contains("unterminated group"));

        let close = parse("ab)", &Config::default()).unwrap_err();
        assert_eq!(close.kind, FrontendErrorKind::Syntax);
        assert!(close.message.contains("unexpected character"));
    }

    #[test]
    fn ascii_mode_rejects_non_ascii_literal() {
        let error = parse("é", &Config::default()).unwrap_err();
        assert_eq!(error.kind, FrontendErrorKind::Unsupported);
        assert_eq!(error.span_start, 0);
        assert_eq!(error.span_end, "é".len());
    }

    #[test]
    fn unicode_mode_accepts_non_ascii_literal_and_class() {
        let config = Config {
            alphabet: Alphabet::Unicode,
            ..Config::default()
        };
        parse("é", &config).unwrap();
        parse("[α-ω]+", &config).unwrap();
    }

    #[test]
    fn rejects_unicode_property_escape() {
        let error = parse(r"\p", &Config::default()).unwrap_err();
        assert_eq!(error.kind, FrontendErrorKind::Unsupported);
    }
}
