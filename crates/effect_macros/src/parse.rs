//! Parsing of Effect macro control flow.

use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Expr, Stmt};

/// Variants of unsupported syntax used by parse.
enum UnsupportedSyntax {
    BranchAssignment,
    NestedTry,
    NestedReturn,
    LetElse,
}

impl UnsupportedSyntax {
    // AST conditions select these diagnostics; diagnostic text is presentation.
    ///
    /// # Parameters
    /// - `span` (`proc_macro2::Span`): The span supplied to this operation.
    ///
    /// # Returns
    /// Type: `syn::Error`.
    /// Returns `syn::Error`.
    fn at(self, span: proc_macro2::Span) -> syn::Error {
        let message = match self {
            Self::BranchAssignment => {
                "Assignments inside effectful branches are not supported yet; return updated values from the branch or use an explicit shared state service"
            }
            Self::NestedTry => {
                "Effect ? must be the entire initializer, expression statement, or tail expression; bind the intermediate value first"
            }
            Self::NestedReturn => {
                "Effect return must be a top-level workflow statement; use a branch tail value inside effectful control flow"
            }
            Self::LetElse => "let-else workflow bindings are not supported yet",
        };
        syn::Error::new(span, message)
    }
}

/// Typed validate used by parse.
struct Validate {
    error: Option<syn::Error>,
}

#[derive(Default)]
/// Typed effect steps used by parse.
struct EffectSteps {
    found: bool,
}

