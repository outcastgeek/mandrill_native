//! Source lint for multiline macro braces in the core café examples.
//! Only opening-line placement is checked; comments are skipped and literal interiors stay opaque.
use proc_macro2::{Delimiter, LineColumn, Span, TokenStream, TokenTree};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use syn::visit::Visit;

/// Opening brace of a multiline group whose first code token shares its line.
#[derive(Debug, PartialEq, Eq)]
struct Violation {
    /// One-based source line and zero-based source column of the opening brace.
    opening: LineColumn,
}
impl Violation {
    /// Name the source location and the exact correction required by this rule.
    fn diagnostic(&self, file: &Path) -> String {
        format!(
            "{}:{}:{}: macro layout: move the first code token of this multiline braced group to a new line after `{{`",
            file.display(),
            self.opening.line,
            self.opening.column + 1,
        )
    }
}

/// Parsed macro visitor; macro names and aliases do not affect enforcement.
#[derive(Default)]
struct Layout {
    /// Source positions needing whitespace-only corrections.
    violations: Vec<Violation>,
}
impl Layout {
    /// Check one pair of actual brace spans, excluding empty/comment-only input.
    fn braces(&mut self, opening: Span, closing: Span, tokens: TokenStream) {
        let opening = opening.start();
        if opening.line != closing.start().line
            && first_code(tokens).is_some_and(|token| token.span().start().line == opening.line)
        {
            self.violations.push(Violation { opening });
        }
    }

    /// Attribute arguments are opaque to syn's visitor; parse invocation-shaped token triples.
    fn attribute_macros(&mut self, tokens: TokenStream) {
        let tokens = tokens.into_iter().collect::<Vec<_>>();
        let mut remaining = tokens.as_slice();
        while let Some((first, rest)) = remaining.split_first() {
            if let [
                TokenTree::Ident(name),
                TokenTree::Punct(bang),
                TokenTree::Group(group),
                tail @ ..,
            ] = remaining
                && bang.as_char() == '!'
                && let Ok(invocation) = syn::parse2::<syn::Macro>(
                    [
                        TokenTree::Ident(name.clone()),
                        TokenTree::Punct(bang.clone()),
                        TokenTree::Group(group.clone()),
                    ]
                    .into_iter()
                    .collect(),
                )
            {
                self.visit_macro(&invocation);
                remaining = tail;
            } else {
                if let TokenTree::Group(group) = first {
                    self.attribute_macros(group.stream());
                }
                remaining = rest;
            }
        }
    }

    /// Inspect all nested input groups, without interpreting custom DSL grammar or literals.
    fn groups(&mut self, tokens: TokenStream) {
        for token in tokens {
            if let TokenTree::Group(group) = token {
                if group.delimiter() == Delimiter::Brace {
                    self.braces(group.span_open(), group.span_close(), group.stream());
                }
                self.groups(group.stream());
            }
        }
    }
}
impl<'ast> Visit<'ast> for Layout {
    /// Include invocations in attribute argument lists as well as ordinary Rust expressions.
    fn visit_attribute(&mut self, attribute: &'ast syn::Attribute) {
        if let syn::Meta::List(list) = &attribute.meta {
            self.attribute_macros(list.tokens.clone());
        }
        syn::visit::visit_attribute(self, attribute);
    }

    /// Check every invocation, including qualified names and imported aliases.
    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        if let syn::MacroDelimiter::Brace(brace) = &invocation.delimiter {
            self.braces(
                brace.span.open(),
                brace.span.close(),
                invocation.tokens.clone(),
            );
        }
        self.groups(invocation.tokens.clone());
    }
}

