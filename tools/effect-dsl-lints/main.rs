//! Compiler-resolved enforcement of macro-owned DSL construction.
#![feature(rustc_private)]

use rustc_middle::ty::print::{with_no_trimmed_paths, with_no_visible_paths};
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{CRATE_DEF_ID, DefId, LOCAL_CRATE};
use rustc_hir::intravisit::{self, Visitor};
use rustc_hir::{Expr, ExprKind};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_middle::ty::{self, TyCtxt};
use rustc_session::lint::Level;
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::{Span, sym};
mod kernel;
mod registry;

/// Use defining paths for policy identity, regardless of visible reexports.
fn definition_path(tcx: TyCtxt<'_>, id: DefId) -> String {
    with_no_trimmed_paths!(with_no_visible_paths!(tcx.def_path_str(id)))
}

declare_lint! { pub MANUAL_DSL_CONSTRUCTION, Forbid,
"DSL construction and composition must use the owning Effect macro",
report_in_external_macro }
declare_lint_pass!(Dsl => [MANUAL_DSL_CONSTRUCTION]);

/// Resolve the defining ADT of an associated function, independent of aliases/UFCS.
fn associated_owner(tcx: TyCtxt<'_>, id: DefId) -> Option<DefId> {
    let parent = tcx.opt_parent(id)?;
    if matches!(tcx.def_kind(parent), DefKind::Impl { .. }) {
        if let ty::Adt(adt, _) = tcx
            .type_of(parent)
            .instantiate_identity()
            .skip_norm_wip()
            .kind()
        {
            return Some(adt.did());
        }
    }
    None
}

fn canonical_type(tcx: TyCtxt<'_>, ty: ty::Ty<'_>, path: &str) -> bool {
    let ty::Adt(adt, _) = ty.kind() else {
        return false;
    };
    let id = adt.did();
    let actual = if id.is_local() {
        format!("{}::{}", tcx.crate_name(id.krate), definition_path(tcx, id))
    } else {
        definition_path(tcx, id)
    };
    actual == path
}

fn conversion_trait(tcx: TyCtxt<'_>, id: DefId) -> bool {
    let trait_id = tcx
        .opt_parent(id)
        .and_then(|parent| match tcx.def_kind(parent) {
            DefKind::Trait => Some(parent),
            DefKind::Impl { of_trait: true } => Some(
                tcx.impl_trait_ref(parent)
                    .instantiate_identity()
                    .skip_norm_wip()
                    .def_id,
            ),
            _ => None,
        });
    trait_id.is_some_and(|id| {
        tcx.is_diagnostic_item(sym::From, id) || tcx.is_diagnostic_item(sym::Into, id)
    })
}

/// Only the real Effect<()> -> Release conversion is a DSL construction route.
fn release_conversion<'tcx>(tcx: TyCtxt<'tcx>, id: DefId, args: ty::GenericArgsRef<'tcx>) -> bool {
    if !conversion_trait(tcx, id) {
        return false;
    }
    let signature = tcx.fn_sig(id).instantiate(tcx, args).skip_binder();
    let Some(source) = signature.inputs().first() else {
        return false;
    };
    canonical_type(tcx, *source, "effect::program::Effect")
        && matches!(source.kind(), ty::Adt(_, args) if args.type_at(0).is_unit())
        && canonical_type(tcx, signature.output(), "effect::resource::Release")
}

/// A generic Into<Release> helper can otherwise hide the same conversion until
/// monomorphization. Require the DSL for construction into this resolved owner;
/// transferring an already constructed Release is an ordinary identity conversion.
fn release_construction<'tcx>(
    tcx: TyCtxt<'tcx>,
    id: DefId,
    args: ty::GenericArgsRef<'tcx>,
) -> bool {
    if !conversion_trait(tcx, id) {
        return false;
    }
    let signature = tcx.fn_sig(id).instantiate(tcx, args).skip_binder();
    canonical_type(tcx, signature.output(), "effect::resource::Release")
        && signature
            .inputs()
            .first()
            .is_some_and(|source| !canonical_type(tcx, *source, "effect::resource::Release"))
}

