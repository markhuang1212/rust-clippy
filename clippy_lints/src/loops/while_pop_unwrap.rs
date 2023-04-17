use clippy_utils::{
    diagnostics::{multispan_sugg_with_applicability, span_lint_and_then},
    match_def_path, paths,
    source::snippet,
    SpanlessEq,
};
use rustc_errors::Applicability;
use rustc_hir::{Expr, ExprKind, Pat, Stmt, StmtKind, UnOp};
use rustc_lint::LateContext;
use rustc_span::Span;
use std::borrow::Cow;

use super::WHILE_POP_UNWRAP;

/// The kind of statement that the `pop()` call appeared in.
///
/// Depending on whether the value was assigned to a variable or not changes what pattern
/// we use for the suggestion.
#[derive(Copy, Clone)]
enum PopStmt<'hir> {
    /// `x.pop().unwrap()` was and assigned to a variable.
    /// The pattern of this local variable will be used and the local statement
    /// is deleted in the suggestion.
    Local(&'hir Pat<'hir>),
    /// `x.pop().unwrap()` appeared in an arbitrary expression and was not assigned to a variable.
    /// The suggestion will use some placeholder identifier and the `x.pop().unwrap()` expression
    /// is replaced with that identifier.
    Anonymous,
}

fn report_lint(cx: &LateContext<'_>, pop_span: Span, pop_stmt_kind: PopStmt<'_>, loop_span: Span, receiver_span: Span) {
    span_lint_and_then(
        cx,
        WHILE_POP_UNWRAP,
        pop_span,
        "you seem to be trying to pop elements from a `Vec` in a loop",
        |diag| {
            let (pat, pop_replacement) = match pop_stmt_kind {
                PopStmt::Local(pat) => (snippet(cx, pat.span, ".."), String::new()),
                PopStmt::Anonymous => (Cow::Borrowed("element"), "element".into()),
            };

            let loop_replacement = format!("while let Some({}) = {}.pop()", pat, snippet(cx, receiver_span, ".."));
            multispan_sugg_with_applicability(
                diag,
                "consider using a `while..let` loop",
                Applicability::MachineApplicable,
                [(loop_span, loop_replacement), (pop_span, pop_replacement)],
            );
        },
    );
}

fn match_method_call(cx: &LateContext<'_>, expr: &Expr<'_>, method: &[&str]) -> bool {
    if let ExprKind::MethodCall(..) = expr.kind
        && let Some(id) = cx.typeck_results().type_dependent_def_id(expr.hir_id)
    {
        match_def_path(cx, id, method)
    } else {
        false
    }
}

fn is_vec_pop_unwrap(cx: &LateContext<'_>, expr: &Expr<'_>, is_empty_recv: &Expr<'_>) -> bool {
    if (match_method_call(cx, expr, &paths::OPTION_UNWRAP) || match_method_call(cx, expr, &paths::OPTION_EXPECT))
        && let ExprKind::MethodCall(_, unwrap_recv, ..) = expr.kind
        && match_method_call(cx, unwrap_recv, &paths::VEC_POP)
        && let ExprKind::MethodCall(_, pop_recv, ..) = unwrap_recv.kind
    {
        // make sure they're the same `Vec`
        SpanlessEq::new(cx).eq_expr(pop_recv, is_empty_recv)
    } else {
        false
    }
}

fn check_local(cx: &LateContext<'_>, stmt: &Stmt<'_>, is_empty_recv: &Expr<'_>, loop_span: Span) {
    if let StmtKind::Local(local) = stmt.kind
        && let Some(init) = local.init
        && is_vec_pop_unwrap(cx, init, is_empty_recv)
    {
        report_lint(cx, stmt.span, PopStmt::Local(local.pat), loop_span, is_empty_recv.span);
    }
}

fn check_call_arguments(cx: &LateContext<'_>, stmt: &Stmt<'_>, is_empty_recv: &Expr<'_>, loop_span: Span) {
    if let StmtKind::Semi(expr) | StmtKind::Expr(expr) = stmt.kind {
        if let ExprKind::MethodCall(.., args, _) | ExprKind::Call(_, args) = expr.kind {
            let offending_arg = args
                .iter()
                .find_map(|arg| is_vec_pop_unwrap(cx, arg, is_empty_recv).then_some(arg.span));

            if let Some(offending_arg) = offending_arg {
                report_lint(cx, offending_arg, PopStmt::Anonymous, loop_span, is_empty_recv.span);
            }
        }
    }
}

pub(super) fn check<'tcx>(cx: &LateContext<'tcx>, full_cond: &'tcx Expr<'_>, body: &'tcx Expr<'_>, loop_span: Span) {
    if let ExprKind::Unary(UnOp::Not, cond) = full_cond.kind
        && let ExprKind::MethodCall(_, is_empty_recv, _, _) = cond.kind
        && match_method_call(cx, cond, &paths::VEC_IS_EMPTY)
        && let ExprKind::Block(body, _) = body.kind
        && let Some(stmt) = body.stmts.first()
    {
        check_local(cx, stmt, is_empty_recv, loop_span);
        check_call_arguments(cx, stmt, is_empty_recv, loop_span);
    }
}