/// Normal comments vanish during tokenization; skip synthetic attributes from doc comments too.
fn first_code(tokens: TokenStream) -> Option<TokenTree> {
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        let doc_comment = matches!(&token, TokenTree::Punct(punctuation) if punctuation.as_char() == '#')
            && token.span().source_text().is_some_and(|text| {
                ["///", "//!", "/**", "/*!"]
                    .iter()
                    .any(|prefix| text.starts_with(prefix))
            });
        if doc_comment {
            if matches!(tokens.peek(), Some(TokenTree::Punct(punctuation)) if punctuation.as_char() == '!')
            {
                let _ = tokens.next();
            }
            let _ = tokens.next(); // The paired synthetic [doc = "..."] group.
        } else {
            return Some(token);
        }
    }
    None
}

/// Parse Rust once; custom macro input remains an opaque, accurately spanned token stream.
fn audit(source: &str) -> syn::Result<Vec<Violation>> {
    let syntax = syn::parse_file(source)?;
    let mut layout = Layout::default();
    layout.visit_file(&syntax);
    Ok(layout.violations)
}

/// Collect example sources recursively and sort for stable, actionable diagnostics.
fn rust_sources(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut sources = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            sources.extend(rust_sources(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
    sources.sort();
    Ok(sources)
}

/// Normal package tests enforce this layout rule across the core stories and their support.
#[test]
fn cafe_examples_follow_macro_layout() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut diagnostics = Vec::new();
    let directories = [
        "effect",
        "effect_sql",
        "effect_server",
        "effect_server_identity",
        "effect_server_web",
        "effect_otel",
    ];
    let mut sources = Vec::new();
    for directory in directories {
        sources.extend(rust_sources(
            &root.join(format!("../{directory}/examples")),
        )?);
    }
    for file in sources {
        let source =
            fs::read_to_string(&file).map_err(|error| format!("{}: {error}", file.display()))?;
        let violations = audit(&source).map_err(|error| format!("{}: {error}", file.display()))?;
        diagnostics.extend(
            violations
                .iter()
                .map(|violation| violation.diagnostic(&file)),
        );
    }
    assert!(diagnostics.is_empty(), "{}", diagnostics.join("\n"));
    Ok(())
}

/// Qualified invocations, aliases and nested input braces are all checked with exact locations.
#[test]
fn rejects_multiline_opening_line_code() -> Result<(), Box<dyn std::error::Error>> {
    let source = "use cafe::recipe as order;\nfn visit() {\n    cafe::schema! { struct Order { customer: string(),\n        quantity: integer() }\n    };\n    order!({ first_step();\n        next_step(); });\n}";
    let violations = audit(source)?;
    assert_eq!(violations.len(), 3);
    let attribute = "#[describe(qualified::recipe! { item,\n    next })] fn visit() {}";
    assert_eq!(audit(attribute)?.len(), 1);
    let first = violations.first().ok_or("missing opening-line violation")?;
    assert_eq!(
        first.opening,
        LineColumn {
            line: 3,
            column: 18
        }
    );
    assert!(
        first
            .diagnostic(Path::new("example.rs"))
            .starts_with("example.rs:3:19: macro layout:")
    );
    Ok(())
}

/// Comments, doc comments and fake macro/bracing text inside raw strings are not code tokens.
#[test]
fn accepts_comments_literals_and_one_line_groups() -> syn::Result<()> {
    let source = r####"fn visit() {
    recipe! { value }
    recipe! { /* comment with { and } across
                 two lines */
        value
    }
    recipe! { /** pickup documentation */
        value
    }
    recipe! { /*! pickup documentation */
        value
    }
    recipe! { // a comment beside the brace is allowed
        r#"fake! { code
           nested: { more
           }
        }"#
    }
    recipe! { /* comment only
    */ }
}"####;
    assert!(audit(source)?.is_empty());
    Ok(())
}

/// Correct multiline nested bodies and true one-line inner groups remain permitted.
#[test]
fn accepts_nested_macro_input_groups() -> syn::Result<()> {
    let source = "fn visit() { qualified::recipe!(\n    {\n        run |shop| {\n            nested! { shop.name() }\n        }\n    }\n); }";
    assert!(audit(source)?.is_empty());
    Ok(())
}