/// Lazy codec mode owns this standard constructor only for actual codec values.
/// The instantiated signature also covers aliases, UFCS and callable references.
fn lazy_codec_construction<'tcx>(
    tcx: TyCtxt<'tcx>,
    id: DefId,
    args: ty::GenericArgsRef<'tcx>,
) -> bool {
    if tcx.item_name(id).as_str() != "new" {
        return false;
    }
    let output = tcx.fn_sig(id).instantiate(tcx, args).skip_binder().output();
    if !canonical_type(tcx, output, "std::sync::lazy_lock::LazyLock") {
        return false;
    }
    let ty::Adt(_, args) = output.kind() else {
        return false;
    };
    let value = args.type_at(0);
    canonical_type(tcx, value, "effect::schema::codec::Codec")
        || canonical_type(tcx, value, "effect::schema::codec::PureCodec")
}

/// Only tokens produced by this repository's actual procedural macro exports are trusted.
/// Interpolated handwritten tokens retain their original spans and are checked normally.
fn generated(tcx: TyCtxt<'_>, span: Span) -> bool {
    let data = span.ctxt().outer_expn_data();
    data.macro_def_id.is_some_and(|id| {
        tcx.crate_name(id.krate).as_str() == "effect_macros"
            && registry::MACROS.contains(&tcx.item_name(id).as_str())
            && registry::MACRO_OWNERS
                .iter()
                .any(|(name, _)| *name == tcx.item_name(id).as_str())
            && std::iter::once(LOCAL_CRATE)
                .chain(tcx.crates(()).iter().copied())
                .filter(|krate| registry::facade(tcx.crate_name(*krate).as_str()))
                .any(|krate| {
                    let children = if krate == LOCAL_CRATE {
                        tcx.module_children_local(CRATE_DEF_ID)
                    } else {
                        tcx.module_children(krate.as_def_id())
                    };
                    children.iter().any(|child| {
                        child.vis.is_public()
                            && matches!(child.res, Res::Def(DefKind::Macro(_), export) if export == id)
                    })
                })
    })
}

