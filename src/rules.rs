use std::collections::HashSet;
use std::path::Path;

use tree_sitter::{Node, Point, Tree};

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Rule {
    Assign,
    Block,
    BlockMax,
    ExprAfter,
    ExprUnrelated,
    ExprMax,
    Jump,
    Tail,
    Decl,
    UseInFn,
    Comment,
}

impl Rule {
    pub fn label(self) -> &'static str {
        match self {
            Rule::Assign => "R1 assign",
            Rule::Block => "R2 block",
            Rule::BlockMax => "R3 block-max",
            Rule::ExprAfter => "R4a expr-after",
            Rule::ExprUnrelated => "R4b expr-unrelated",
            Rule::ExprMax => "R4c expr-max",
            Rule::Jump => "R5 jump",
            Rule::Tail => "R6 tail",
            Rule::Decl => "R7 decl",
            Rule::UseInFn => "R8 use-in-fn",
            Rule::Comment => "R9 comment",
        }
    }
}

/// Пустая строка вставляется перед строкой `row` (с нуля), байтовое смещение начала которой — `line_start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Violation {
    pub rule: Rule,
    pub row: usize,
    pub line_start: usize,
}

impl Violation {
    pub fn render(&self, path: &Path) -> String {
        let hint = match self.rule {
            Rule::UseInFn => ": move this import to the top of the module",
            _ => "",
        };

        format!(
            "{}:{}: {}{hint}",
            path.display(),
            self.row + 1,
            self.rule.label()
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Let,
    Assign,
    Jump,
    Label,
    Block,
    // Тип узла нужен R7: два `use` подряд — один вид, `use` и `const` — разные.
    Item(&'static str),
    Tail,
    Expr,
}

const ITEMS: &[&str] = &[
    "function_item",
    "struct_item",
    "enum_item",
    "union_item",
    "use_declaration",
    "const_item",
    "static_item",
    "impl_item",
    "trait_item",
    "mod_item",
    "type_item",
    "macro_definition",
    "extern_crate_declaration",
    "foreign_mod_item",
];

const BLOCK_LIKE: &[&str] = &[
    "if_expression",
    "match_expression",
    "for_expression",
    "while_expression",
    "loop_expression",
    "block",
    "unsafe_block",
    "async_block",
    "const_block",
    "try_block",
    "gen_block",
];

/// Начало узла; `column` в tree-sitter байтовый, поэтому начало строки — `byte - column`.
#[derive(Clone, Copy)]
struct Top {
    point: Point,
    byte: usize,
}

impl Top {
    fn of(n: Node) -> Self {
        Self {
            point: n.start_position(),
            byte: n.start_byte(),
        }
    }

    fn line_start(self) -> usize {
        self.byte - self.point.column
    }
}

/// Непрерывная серия комментариев и атрибутов над инструкцией.
struct Header {
    top: Top,
    last_row: usize,
    has_comment: bool,
}

impl Header {
    fn new(n: Node) -> Self {
        Self {
            top: Top::of(n),
            last_row: end_row(n),
            has_comment: is_comment(n),
        }
    }

    fn attaches(&self, row: usize) -> bool {
        row <= self.last_row + 1
    }

    fn extend(&mut self, n: Node) {
        self.last_row = self.last_row.max(end_row(n));
        self.has_comment |= is_comment(n);
    }
}

struct Stmt<'t> {
    node: Node<'t>,
    kind: Kind,
    top: Top,
    end: usize,
    skip: bool,
    has_comment: bool,
}

fn end_row(n: Node) -> usize {
    let p = n.end_position();
    if p.column == 0 && p.row > n.start_position().row {
        p.row - 1
    } else {
        p.row
    }
}

fn text<'a>(n: Node, src: &'a [u8]) -> &'a [u8] {
    &src[n.start_byte()..n.end_byte()]
}

fn is_comment(n: Node) -> bool {
    matches!(n.kind(), "line_comment" | "block_comment")
}