impl<'ast> Visit<'ast> for EffectSteps {
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_try(&mut self, _: &'ast syn::ExprTry) {
        self.found = true;
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_item(&mut self, _: &'ast syn::Item) {}
}

/// # Parameters
/// - `expr` (`&Expr`): Rust syntax expression parsed or expanded by this macro.
///
/// # Returns
/// Type: `bool`.
/// Returns whether the requested condition holds.
pub(crate) fn has_effect_steps(expr: &Expr) -> bool {
    let mut visitor = EffectSteps::default();
    visitor.visit_expr(expr);
    visitor.found
}

/// Typed branch assignments used by parse.
struct BranchAssignments {
    error: Option<syn::Error>,
}

impl BranchAssignments {
    ///
    /// # Parameters
    /// - `span` (`proc_macro2::Span`): The span supplied to this operation.
    ///
    /// # Returns
    /// Type: `()`.
    fn reject(&mut self, span: proc_macro2::Span) {
        self.error = Some(UnsupportedSyntax::BranchAssignment.at(span));
    }
}

impl<'ast> Visit<'ast> for BranchAssignments {
    ///
    /// # Parameters
    /// - `expr` (`&'ast syn::ExprAssign`): Rust syntax expression parsed or expanded by this macro.
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_assign(&mut self, expr: &'ast syn::ExprAssign) {
        self.reject(expr.span());
    }
    ///
    /// # Parameters
    /// - `expr` (`&'ast syn::ExprBinary`): Rust syntax expression parsed or expanded by this macro.
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_binary(&mut self, expr: &'ast syn::ExprBinary) {
        if matches!(
            expr.op,
            syn::BinOp::AddAssign(_)
                | syn::BinOp::SubAssign(_)
                | syn::BinOp::MulAssign(_)
                | syn::BinOp::DivAssign(_)
                | syn::BinOp::RemAssign(_)
                | syn::BinOp::BitXorAssign(_)
                | syn::BinOp::BitAndAssign(_)
                | syn::BinOp::BitOrAssign(_)
                | syn::BinOp::ShlAssign(_)
                | syn::BinOp::ShrAssign(_)
        ) {
            self.reject(expr.span());
        } else {
            syn::visit::visit_expr_binary(self, expr);
        }
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_item(&mut self, _: &'ast syn::Item) {}
}

impl<'ast> Visit<'ast> for Validate {
    ///
    /// # Parameters
    /// - `expr` (`&'ast syn::ExprTry`): Rust syntax expression parsed or expanded by this macro.
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_try(&mut self, expr: &'ast syn::ExprTry) {
        self.error = Some(UnsupportedSyntax::NestedTry.at(expr.span()));
    }
    ///
    /// # Parameters
    /// - `expr` (`&'ast syn::ExprReturn`): Rust syntax expression parsed or expanded by this macro.
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_return(&mut self, expr: &'ast syn::ExprReturn) {
        self.error = Some(UnsupportedSyntax::NestedReturn.at(expr.span()));
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_item(&mut self, _: &'ast syn::Item) {}
}

/// # Parameters
/// - `expr` (`&Expr`): Rust syntax expression parsed or expanded by this macro.
///
/// # Returns
/// Type: `syn::Result<()>`.
pub(crate) fn pure(expr: &Expr) -> syn::Result<()> {
    let mut visitor = Validate { error: None };
    visitor.visit_expr(expr);
    visitor.error.map_or(Ok(()), Err)
}

/// # Parameters
/// - `statements` (`&[Stmt]`): Rust statements parsed and lowered by the procedural macro.
///
/// # Returns
/// Type: `syn::Result<()>`.
pub(crate) fn validate(statements: &[Stmt]) -> syn::Result<()> {
    validate_statements(statements, true)
}

///
/// # Parameters
/// - `expr` (`&Expr`): Rust syntax expression parsed or expanded by this macro.
///
/// # Returns
/// Type: `syn::Result<()>`.
fn workflow_expression(expr: &Expr) -> syn::Result<()> {
    if has_effect_steps(expr)
        && (matches!(expr, Expr::If(_) | Expr::Match(_))
            || matches!(expr, Expr::Block(block) if block.label.is_none()))
    {
        let mut visitor = BranchAssignments { error: None };
        visitor.visit_expr(expr);
        if let Some(error) = visitor.error {
            return Err(error);
        }
    }
    if let Expr::Try(expr) = expr {
        return pure(&expr.expr);
    }
    if let Expr::If(expr) = expr {
        pure(&expr.cond)?;
        validate_statements(&expr.then_branch.stmts, false)?;
        if let Some((_, other)) = &expr.else_branch {
            workflow_expression(other)?;
        }
        return Ok(());
    }
    if let Expr::Match(expr) = expr {
        pure(&expr.expr)?;
        for arm in &expr.arms {
            let mut visitor = Validate { error: None };
            visitor.visit_pat(&arm.pat);
            visitor.error.map_or(Ok(()), Err)?;
            workflow_expression(&arm.body)?;
        }
        return Ok(());
    }
    if let Expr::Block(expr) = expr
        && expr.label.is_none()
    {
        return validate_statements(&expr.block.stmts, false);
    }
    pure(expr)
}

///
/// # Parameters
/// - `statements` (`&[Stmt]`): Rust statements parsed and lowered by the procedural macro.
/// - `allow_return` (`bool`): The allow return supplied to this operation.
///
/// # Returns
/// Type: `syn::Result<()>`.
fn validate_statements(statements: &[Stmt], allow_return: bool) -> syn::Result<()> {
    for statement in statements {
        match statement {
            Stmt::Local(local) => {
                if let Some(init) = &local.init {
                    if init.diverge.is_some() {
                        return Err(UnsupportedSyntax::LetElse.at(local.span()));
                    }
                    workflow_expression(&init.expr)?;
                }
            }
            Stmt::Expr(Expr::Try(expr), _) => pure(&expr.expr)?,
            Stmt::Expr(Expr::Return(expr), _) if allow_return => {
                if let Some(value) = &expr.expr {
                    pure(value)?;
                }
            }
            Stmt::Expr(expr, _) => workflow_expression(expr)?,
            Stmt::Item(_) | Stmt::Macro(_) => {}
        }
    }
    Ok(())
}

#[derive(Default)]
/// Typed loops used by parse.
struct Loops {
    found: bool,
}
impl<'ast> Visit<'ast> for Loops {
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_loop(&mut self, _: &'ast syn::ExprLoop) {
        self.found = true;
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_while(&mut self, _: &'ast syn::ExprWhile) {
        self.found = true;
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_for_loop(&mut self, _: &'ast syn::ExprForLoop) {
        self.found = true;
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}
    ///
    /// # Returns
    /// Type: `()`.
    fn visit_item(&mut self, _: &'ast syn::Item) {}
}
/// # Parameters
/// - `statements` (`&[Stmt]`): Rust statements parsed and lowered by the procedural macro.
///
/// # Returns
/// Type: `bool`.
/// Returns whether the requested condition holds.
pub(crate) fn has_loops(statements: &[Stmt]) -> bool {
    let mut loops = Loops::default();
    for statement in statements {
        loops.visit_stmt(statement);
    }
    loops.found
}