/// Check exact native constructor/continuation edges and sealed implementation identities.
fn primitive(cx: &LateContext<'_>, expr: &Expr<'_>, callee: DefId) -> bool {
    if definition_path(cx.tcx, expr.hir_id.owner.def_id.to_def_id())
        .split("::")
        .any(|part| part == "tests")
    {
        return false;
    }
    let target = associated_owner(cx.tcx, callee);
    let mut caller = expr.hir_id.owner.def_id.to_def_id();
    loop {
        // The macro's actual factory adapter creates this exact native selection;
        // routing it through identity! would recurse into the same adapter.
        if caller.is_local()
            && callee.is_local()
            && cx.tcx.crate_name(caller.krate).as_str() == "effect_server_identity"
            && matches!(cx.tcx.def_kind(caller), DefKind::Fn)
            && definition_path(cx.tcx, caller) == "definition::definition_with_factory"
            && target.is_some_and(|owner| {
                definition_path(cx.tcx, owner) == "definition::IdentityDefinition"
            })
            && cx.tcx.item_name(callee).as_str() == "new"
        {
            return true;
        }
        // The macro's actual borrowed-primary adapter converts its generic
        // cleanup result. This one resolved Into edge grants no other helpers
        // permission to hide Effect -> Release construction behind a type parameter.
        if caller.is_local()
            && cx.tcx.crate_name(caller.krate).as_str() == "effect"
            && matches!(cx.tcx.def_kind(caller), DefKind::Fn)
            && definition_path(cx.tcx, caller) == "__private::primary_release"
            && cx.tcx.item_name(callee).as_str() == "into"
            && conversion_trait(cx.tcx, callee)
            && matches!(expr.kind, ExprKind::MethodCall(..))
            && release_construction(cx.tcx, callee, cx.typeck_results().node_args(expr.hir_id))
        {
            return true;
        }
        if caller.is_local()
            && callee.is_local()
            && cx.tcx.crate_name(caller.krate).as_str() == "effect"
        {
            let callee_path = target.map_or_else(
                || definition_path(cx.tcx, callee),
                |owner| {
                    format!(
                        "{}::{}",
                        definition_path(cx.tcx, owner),
                        cx.tcx.item_name(callee)
                    )
                },
            );
            if matches!(cx.tcx.def_kind(caller), DefKind::Fn)
                && kernel::transition(&definition_path(cx.tcx, caller), &callee_path)
            {
                return true;
            }
            if matches!(cx.tcx.def_kind(caller), DefKind::AssocFn)
                && cx.tcx.item_name(caller).as_str() == "register_in"
                && callee_path == "cluster::entity_definition::ClusterDefinition::register"
                && associated_owner(cx.tcx, caller).is_some_and(|owner| {
                    definition_path(cx.tcx, owner) == "cluster::entity_definition::EntityDefinition"
                })
                && cx.tcx.opt_parent(caller).is_some_and(|parent| {
                    matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: true })
                        && definition_path(
                            cx.tcx,
                            cx.tcx
                                .impl_trait_ref(parent)
                                .instantiate_identity()
                                .skip_norm_wip()
                                .def_id,
                        ) == "cluster::entity_definition::ClusterEntry"
                })
            {
                return true;
            }
            if matches!(cx.tcx.def_kind(caller), DefKind::AssocFn)
                && cx.tcx.item_name(caller).as_str() == "in_context"
                && callee_path == "internal::macro_future::AsyncBindable::into_scoped_effect"
                && associated_owner(cx.tcx, caller).is_some_and(|owner| {
                    definition_path(cx.tcx, owner) == "resource::ResourceBinding"
                })
                && cx.tcx.opt_parent(caller).is_some_and(|parent| {
                    matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: true })
                        && definition_path(
                            cx.tcx,
                            cx.tcx
                                .impl_trait_ref(parent)
                                .instantiate_identity()
                                .skip_norm_wip()
                                .def_id,
                        ) == "reactivity::atom_program::AtomBindable"
                })
            {
                return true;
            }
            if matches!(cx.tcx.def_kind(caller), DefKind::AssocFn)
                && cx.tcx.item_name(caller).as_str() == "bind_workflow"
                && cx.tcx.opt_parent(caller).is_some_and(|parent| {
                    matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: true })
                        && definition_path(
                            cx.tcx,
                            cx.tcx
                                .impl_trait_ref(parent)
                                .instantiate_identity()
                                .skip_norm_wip()
                                .def_id,
                        ) == "internal::workflow::Bindable"
                })
                && associated_owner(cx.tcx, caller).is_some_and(|owner| {
                    kernel::binding_dispatch(&definition_path(cx.tcx, owner), &callee_path)
                })
            {
                return true;
            }
        }
        // The actual From<Effect<()>> for Release implementation delegates to
        // from_effect. Other trait implementations do not inherit this boundary.
        if callee.is_local()
            && caller.is_local()
            && cx.tcx.crate_name(callee.krate).as_str() == "effect"
            && cx.tcx.item_name(callee).as_str() == "from_effect"
            && target.is_some_and(|owner| definition_path(cx.tcx, owner) == "resource::Release")
            && matches!(cx.tcx.def_kind(caller), DefKind::AssocFn)
            && release_conversion(
                cx.tcx,
                caller,
                ty::GenericArgs::identity_for_item(cx.tcx, caller),
            )
        {
            return true;
        }
        // Root Web conversion revalidates only its actual native Security or
        // Result<Security, _> input. Other trait implementations gain no permission.
        if callee.is_local() && caller.is_local()
            && cx.tcx.crate_name(caller.krate).as_str() == "effect_server_web"
            && matches!(cx.tcx.def_kind(caller), DefKind::AssocFn)
            && cx.tcx.item_name(caller).as_str() == "into_security"
            && cx.tcx.item_name(callee).as_str() == "validate"
            && target.is_some_and(|owner| definition_path(cx.tcx, owner) == "security::Security")
            && cx.tcx.opt_parent(caller).is_some_and(|parent| {
                if !matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: true }) {
                    return false;
                }
                let implemented = cx.tcx.impl_trait_ref(parent).instantiate_identity().skip_norm_wip().def_id;
                if !implemented.is_local() || definition_path(cx.tcx, implemented) != "application::IntoWebSecurity" {
                    return false;
                }
                let self_ty = cx.tcx.type_of(parent).instantiate_identity().skip_norm_wip();
                canonical_type(cx.tcx, self_ty, "effect_server_web::security::Security")
                    || canonical_type(cx.tcx, self_ty, "core::result::Result")
                        && matches!(self_ty.kind(), ty::Adt(_, args) if args.types().next().is_some_and(|input| canonical_type(cx.tcx, input, "effect_server_web::security::Security")))
            })
        {
            return true;
        }
        // Reviewed Default bodies delegate only to their own empty
        // constructor. A user trait, another helper or another callee gains nothing.
        if callee.is_local()
            && caller.is_local()
            && registry::facade(cx.tcx.crate_name(caller.krate).as_str())
            && matches!(cx.tcx.def_kind(caller), DefKind::AssocFn)
            && cx.tcx.item_name(caller).as_str() == "default"
            && cx.tcx.item_name(callee).as_str() == "new"
            && associated_owner(cx.tcx, caller) == target
            && target.is_some_and(|owner| {
                kernel::default_constructor(
                    cx.tcx.crate_name(owner.krate).as_str(),
                    &definition_path(cx.tcx, owner),
                )
            })
            && cx.tcx.opt_parent(caller).is_some_and(|parent| {
                matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: true })
                    && cx.tcx.is_diagnostic_item(
                        sym::Default,
                        cx.tcx
                            .impl_trait_ref(parent)
                            .instantiate_identity()
                            .skip_norm_wip()
                            .def_id,
                    )
            })
        {
            return true;
        }
        // Native metadata derives copy their own values or initialize their
        // empty value. Actual std traits and the defining Self prevent helpers
        // or consumer traits from borrowing this representation permission.
        if callee.is_local()
            && caller.is_local()
            && ((cx.tcx.crate_name(callee.krate).as_str() == "effect_server"
                && definition_path(cx.tcx, callee) == "endpoint::Registration")
                || (cx.tcx.crate_name(callee.krate).as_str() == "effect_otel"
                    && definition_path(cx.tcx, callee) == "signals::SpanOptions"))
            && matches!(cx.tcx.def_kind(caller), DefKind::AssocFn)
            && associated_owner(cx.tcx, caller) == Some(callee)
            && cx.tcx.opt_parent(caller).is_some_and(|parent| {
                matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: true }) && {
                    let implementation = cx
                        .tcx
                        .impl_trait_ref(parent)
                        .instantiate_identity()
                        .skip_norm_wip();
                    let caller_name = cx.tcx.item_name(caller);
                    let name = caller_name.as_str();
                    (name == "clone"
                        && cx.tcx.is_diagnostic_item(sym::Clone, implementation.def_id))
                        || (name == "default"
                            && cx
                                .tcx
                                .is_diagnostic_item(sym::Default, implementation.def_id))
                }
            })
        {
            return true;
        }
        // Native HTTP completion closes its retained owner under the decorated
        // context and combines every Exit channel before final response settlement.
        if caller.is_local()
            && cx.tcx.crate_name(caller.krate).as_str() == "effect_server"
            && matches!(cx.tcx.def_kind(caller), DefKind::AssocFn)
            && cx.tcx.item_name(caller).as_str() == "complete_native_selection"
            && associated_owner(cx.tcx, caller)
                .is_some_and(|owner| definition_path(cx.tcx, owner) == "host::Boundary")
            && cx.tcx.opt_parent(caller).is_some_and(|parent| {
                matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
            })
            && definition_path(cx.tcx, callee) == "effect::__private::close_scope_owned"
        {
            return true;
        }
        // The native row resource binder acquires under the retained row scope.
        // Only the actual RowStep implementation for core ResourceBinding owns it.
        if caller.is_local()
            && cx.tcx.crate_name(caller.krate).as_str() == "effect_sql"
            && matches!(cx.tcx.def_kind(caller), DefKind::AssocFn)
            && cx.tcx.item_name(caller).as_str() == "evaluate"
            && associated_owner(cx.tcx, caller).is_some_and(|owner| {
                definition_path(cx.tcx, owner) == "effect::resource::ResourceBinding"
            })
            && cx.tcx.opt_parent(caller).is_some_and(|parent| {
                matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: true })
                    && definition_path(
                        cx.tcx,
                        cx.tcx
                            .impl_trait_ref(parent)
                            .instantiate_identity()
                            .skip_norm_wip()
                            .def_id,
                    ) == "row::RowStep"
            })
            && definition_path(cx.tcx, callee)
                == "effect::internal::macro_future::AsyncBindable::into_scoped_effect"
        {
            return true;
        }
        if callee.is_local()
            && caller.is_local()
            && cx.tcx.crate_name(callee.krate).as_str() == "effect"
            && matches!(cx.tcx.def_kind(caller), DefKind::AssocFn)
            && cx.tcx.opt_parent(caller).is_some_and(|parent| {
                matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
            })
            && associated_owner(cx.tcx, caller).is_some_and(|owner| {
                kernel::inherent_transition(
                    &definition_path(cx.tcx, owner),
                    cx.tcx.item_name(caller).as_str(),
                    target
                        .map(|owner| definition_path(cx.tcx, owner))
                        .as_deref(),
                    cx.tcx.item_name(callee).as_str(),
                    &target.map_or_else(
                        || definition_path(cx.tcx, callee),
                        |owner| {
                            format!(
                                "{}::{}",
                                definition_path(cx.tcx, owner),
                                cx.tcx.item_name(callee)
                            )
                        },
                    ),
                ) || kernel::machine(
                    &definition_path(cx.tcx, owner),
                    cx.tcx.item_name(caller).as_str(),
                    &definition_path(cx.tcx, caller),
                    target
                        .map(|owner| definition_path(cx.tcx, owner))
                        .as_deref(),
                    cx.tcx.item_name(callee).as_str(),
                )
            })
        {
            return true;
        }
        // A macro lowers to these exact native constructors/combinators. Calling
        // the same DSL from their implementation would recurse into this entry.
        // This does not admit other methods of the native type or user trait impls.
        if caller.is_local()
            && matches!(cx.tcx.def_kind(caller), DefKind::AssocFn)
            && cx.tcx.opt_parent(caller).is_some_and(|parent| {
                matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: false })
            })
        {
            if let Some(owner) = associated_owner(cx.tcx, caller) {
                if owner.is_local()
                    && registry::implementation_entry(
                        cx.tcx.crate_name(owner.krate).as_str(),
                        &definition_path(cx.tcx, owner),
                        cx.tcx.item_name(caller).as_str(),
                    )
                {
                    return true;
                }
            }
        }
        match cx.tcx.opt_parent(caller) {
            Some(parent) => caller = parent,
            None => return false,
        }
    }
}