fn is_header(n: Node) -> bool {
    is_comment(n) || n.kind() == "attribute_item"
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn has_skip_attr(n: Node, src: &[u8]) -> bool {
    matches!(n.kind(), "attribute_item" | "inner_attribute_item")
        && contains(text(n, src), b"rustfmt::skip")
}

/// Серия шапки (комментарии и атрибуты, пустые строки не рвут её) с `rustfmt::skip` после узла `n`.
fn series_after(series: bool, n: Node, src: &[u8]) -> bool {
    has_skip_attr(n, src) || (series && is_header(n))
}

fn has_inner_skip(n: Node, src: &[u8]) -> bool {
    let mut c = n.walk();

    n.children(&mut c)
        .any(|ch| ch.kind() == "inner_attribute_item" && has_skip_attr(ch, src))
}

/// Обход в глубину; `visit(узел, skip)` решает, спускаться ли в детей, `skip` — серия шапки с
/// `rustfmt::skip` перед узлом.
pub(crate) fn walk<'t>(root: Node<'t>, src: &[u8], mut visit: impl FnMut(Node<'t>, bool) -> bool) {
    let mut cursor = root.walk();

    // Состояние серии на текущем уровне и после родителей, в которые спустились.
    let mut series = false;
    let mut parents: Vec<bool> = Vec::new();

    loop {
        let node = cursor.node();
        let after = series_after(series, node, src);

        if visit(node, series) && cursor.goto_first_child() {
            parents.push(after);

            series = false;

            continue;
        }

        series = after;

        loop {
            if cursor.goto_next_sibling() {
                break;
            }

            if !cursor.goto_parent() {
                return;
            }

            series = parents.pop().unwrap_or(false);
        }
    }
}

fn expr_of(n: Node) -> Option<Node> {
    if n.kind() == "expression_statement" {
        n.named_child(0)
    } else {
        Some(n)
    }
}

fn classify(n: Node, is_last: bool) -> Kind {
    let t = n.kind();
    if t == "let_declaration" {
        return Kind::Let;
    }

    if let Some(item) = ITEMS.iter().find(|k| **k == t) {
        return Kind::Item(item);
    }

    let Some(e) = expr_of(n) else {
        return Kind::Expr;
    };

    let et = e.kind();
    if matches!(et, "assignment_expression" | "compound_assignment_expr") {
        return Kind::Assign;
    }

    if matches!(
        et,
        "return_expression" | "break_expression" | "continue_expression"
    ) {
        return Kind::Jump;
    }

    if BLOCK_LIKE.contains(&et) {
        let mut c = e.walk();
        let labeled = e.children(&mut c).any(|ch| ch.kind() == "label");

        return if labeled { Kind::Label } else { Kind::Block };
    }

    if is_last && t != "expression_statement" {
        return Kind::Tail;
    }

    Kind::Expr
}

fn collect<'t>(block: Node<'t>, src: &[u8]) -> Vec<Stmt<'t>> {
    let mut stmts: Vec<Stmt<'t>> = Vec::new();
    let mut pending: Option<Header> = None;

    let mut series = false;

    let mut cursor = block.walk();
    for c in block.children(&mut cursor) {
        let skip = series;
        series = series_after(series, c, src);

        if !c.is_named() {
            continue;
        }

        let s = c.start_position().row;
        match c.kind() {
            _ if is_header(c) => {
                if pending.is_none()
                    && is_comment(c)
                    && let Some(last) = stmts.last_mut()
                    && s == last.end
                {
                    last.end = last.end.max(end_row(c));

                    continue;
                }

                match &mut pending {
                    Some(h) if h.attaches(s) => h.extend(c),
                    _ => pending = Some(Header::new(c)),
                }
            }
            "empty_statement" => {
                if let Some(last) = stmts.last_mut() {
                    last.end = last.end.max(end_row(c));
                }
            }
            "inner_attribute_item" => {}
            _ => {
                let header = pending.take().filter(|h| h.attaches(s));

                stmts.push(Stmt {
                    node: c,
                    kind: classify(c, false),
                    top: header.as_ref().map_or_else(|| Top::of(c), |h| h.top),
                    end: end_row(c),
                    skip,
                    has_comment: header.is_some_and(|h| h.has_comment),
                });
            }
        }
    }

    if let Some(last) = stmts.last_mut() {
        last.kind = classify(last.node, true);
    }

    stmts
}

