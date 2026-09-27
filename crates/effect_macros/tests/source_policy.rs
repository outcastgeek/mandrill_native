//! Source inventory guard for the no-magic-values requirement.
//!
//! A semantic review is still required: AST inspection cannot justify a policy.
//! These checks make new fixed numeric values and unreviewed string-expression
//! sites visible, including literals emitted inside quote/quote_spanned inputs.
use proc_macro2::{TokenStream, TokenTree};
use std::path::{Path, PathBuf};
use syn::visit::Visit;

struct Audit {
    file: PathBuf,
    item: Vec<String>,
    violations: Vec<String>,
}

impl Audit {
    fn literal(&mut self, literal: &syn::Lit) {
        let presentation = match (self.file.to_str(), self.item.last().map(String::as_str)) {
            // Exhaustive typed defect conditions select human diagnostics.
            (Some("effect/src/defect.rs"), Some("message")) => true,
            // UnsupportedSyntax AST conditions select compiler diagnostics.
            (Some("effect_macros/src/parse.rs"), Some("at")) => true,
            // Console renders the supplied message; it selects no policy.
            (Some("effect/src/console.rs"), Some("write")) => true,
            // A hygienic identifier is derived at compile time from an AST ordinal.
            (Some("effect_macros/src/async_expand.rs"), Some("checkpoint")) => true,
            // Cargo package identity used for dependency-alias resolution.
            (Some("effect_macros/src/expand.rs"), Some("EFFECT_PACKAGE_NAME")) => true,
            _ => false,
        };
        let presentation = presentation
            || match (self.file.to_str(), literal) {
                (Some("packages/effect_app/src/main.rs"), syn::Lit::Str(value)) => matches!(
                    value.value().as_str(),
                    "Input, output, and staging paths must be distinct" | "{diagnostic:?}"
                ),
                (Some("packages/effect_app/src/lib.rs"), syn::Lit::Str(value)) => matches!(
                    value.value().as_str(),
                    "{sum}"
                        | "sum={} input_bytes={}"
                        | "{error:?}"
                        | "{defect:?}"
                        | "cleanup failed after {primary}: {failures:?}"
                ),
                _ => false,
            };
        match literal {
            syn::Lit::Int(value) => self.violations.push(format!(
                "{}: fixed integer {value} needs a documented contract and inventory review",
                self.file.display()
            )),
            syn::Lit::Float(value) => self.violations.push(format!(
                "{}: fixed float {value} needs a documented contract and inventory review",
                self.file.display()
            )),
            syn::Lit::Str(value) if !presentation => self.violations.push(format!(
                "{}: unreviewed string expression {:?}",
                self.file.display(),
                value.value()
            )),
            syn::Lit::Str(_) | syn::Lit::Bool(_) => {}
            syn::Lit::ByteStr(_)
            | syn::Lit::CStr(_)
            | syn::Lit::Byte(_)
            | syn::Lit::Char(_)
            | syn::Lit::Verbatim(_) => self.violations.push(format!(
                "{}: this literal form needs an inventory review",
                self.file.display()
            )),
            _ => self.violations.push(format!(
                "{}: an unrecognized dependency literal form needs an inventory review",
                self.file.display()
            )),
        }
    }

    fn tokens(&mut self, stream: TokenStream) {
        for token in stream {
            match token {
                TokenTree::Group(group) => self.tokens(group.stream()),
                TokenTree::Literal(literal) => {
                    if let Ok(literal) = syn::parse2::<syn::Lit>(TokenTree::Literal(literal).into())
                    {
                        self.literal(&literal);
                    }
                }
                TokenTree::Ident(_) | TokenTree::Punct(_) => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for Audit {
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {
        // Rust compiler metadata (documentation, module paths, and must_use
        // diagnostics) is covered by the written inventory, not policy values.
    }

    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        self.item.push(function.sig.ident.to_string());
        syn::visit::visit_item_fn(self, function);
        let _ = self.item.pop();
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        self.item.push(function.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, function);
        let _ = self.item.pop();
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        self.item.push(item.ident.to_string());
        syn::visit::visit_item_const(self, item);
        let _ = self.item.pop();
    }

    fn visit_expr_lit(&mut self, expr: &'ast syn::ExprLit) {
        self.literal(&expr.lit);
    }

    fn visit_expr_call(&mut self, expr: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = expr.func.as_ref() {
            let mut segments = path.path.segments.iter().rev();
            if segments
                .next()
                .is_some_and(|segment| segment.ident == "new")
                && segments
                    .next()
                    .is_some_and(|segment| segment.ident == "Defect")
            {
                self.violations.push(format!(
                    "{}: a library-generated defect needs a typed identity",
                    self.file.display()
                ));
            }
        }
        syn::visit::visit_expr_call(self, expr);
    }

    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        if invocation.path.is_ident("include_str") {
            // A source-file inclusion is a Rust build contract. The inventory
            // records lib.rs including the checked-in README as crate docs.
            return;
        }
        self.tokens(invocation.tokens.clone());
    }
}

fn source_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut sources = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            sources.extend(source_files(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
    sources.sort();
    Ok(sources)
}

fn audit(file: PathBuf, source: &str) -> syn::Result<Vec<String>> {
    let syntax = syn::parse_file(source)?;
    let mut audit = Audit {
        file,
        item: Vec::new(),
        violations: Vec::new(),
    };
    audit.visit_file(&syntax);
    Ok(audit.violations)
}

#[test]
fn production_literals_match_the_reviewed_semantic_inventory()
-> Result<(), Box<dyn std::error::Error>> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("macro crate has no parent")?;
    let mut violations = Vec::new();
    for package in [
        "effect",
        "effect_macros",
        "effect_tokio",
        "effect_rayon",
        "effect_native_runtime",
        "effect_tracing",
    ] {
        for file in source_files(&crates.join(package).join("src"))? {
            let relative = file.strip_prefix(crates)?.to_owned();
            violations.extend(audit(relative, &std::fs::read_to_string(file)?)?);
        }
    }
    let workspace = crates.parent().ok_or("crates has no workspace parent")?;
    for file in source_files(&workspace.join("packages/effect_app/src"))? {
        let relative = file.strip_prefix(workspace)?.to_owned();
        violations.extend(audit(relative, &std::fs::read_to_string(file)?)?);
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    Ok(())
}

#[test]
fn the_inventory_guard_detects_constants_generated_literals_and_string_identities()
-> syn::Result<()> {
    // Regression fixtures intentionally reproduce the removed arbitrary budget
    // and the string-only defect pattern; none is executed or emitted to users.
    for source in [
        "const UNJUSTIFIED_QUANTUM: usize = 256;",
        "fn expansion() { quote!(step(256)); }",
        "fn dispatch() { let state = \"closed\"; }",
        "fn failure() { Defect::new(reason); }",
    ] {
        assert!(!audit(PathBuf::from("effect/src/regression.rs"), source)?.is_empty());
    }
    Ok(())
}