impl<'tcx> LateLintPass<'tcx> for Dsl {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        // A helper's generic body must be checked with the actual caller types.
        // Its uninstantiated T::default cannot identify a native builder.
        let instantiated = if let ty::FnDef(id, args) = cx.typeck_results().expr_ty(expr).kind() {
            Some((*id, args.skip_binder()))
        } else if matches!(expr.kind, ExprKind::MethodCall(..)) {
            cx.typeck_results()
                .type_dependent_def_id(expr.hir_id)
                .map(|id| (id, cx.typeck_results().node_args(expr.hir_id)))
        } else {
            None
        };
        if let Some((id, args)) = instantiated
            && !generated(cx.tcx, expr.span)
            && !primitive(cx, expr, id)
            && let Some((constructor, owner)) =
                generic_constructor(cx.tcx, id, args, &mut Vec::new())
            && !primitive(cx, expr, constructor)
        {
            cx.emit_span_lint(
                MANUAL_DSL_CONSTRUCTION,
                expr.span,
                rustc_errors::DiagDecorator(|diag| {
                    diag.primary_message(format!(
                        "manual DSL construction through generic helper `{}`",
                        definition_path(cx.tcx, id)
                    ));
                    diag.help(format!(
                        "use {}; its instantiated body calls `{}`",
                        registry::intended_dsl(
                            Some(&owner),
                            cx.tcx.item_name(constructor).as_str()
                        ),
                        definition_path(cx.tcx, constructor)
                    ));
                }),
            );
            return;
        }
        let candidate = match expr.kind {
            ExprKind::Path(ref path) => match cx.qpath_res(path, expr.hir_id) {
                Res::Def(_, id) => Some((id, expr.span)),
                _ => None,
            },
            ExprKind::MethodCall(segment, _, _, _) => cx
                .typeck_results()
                .type_dependent_def_id(expr.hir_id)
                .map(|id| (id, segment.ident.span)),
            ExprKind::Struct(ref path, _, _) => match cx.qpath_res(path, expr.hir_id) {
                Res::Def(_, id) => Some((id, expr.span)),
                _ => None,
            },
            _ => None,
        };
        let Some((id, span)) = candidate else {
            return;
        };
        let mut owner = associated_owner(cx.tcx, id);
        // Associated trait functions (notably Default::default) retain the native
        // Self type in their instantiated function arguments, even through UFCS.
        if owner.is_none() && matches!(cx.tcx.def_kind(id), DefKind::AssocFn) {
            if let ty::FnDef(_, args) = cx.typeck_results().expr_ty(expr).kind() {
                if let Some(self_ty) = args.skip_binder().types().next() {
                    if let ty::Adt(adt, _) = self_ty.kind() {
                        owner = Some(adt.did());
                    }
                }
            }
        }
        let crate_name = cx.tcx.crate_name(owner.unwrap_or(id).krate);
        let name = cx.tcx.item_name(id);
        let owner_path = owner.map(|id| definition_path(cx.tcx, id));
        let typed_construction = if matches!(cx.tcx.def_kind(id), DefKind::AssocFn) {
            match cx.typeck_results().expr_ty(expr).kind() {
                ty::FnDef(_, args) => {
                    release_construction(cx.tcx, id, args.skip_binder())
                        || lazy_codec_construction(cx.tcx, id, args.skip_binder())
                }
                _ if matches!(expr.kind, ExprKind::MethodCall(..)) => {
                    release_construction(cx.tcx, id, cx.typeck_results().node_args(expr.hir_id))
                }
                _ => false,
            }
        } else {
            false
        };
        if !(typed_construction
            || registry::protected(
                crate_name.as_str(),
                owner_path.as_deref(),
                name.as_str(),
                cx.tcx.def_kind(id),
                &if id.is_local() {
                    format!(
                        "{}::{}",
                        cx.tcx.crate_name(id.krate),
                        definition_path(cx.tcx, id)
                    )
                } else {
                    definition_path(cx.tcx, id)
                },
            ))
            || generated(cx.tcx, span)
            || primitive(cx, expr, id)
        {
            return;
        }
        cx.emit_span_lint(MANUAL_DSL_CONSTRUCTION, span, rustc_errors::DiagDecorator(|diag| {
            diag.primary_message(format!("manual DSL construction through `{}`", definition_path(cx.tcx, id)));
            diag.help(format!("use {}; aliases, callable references and handwritten macro inputs are subject to the same requirement", registry::intended_dsl(owner_path.as_deref(), name.as_str())));
        }));
    }
}