fn word_len(s: &[u8]) -> usize {
    s.iter()
        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
        .count()
}

/// Имена из `{name}` и `{:w$}` / `{:.p$}`: они используют переменные так же, как аргументы макроса.
fn add_format_args(s: &[u8], out: &mut HashSet<String>) {
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'{' {
            i += 1;

            continue;
        }

        if s.get(i + 1) == Some(&b'{') {
            i += 2;

            continue;
        }

        let start = i + 1;
        let j = start + word_len(&s[start..]);

        if j > start && !s[start].is_ascii_digit() && matches!(s.get(j), Some(b'}') | Some(b':')) {
            out.insert(String::from_utf8_lossy(&s[start..j]).into_owned());
        }

        i = j.max(i + 1);
        if s.get(j) == Some(&b':') {
            i = add_count_args(s, j + 1, out);
        }
    }
}

/// Идентификаторы вида `name$` в спецификации формата до `}`; возвращает позицию её конца.
fn add_count_args(s: &[u8], from: usize, out: &mut HashSet<String>) -> usize {
    let mut k = from;
    while k < s.len() && !matches!(s[k], b'}' | b'{') {
        let len = word_len(&s[k..]);
        if len == 0 {
            k += 1;

            continue;
        }

        if s.get(k + len) == Some(&b'$') && !s[k].is_ascii_digit() {
            out.insert(String::from_utf8_lossy(&s[k..k + len]).into_owned());
        }

        k += len;
    }

    k
}

// `pattern`: имена типов/вариантов (`Some` в `Some(x)`) переменными не считаем.
fn idents(n: Node, src: &[u8], in_macro: bool, pattern: bool, out: &mut HashSet<String>) {
    match n.kind() {
        "identifier" | "self" | "shorthand_field_identifier" => {
            out.insert(String::from_utf8_lossy(text(n, src)).into_owned());
        }
        "string_content" if in_macro => add_format_args(text(n, src), out),
        _ => {}
    }

    let in_macro = in_macro || n.kind() == "token_tree";
    let skip_type = pattern && matches!(n.kind(), "tuple_struct_pattern" | "struct_pattern");

    for i in 0..n.child_count() {
        let Some(c) = n.child(i) else { continue };

        if skip_type && n.field_name_for_child(i) == Some("type") {
            continue;
        }

        idents(c, src, in_macro, pattern, out);
    }
}

fn used(n: Node, src: &[u8]) -> HashSet<String> {
    let mut out = HashSet::new();
    idents(n, src, false, false, &mut out);

    out
}

fn bound(n: Node, src: &[u8]) -> HashSet<String> {
    let mut out = HashSet::new();
    match expr_of(n) {
        Some(e) if n.kind() == "let_declaration" => {
            if let Some(p) = e.child_by_field_name("pattern") {
                idents(p, src, false, true, &mut out);
            }
        }
        Some(e) => {
            if let Some(l) = e.child_by_field_name("left") {
                idents(l, src, false, false, &mut out);
            }
        }
        None => {}
    }

    out
}

fn first_in_body(body: Node) -> Option<Node> {
    let mut c = body.walk();

    body.named_children(&mut c).find(|ch| {
        !is_header(*ch) && !matches!(ch.kind(), "empty_statement" | "inner_attribute_item")
    })
}

fn body_of(e: Node) -> Option<Node> {
    e.child_by_field_name("consequence")
        .or_else(|| e.child_by_field_name("body"))
        .or_else(|| {
            if e.kind() == "block" {
                return Some(e);
            }

            let mut c = e.walk();

            e.named_children(&mut c).find(|ch| ch.kind() == "block")
        })
}

