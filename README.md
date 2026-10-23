# rust-wsl

A small, syntax-aware blank-line linter and fixer for Rust, inspired by the whitespace linting style of Go's `wsl`.

`rustfmt` handles formatting; `rust-wsl` adds separation between logical groups of statements. It uses [Tree-sitter](https://tree-sitter.github.io/tree-sitter/) to inspect Rust syntax rather than matching lines with regular expressions.

## Example

Before:

```rust
fn process() -> Result<()> {
    let input = read_input()?;
    let options = Options::default();
    run(input, options)?;
    Ok(())
}
```

After:

```rust
fn process() -> Result<()> {
    let input = read_input()?;
    let options = Options::default();

    run(input, options)?;

    Ok(())
}
```

A single related binding can stay next to the statement that uses it:

```rust
let value = read_value()?;
consume(value);
```

Groups of two or more consecutive bindings or assignments are separated from the following action, even when it uses the last binding.

## Installation

Requires **Rust 1.88 or newer** and a C toolchain for the Tree-sitter dependencies.

Install from GitHub:

```sh
cargo install --git https://github.com/GoWebProd/rust-wsl --locked
```

Or build from a local checkout:

```sh
git clone https://github.com/GoWebProd/rust-wsl.git
cd rust-wsl
cargo install --path . --locked
```

## Usage

```sh
# Insert missing blank lines in place.
rust-wsl src/main.rs src/lib.rs

# Report violations without changing files.
rust-wsl --check src/main.rs src/lib.rs

# Run directly from the source checkout.
cargo run --release -- --check src/main.rs
```

Pass explicit file paths. The CLI does not recursively scan directories, expand globs itself, or read source from stdin. With no files, it exits successfully without checking anything.

To check all tracked Rust files in a Git repository on a Unix-like system:

```sh
git ls-files -z -- '*.rs' | xargs -0 rust-wsl --check
```

Run `rustfmt` first, then `rust-wsl`. It complements `rustfmt` and Clippy rather than replacing either.

### Diagnostics

```text
src/main.rs:12: R6 tail
src/main.rs:20: R8 use-in-fn: move this import to the top of the module
```

Line numbers are one-based. Check mode writes violations to stdout; operational errors go to stderr. Fix mode writes remaining import violations to stderr and leaves stdout empty.

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | No violations remain, or all fixable violations were fixed. |
| `1` | Violations found in check mode, or non-fixable import violations remain in fix mode. |
| `2` | An error occurred, such as an unreadable file, a parse error in check mode, or an unknown long option. |

Fix mode **warns and skips files that cannot be parsed**. Use check mode in CI to make parse errors fail the run.

## Rules

Rules apply to adjacent statements inside Rust blocks. Existing blank lines already satisfy separation requirements.

| Diagnostic | Requires separation before… |
| --- | --- |
| `R1 assign` | A binding or assignment following a statement that is neither a binding nor an assignment. |
| `R2 block` | A block-like statement, unless it uses a name bound or assigned by the immediately preceding statement. |
| `R3 block-max` | A block-like statement following a group of two or more bindings or assignments, even when related. |
| `R4a expr-after` | An expression statement following a block-like statement, label, or declaration. |
| `R4b expr-unrelated` | An expression statement unrelated to the immediately preceding binding or assignment. |
| `R4c expr-max` | An expression statement following a group of two or more bindings or assignments, even when related. |
| `R5 jump` | A jump statement, such as `return`, `break`, or `continue`. |
| `R6 tail` | A block's tail expression. |
| `R7 decl` | A declaration or label following a statement of a different kind. |
| `R8 use-in-fn` | Reports imports inside blocks; move them to module scope manually. |
| `R9 comment` | A statement with an attached comment, separating the comment and statement from the previous statement. |

The first statement of a block does not need a leading blank line. Only `R8` requires a manual change; the other rules insert missing blank lines.

“Related” is a syntactic heuristic based on shared identifiers, not compiler-level name resolution or data-flow analysis. For block-like statements, it examines the condition, pattern, value, and first statement of the body.

### Scope and limitations

- Rules are fixed; there is no configuration file or per-rule CLI switch.
- Blank-line checks respect `#[rustfmt::skip]`; the import rule does not.
- Macro token-tree bodies are not linted as ordinary Rust blocks, and macros are not expanded.
- The fixer only inserts blank lines. It does not remove existing blank lines, reorder imports, or rewrite expressions.
- Parseability is determined by the bundled Tree-sitter Rust grammar, not by `rustc`.

## Fixer safety

Before writing a changed file, the fixer reparses it and checks that the token sequence is unchanged. Comments and string/character literals are compared as whole tokens so their contents are preserved.

Writes use a temporary file next to the destination followed by a rename. File permissions are preserved; read-only files are rejected. When given a symbolic link, the fixer updates its target without replacing the link.

## CI

Install the tool, then check tracked Rust files:

```sh
cargo install --git https://github.com/GoWebProd/rust-wsl --locked
git ls-files -z -- '*.rs' | xargs -0 rust-wsl --check
```

For reproducible CI, pin installation to a reviewed commit using `--rev <commit>`; `--locked` pins dependencies, not the repository revision.

## Development

From the repository root:

```sh
cargo test --locked
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
```

Tests include golden input/output fixtures, library checks, and CLI behavior tests.

## License

[MIT](LICENSE).