/// Follow local generic function bodies using their concrete caller substitutions.
/// Existing descriptor transfers remain ordinary calls; only resolved protected
/// operations count, and recursive bodies are visited once along each call path.
fn generic_constructor<'tcx>(
    tcx: TyCtxt<'tcx>,
    id: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
    active: &mut Vec<DefId>,
) -> Option<(DefId, String)> {
    if !matches!(tcx.def_kind(id), DefKind::Fn | DefKind::AssocFn)
        || arguments.is_empty()
        || active.contains(&id)
    {
        return None;
    }
    if !arguments.types().flat_map(|ty| ty.walk()).any(|argument| {
        let ty::GenericArgKind::Type(ty) = argument.kind() else {
            return false;
        };
        let ty::Adt(adt, _) = ty.kind() else {
            return false;
        };
        registry::protected(
            tcx.crate_name(adt.did().krate).as_str(),
            Some(&definition_path(tcx, adt.did())),
            "default",
            DefKind::AssocFn,
            "",
        )
    }) || associated_owner(tcx, id).is_some_and(|owner| {
        registry::implementation_entry(
            tcx.crate_name(owner.krate).as_str(),
            &definition_path(tcx, owner),
            tcx.item_name(id).as_str(),
        )
    }) {
        return None;
    }
    active.push(id);
    let found = if let Some(local) = id.as_local() {
        if let Some(body) = tcx.hir_maybe_body_owned_by(local) {
            let mut visitor = GenericConstructors {
                tcx,
                caller: id,
                typeck: tcx.typeck(local),
                arguments,
                active,
                found: None,
            };
            visitor.visit_expr(body.value);
            visitor.found
        } else {
            None
        }
    } else if tcx.is_mir_available(id) {
        external_generic_constructor(tcx, id, arguments, active)
    } else {
        None
    };
    active.pop();
    found
}