fn used_block(n: Node, src: &[u8]) -> HashSet<String> {
    let mut out = HashSet::new();

    let Some(e) = expr_of(n) else { return out };
    for f in ["condition", "pattern", "value"] {
        if let Some(c) = e.child_by_field_name(f) {
            idents(c, src, false, false, &mut out);
        }
    }

    if let Some(first) = body_of(e).and_then(first_in_body) {
        idents(first, src, false, false, &mut out);
    }

    out
}

fn intersects(a: &HashSet<String>, b: &HashSet<String>) -> bool {
    a.iter().any(|x| b.contains(x))
}

fn check_block(block: Node, src: &[u8], out: &mut Vec<Violation>) {
    let stmts = collect(block, src);
    let mut push = |rule: Rule, s: &Stmt| {
        let line_start = s.top.line_start();

        let ws = src[line_start..s.top.byte]
            .iter()
            .all(|b| b.is_ascii_whitespace());
        if ws {
            out.push(Violation {
                rule,
                row: s.top.point.row,
                line_start,
            });
        }
    };

    for i in 1..stmts.len() {
        let (prev, cur) = (&stmts[i - 1], &stmts[i]);
        if cur.skip || cur.top.point.row != prev.end + 1 {
            continue;
        }

        let pk = prev.kind;

        let linked = matches!(pk, Kind::Let | Kind::Assign);

        // R3/R4c: над действием прилипла группа из двух и больше `let`/присваиваний; пустая строка
        // идёт перед действием. Если `pp` не `let`/присваивание, перед `prev` вставит R1.
        let group_over = i >= 2
            && !prev.skip
            && !prev.has_comment
            && prev.top.point.row == stmts[i - 2].end + 1
            && matches!(stmts[i - 2].kind, Kind::Let | Kind::Assign);

        let related = linked
            && match cur.kind {
                Kind::Block => intersects(&bound(prev.node, src), &used_block(cur.node, src)),
                Kind::Expr => intersects(&bound(prev.node, src), &used(cur.node, src)),
                _ => false,
            };

        let rule = match cur.kind {
            Kind::Let | Kind::Assign if !linked => Some(Rule::Assign),
            Kind::Block if !related => Some(Rule::Block),
            Kind::Block if group_over => Some(Rule::BlockMax),
            Kind::Expr if matches!(pk, Kind::Block | Kind::Label | Kind::Item(_)) => {
                Some(Rule::ExprAfter)
            }
            Kind::Expr if linked && !related => Some(Rule::ExprUnrelated),
            Kind::Expr if linked && group_over => Some(Rule::ExprMax),
            Kind::Jump => Some(Rule::Jump),
            Kind::Tail => Some(Rule::Tail),
            Kind::Item(_) | Kind::Label if pk != cur.kind => Some(Rule::Decl),
            _ => None,
        };

        // Одно нарушение на место: R9 только если R1–R7 не сработали.
        let rule = rule.or(cur.has_comment.then_some(Rule::Comment));
        if let Some(rule) = rule {
            push(rule, cur);
        }
    }
}

fn inside_fn_body(use_decl: Node) -> bool {
    let mut p = use_decl.parent();
    while let Some(n) = p {
        match n.kind() {
            "block" => return true,
            "mod_item" => return false,
            _ => p = n.parent(),
        }
    }

    false
}

/// R8 без автоисправления; `#[rustfmt::skip]` его не отключает.
pub(crate) fn use_in_fn(tree: &Tree, src: &[u8]) -> Vec<Violation> {
    let mut out = Vec::new();

    walk(tree.root_node(), src, |node, _| {
        if node.kind() == "use_declaration" && inside_fn_body(node) {
            let top = Top::of(node);
            out.push(Violation {
                rule: Rule::UseInFn,
                row: top.point.row,
                line_start: top.line_start(),
            });
        }

        node.kind() != "token_tree"
    });

    out
}

pub(crate) fn violations(tree: &Tree, src: &[u8]) -> Vec<Violation> {
    let mut out = Vec::new();

    walk(tree.root_node(), src, |node, skipped| {
        let descend = node.kind() != "token_tree" && !skipped && !has_inner_skip(node, src);
        if descend && node.kind() == "block" {
            check_block(node, src, &mut out);
        }

        descend
    });

    out
}
