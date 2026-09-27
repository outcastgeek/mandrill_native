use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Expr, Stmt};

enum UnsupportedSyntax {
    BranchAssignment,
    NestedTry,
    NestedReturn,
    LetElse,
}

impl UnsupportedSyntax {
    // AST conditions select these diagnostics; diagnostic text is presentation.
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

struct Validate {
    error: Option<syn::Error>,
}

#[derive(Default)]
struct EffectSteps {
    found: bool,
}

impl<'ast> Visit<'ast> for EffectSteps {
    fn visit_expr_try(&mut self, _: &'ast syn::ExprTry) {
        self.found = true;
    }
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}
    fn visit_item(&mut self, _: &'ast syn::Item) {}
}

pub(crate) fn has_effect_steps(expr: &Expr) -> bool {
    let mut visitor = EffectSteps::default();
    visitor.visit_expr(expr);
    visitor.found
}

struct BranchAssignments {
    error: Option<syn::Error>,
}

impl BranchAssignments {
    fn reject(&mut self, span: proc_macro2::Span) {
        self.error = Some(UnsupportedSyntax::BranchAssignment.at(span));
    }
}

impl<'ast> Visit<'ast> for BranchAssignments {
    fn visit_expr_assign(&mut self, expr: &'ast syn::ExprAssign) {
        self.reject(expr.span());
    }
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
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}
    fn visit_item(&mut self, _: &'ast syn::Item) {}
}

impl<'ast> Visit<'ast> for Validate {
    fn visit_expr_try(&mut self, expr: &'ast syn::ExprTry) {
        self.error = Some(UnsupportedSyntax::NestedTry.at(expr.span()));
    }
    fn visit_expr_return(&mut self, expr: &'ast syn::ExprReturn) {
        self.error = Some(UnsupportedSyntax::NestedReturn.at(expr.span()));
    }
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}
    fn visit_item(&mut self, _: &'ast syn::Item) {}
}

pub(crate) fn pure(expr: &Expr) -> syn::Result<()> {
    let mut visitor = Validate { error: None };
    visitor.visit_expr(expr);
    visitor.error.map_or(Ok(()), Err)
}

pub(crate) fn validate(statements: &[Stmt]) -> syn::Result<()> {
    validate_statements(statements, true)
}

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
            if let Some((_, guard)) = &arm.guard {
                pure(guard)?;
            }
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
struct Loops {
    found: bool,
}
impl<'ast> Visit<'ast> for Loops {
    fn visit_expr_loop(&mut self, _: &'ast syn::ExprLoop) {
        self.found = true;
    }
    fn visit_expr_while(&mut self, _: &'ast syn::ExprWhile) {
        self.found = true;
    }
    fn visit_expr_for_loop(&mut self, _: &'ast syn::ExprForLoop) {
        self.found = true;
    }
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}
    fn visit_item(&mut self, _: &'ast syn::Item) {}
}
pub(crate) fn has_loops(statements: &[Stmt]) -> bool {
    let mut loops = Loops::default();
    for statement in statements {
        loops.visit_stmt(statement);
    }
    loops.found
}