/// Inspect exported generic bodies, including standard defaulting helpers.
/// Their MIR carries the same resolved operation identity and substituted Self
/// type as a local helper; ordinary empty containers construct no DSL value.
fn external_generic_constructor<'tcx>(
    tcx: TyCtxt<'tcx>,
    id: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
    active: &mut Vec<DefId>,
) -> Option<(DefId, String)> {
    let body = tcx.optimized_mir(id);
    for block in body.basic_blocks.iter() {
        let terminator = block.terminator();
        if generated(tcx, terminator.source_info.span) {
            continue;
        }
        let rustc_middle::mir::TerminatorKind::Call { func, .. } = &terminator.kind else {
            continue;
        };
        let Some((callee, args)) = func.const_fn_def() else {
            continue;
        };
        let args = ty::EarlyBinder::bind(tcx, args)
            .instantiate(tcx, arguments)
            .skip_norm_wip();
        if let Some(found) = resolved_generic_constructor(tcx, callee, args)
            .or_else(|| generic_constructor(tcx, callee, args, active))
        {
            return Some(found);
        }
    }
    None
}

/// Classify a generic operation only after its Self and output types are resolved.
fn resolved_generic_constructor<'tcx>(
    tcx: TyCtxt<'tcx>,
    id: DefId,
    args: ty::GenericArgsRef<'tcx>,
) -> Option<(DefId, String)> {
    let owner = associated_owner(tcx, id).or_else(|| {
        if !matches!(tcx.def_kind(id), DefKind::AssocFn) {
            return None;
        }
        match args.types().next()?.kind() {
            ty::Adt(adt, _) => Some(adt.did()),
            _ => None,
        }
    });
    let path = owner.map(|owner| definition_path(tcx, owner));
    if owner.is_some_and(|owner| {
        registry::protected(
            tcx.crate_name(owner.krate).as_str(),
            path.as_deref(),
            tcx.item_name(id).as_str(),
            tcx.def_kind(id),
            &definition_path(tcx, id),
        )
    }) || release_construction(tcx, id, args)
        || lazy_codec_construction(tcx, id, args)
    {
        Some((id, path.unwrap_or_default()))
    } else {
        None
    }
}

