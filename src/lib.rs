mod rules;

use anyhow::{Context, Result, bail};
use tree_sitter::{Parser, Tree};

pub use rules::{Rule, Violation};

const MAX_PASSES: usize = 5;

/// Узлы, чей текст сравнивается целиком: пробелы внутри них значимы.
const ATOMIC: &[&str] = &[
    "line_comment",
    "block_comment",
    "string_literal",
    "raw_string_literal",
    "char_literal",
];

/// `Fixed::text == None` — файл не изменился; `leftover` — нарушения без автоисправления, строки — в исходном тексте.
#[derive(Debug, PartialEq, Eq)]
pub struct Fixed {
    pub text: Option<Vec<u8>>,
    pub leftover: Vec<Violation>,
}

fn parse(src: &[u8]) -> Result<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .context("load the Rust grammar")?;

    parser
        .parse(src, None)
        .context("tree-sitter returned no tree")
}

/// `None` — файл не разобрался без ошибок.
fn parse_clean(src: &[u8]) -> Result<Option<Tree>> {
    let tree = parse(src)?;

    Ok((!tree.root_node().has_error()).then_some(tree))
}

/// `None` — файл не разобрался, проверить его нельзя.
pub fn check(src: &[u8]) -> Result<Option<Vec<Violation>>> {
    let Some(tree) = parse_clean(src)? else {
        return Ok(None);
    };

    let mut v = rules::violations(&tree, src);
    v.extend(rules::use_in_fn(&tree, src));
    v.sort_by_key(|x| (x.row, x.rule));

    Ok(Some(v))
}

fn tokens<'a>(tree: &Tree, src: &'a [u8]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();

    rules::walk(tree.root_node(), src, |node, _| {
        if ATOMIC.contains(&node.kind()) {
            out.push(&src[node.start_byte()..node.end_byte()]);

            return false;
        }

        if node.child_count() > 0 {
            return true;
        }

        let t = &src[node.start_byte()..node.end_byte()];
        if !t.iter().all(|b| b.is_ascii_whitespace()) {
            out.push(t);
        }

        false
    });

    out
}

fn tokens_equal(ta: &Tree, a: &[u8], tb: &Tree, b: &[u8]) -> bool {
    tokens(ta, a) == tokens(tb, b)
}

#[doc(hidden)]
pub fn same_tokens(a: &[u8], b: &[u8]) -> Result<bool> {
    let (ta, tb) = (parse(a)?, parse(b)?);

    Ok(tokens_equal(&ta, a, &tb, b))
}

fn apply(src: &[u8], violations: &[Violation]) -> Vec<u8> {
    let mut at: Vec<usize> = violations.iter().map(|v| v.line_start).collect();
    at.sort_unstable();
    at.dedup();

    let mut out = Vec::with_capacity(src.len() + at.len() * 2);

    let mut last = 0;
    for off in at {
        out.extend_from_slice(&src[last..off]);
        out.extend_from_slice(if off >= 2 && &src[off - 2..off] == b"\r\n" {
            b"\r\n"
        } else {
            b"\n"
        });

        last = off;
    }

    out.extend_from_slice(&src[last..]);

    out
}

/// `None` — файл не разобрался, его пропускают.
pub fn fix(src: &[u8]) -> Result<Option<Fixed>> {
    let Some(orig) = parse_clean(src)? else {
        return Ok(None);
    };

    // Результат последнего прохода; пока `None`, актуален исходный текст.
    let mut edited: Option<(Vec<u8>, Tree)> = None;
    for pass in 0..=MAX_PASSES {
        let (text, tree) = match &edited {
            Some((t, tr)) => (t.as_slice(), tr),
            None => (src, &orig),
        };

        let v = rules::violations(tree, text);
        if v.is_empty() {
            break;
        }

        if pass == MAX_PASSES {
            bail!(
                "did not converge in {MAX_PASSES} passes: {} violations left (rule bug)",
                v.len()
            );
        }

        let next = apply(text, &v);
        let tree = parse_clean(&next)?
            .context("blank line insertion produced a parse error (offset bug)")?;

        edited = Some((next, tree));
    }

    // lint-staged при коде 1 откатывает файл, поэтому строки — по исходному тексту.
    let leftover = rules::use_in_fn(&orig, src);
    let Some((text, tree)) = edited else {
        return Ok(Some(Fixed {
            text: None,
            leftover,
        }));
    };

    if !tokens_equal(&orig, src, &tree, &text) {
        bail!("token sequence changed after insertion; file left untouched (offset bug)");
    }

    Ok(Some(Fixed {
        text: Some(text),
        leftover,
    }))
}