/// Resolve each generic callback/reference with the enclosing helper's type arguments.
struct GenericConstructors<'tcx, 'a> {
    /// Compiler context resolving the defining operation identities.
    tcx: TyCtxt<'tcx>,
    /// Actual body owner used to preserve reviewed native interpreter edges.
    caller: DefId,
    /// Type information for this helper's uninstantiated HIR.
    typeck: &'tcx ty::TypeckResults<'tcx>,
    /// Caller substitutions retaining the concrete Self type.
    arguments: ty::GenericArgsRef<'tcx>,
    /// Active bodies preventing recursive call paths from looping.
    active: &'a mut Vec<DefId>,
    /// First protected construction reached by this instantiated body.
    found: Option<(DefId, String)>,
}
impl<'tcx> Visitor<'tcx> for GenericConstructors<'tcx, '_> {
    /// Substitute each resolved operation before following its callback or helper.
    fn visit_expr(&mut self, expr: &'tcx Expr<'tcx>) {
        if self.found.is_some() {
            return;
        }
        let candidate = if let ty::FnDef(id, args) = self.typeck.expr_ty(expr).kind() {
            Some((*id, args.skip_binder()))
        } else if matches!(expr.kind, ExprKind::MethodCall(..)) {
            self.typeck
                .type_dependent_def_id(expr.hir_id)
                .map(|id| (id, self.typeck.node_args(expr.hir_id)))
        } else {
            None
        };
        if let Some((id, args)) = candidate {
            let args = ty::EarlyBinder::bind(self.tcx, args)
                .instantiate(self.tcx, self.arguments)
                .skip_norm_wip();
            if !generated(self.tcx, expr.span) {
                if let Some(found) = resolved_generic_constructor(self.tcx, id, args)
                    && !native_generic_transition(self.tcx, self.caller, id)
                {
                    self.found = Some(found);
                    return;
                }
                self.found = generic_constructor(self.tcx, id, args, self.active);
            }
        }
        intravisit::walk_expr(self, expr);
    }
}

/// Reuse the exact reviewed native body edge while following an instantiated helper.
/// This preserves interpreter transitions without granting their ordinary callers
/// permission to construct any descriptor themselves.
fn native_generic_transition(tcx: TyCtxt<'_>, caller: DefId, callee: DefId) -> bool {
    if !caller.is_local() || !callee.is_local() || tcx.crate_name(caller.krate).as_str() != "effect"
    {
        return false;
    }
    let target = associated_owner(tcx, callee).map(|owner| definition_path(tcx, owner));
    let callee_path = target.as_ref().map_or_else(
        || definition_path(tcx, callee),
        |owner| format!("{owner}::{}", tcx.item_name(callee)),
    );
    if matches!(tcx.def_kind(caller), DefKind::Fn) {
        return kernel::transition(&definition_path(tcx, caller), &callee_path);
    }
    if !matches!(tcx.def_kind(caller), DefKind::AssocFn)
        || !tcx
            .opt_parent(caller)
            .is_some_and(|parent| matches!(tcx.def_kind(parent), DefKind::Impl { of_trait: false }))
    {
        return false;
    }
    let Some(owner) = associated_owner(tcx, caller) else {
        return false;
    };
    let owner = definition_path(tcx, owner);
    kernel::inherent_transition(
        &owner,
        tcx.item_name(caller).as_str(),
        target.as_deref(),
        tcx.item_name(callee).as_str(),
        &callee_path,
    ) || kernel::machine(
        &owner,
        tcx.item_name(caller).as_str(),
        &definition_path(tcx, caller),
        target.as_deref(),
        tcx.item_name(callee).as_str(),
    )
}
struct Callbacks;
impl rustc_driver::Callbacks for Callbacks {
    fn config(&mut self, config: &mut rustc_interface::Config) {
        // rustc has expanded response files and parsed all option spellings.
        // ForceWarn and weaker lint caps override Forbid.
        if config
            .opts
            .lint_cap
            .is_some_and(|level| level != Level::Forbid)
            || config.opts.lint_opts.iter().any(|(name, level)| {
                *level == Level::ForceWarn
                    && matches!(name.as_str(), "manual_dsl_construction" | "warnings")
            })
        {
            rustc_session::EarlyDiagCtxt::new(config.opts.error_format)
                .early_fatal("compiler flags cannot weaken manual_dsl_construction");
        }
        config
            .opts
            .lint_opts
            .push(("manual_dsl_construction".into(), Level::Forbid));
        config.register_lints = Some(Box::new(|_, store| {
            store.register_lints(&[MANUAL_DSL_CONSTRUCTION]);
            store.register_late_lint_pass(Box::new(|_| Box::new(Dsl)));
        }));
    }
    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        _: TyCtxt<'tcx>,
    ) -> rustc_driver::Compilation {
        rustc_driver::Compilation::Stop
    }
}
fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    rustc_driver::run_compiler(&args, &mut Callbacks);
}
