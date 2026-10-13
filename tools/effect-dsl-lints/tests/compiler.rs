//! Regressions run the mandatory native Cargo wrapper against actual Effect metadata.
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
/// A successful mandatory check validates metadata once before downstream probes.
static METADATA: OnceLock<Result<PathBuf, String>> = OnceLock::new();

/// Select the metadata built by the required native workspace check.
fn metadata(root: &Path) -> io::Result<PathBuf> {
    METADATA
        .get_or_init(|| validated_metadata(root).map_err(|error| error.to_string()))
        .clone()
        .map_err(io::Error::other)
}

/// Require current source to pass the configured compiler lint before selecting its library artifact.
fn validated_metadata(root: &Path) -> io::Result<PathBuf> {
    let check = Command::new("cargo")
        .args([
            "+1.99.0",
            "check",
            "--locked",
            "-p",
            "effect",
            "--lib",
            "-j2",
            "--message-format=json",
        ])
        .current_dir(root)
        .output()?;
    if !check.status.success() {
        let diagnostics = String::from_utf8_lossy(&check.stdout)
            .lines()
            .filter(|line| line.contains("\"reason\":\"compiler-message\""))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(io::Error::other(format!(
            "mandatory Effect metadata check failed: {}\n{diagnostics}",
            String::from_utf8_lossy(&check.stderr)
        )));
    }
    // Cargo reports the exact artifact even when fresh. Never choose another
    // invocation's metadata by directory age or guessed feature configuration.
    let output = String::from_utf8(check.stdout).map_err(io::Error::other)?;
    let mut artifacts = output
        .lines()
        .filter(|line| {
            line.contains("\"reason\":\"compiler-artifact\"")
                && line.contains("\"name\":\"effect\"")
                && line.contains("\"kind\":[\"lib\"]")
        })
        .filter_map(|line| line.split_once("\"filenames\":[").map(|(_, names)| names))
        .filter_map(|names| names.split_once(']').map(|(names, _)| names))
        .flat_map(|names| names.split('"'))
        .filter(|name| name.ends_with(".rmeta"))
        .map(PathBuf::from);
    let artifact = artifacts
        .next()
        .ok_or_else(|| io::Error::other("Cargo did not report Effect library metadata"))?;
    if artifacts.next().is_some() || !artifact.metadata()?.is_file() {
        return Err(io::Error::other(
            "Cargo reported ambiguous or missing Effect library metadata",
        ));
    }
    Ok(artifact)
}

/// Analyze one real downstream source through the configured native wrapper.
fn compile(body: &str) -> io::Result<Output> {
    compile_crate(body, "dsl_lint_probe")
}

/// Native caller probes exercise the local DefId boundary without linking a runtime.
fn compile_crate(body: &str, crate_name: &str) -> io::Result<Output> {
    compile_crate_flags(body, crate_name, &[])
}

/// Probe explicit compiler options without changing workspace configuration.
fn compile_crate_flags(body: &str, crate_name: &str, flags: &[&str]) -> io::Result<Output> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("missing workspace root"))?;
    let directory = env::temp_dir().join(format!(
        "effect-dsl-regression-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory)?;
    let source = directory.join("probe.rs");
    fs::write(&source, format!("#![allow(dead_code)]\n{body}\n"))?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_effect-dsl-lint"));
    command
        .arg("rustc")
        .arg("--edition=2024")
        .arg(format!("--crate-name={crate_name}"))
        .arg(&source)
        .arg("--emit=metadata")
        .arg("-o")
        .arg(directory.join("probe.rmeta"))
        .arg("-L")
        .arg(format!(
            "dependency={}",
            root.join("target/debug/deps").display()
        ))
        .env(
            "CARGO_MANIFEST_DIR",
            root.join("crates/effect/tests/fixtures/runtime_neutral"),
        )
        .env("CARGO_PKG_NAME", "effect-neutral-consumer");
    if crate_name == "effect" {
        command.arg("--cfg=test");
    } else {
        command
            .arg("--extern")
            .arg(format!("fx={}", metadata(root)?.display()));
    }
    let result = command.args(flags).output();
    fs::remove_file(source)?;
    let metadata = directory.join("probe.rmeta");
    if metadata.exists() {
        fs::remove_file(metadata)?;
    }
    fs::remove_dir(directory)?;
    result
}

/// A rejected probe must fail specifically on the compiler construction lint.
fn reject(body: &str) -> io::Result<()> {
    let result = compile(body)?;
    let diagnostics = String::from_utf8_lossy(&result.stderr);
    assert!(
        !result.status.success(),
        "manual construction unexpectedly passed"
    );
    assert!(
        diagnostics.contains("manual DSL construction through"),
        "wrong failure: {diagnostics}"
    );
    Ok(())
}

#[test]
fn generated_blocks_and_native_callbacks_pass() -> io::Result<()> {
    let result = compile(
        r#"
use fx::{effect, effect_future, effect_ref, effect_resource, effect_sync, effect_using, workflow};
fn main() {
        let _: fx::Effect<u8, ()> = effect! {
            let value = effect_sync! { body { 1u8 }, }?;
            value + 1
        };
        let _: fx::Effect<String, ()> = effect_sync! { once; body { String::from("owned") }, };
        let _: fx::Effect<Vec<u8>, ()> = effect! {
            vec! { 1u8, 2u8 }
        };
        let _ = fx::AtomicRef::new(1u8);
        let _ = fx::SynchronizedRef::new(String::from("ordinary state"));
        let state = fx::SynchronizedRef::new(String::from("native transition"));
        let _: fx::Effect<usize, ()> = effect_ref! {
            synchronized_pure;
            reference: state,
            modify: |old| (old.len(), old.as_ref().clone()),
            on_error: |_| (),
        };
        let _: fx::ResourceBinding<String, ()> = effect_using! {
            binding;
            acquire: effect_sync! { body: || String::from("owned resource") },
        };
        let _: fx::Release<()> = effect_resource! {
            release_sync;
            body: move || drop(String::from("native disposal")),
        };
        let _: fx::Effect<(), ()> = effect_resource! {
            acquire: effect_sync! { body: || (), },
            release: |(), _| effect_sync! { once; body: || (), },
            body: |_| effect_sync! { body: || (), },
        };
        let _: fx::workflow::Durable<String, String, ()> = workflow! {
            result; value: Ok(String::from("owned")),
        };
        let _: fx::workflow::Durable<String, String, ()> = workflow! {
            result; value: Err(String::from("typed failure")),
        };
        let native: fx::Effect<(), &'static str> = effect_future! {
            exit_with_context;
            body: |_, context| async move {
                let _ = context.cancellation();
                fx::Exit::Fail("primary")
            },
        };
        let _: fx::Effect<(), &'static str> = effect_resource! {
            ensuring;
            program: native,
            release: |primary| {
                assert!(matches!(primary, fx::PrimaryOutcomeRef::Failed(&"primary")));
                effect_sync! { body { () }, }
            },
        };
    }"#,
    )?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

#[test]
fn operator_callbacks_infer_collections_tuples_and_borrowed_environments() -> io::Result<()> {
    let result = compile(
        r#"
        
use fx::{effect_map, effect_map_error, effect_provide_some, effect_sync, effect_try_sync};
struct Outer;
        fn main() {
            let collection: fx::Effect<Vec<String>, ()> = effect_sync! { body { Vec::new() }, };
            let _: fx::Effect<usize, ()> = effect_map! {
                map: |values| values.len(),
                program: collection,
            };
            let pair: fx::Effect<(Vec<u8>, Vec<u8>), ()> = effect_sync! { body { (vec![], vec![]) }, };
            let _: fx::Effect<Vec<u8>, ()> = effect_map! {
                program: pair,
                map: |(mut first, rest)| { first.extend(rest); first },
            };
            let failure: fx::Effect<(), Vec<u8>> = effect_try_sync! { body { Err(vec![]) }, };
            let _: fx::Effect<(), usize> = effect_map_error! {
                program: failure,
                map: |error| error.len(),
            };
            let program: fx::Effect<u8, ()> = effect_sync! { body { 1 }, };
            let _: fx::Effect<u8, (), Outer> = effect_provide_some! {
                program: program,
                project: |_| (),
            };
        }
    "#,
    )?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

#[test]
fn aliases_reexports_ufcs_and_callable_references_fail() -> io::Result<()> {
    for body in [
        "fn main() { let _: fx::Effect<u8, ()> = fx::Effect::sync(|| 1); }",
        "use fx::Effect as Renamed; fn main() { let _: Renamed<u8, ()> = Renamed::sync(|| 1); }",
        "mod facade { pub use fx::Effect as Program; } fn main() { let _: facade::Program<u8, ()> = facade::Program::sync(|| 1); }",
        "fn main() { let _: fx::Effect<u8, ()> = <fx::Effect<u8, ()>>::sync(|| 1); }",
        "fn main() { let make = fx::Effect::<u8, ()>::sync; let _ = make(|| 1); }",
    ] {
        reject(body)?;
    }
    Ok(())
}

#[test]
fn method_chains_and_handwritten_macro_inputs_fail() -> io::Result<()> {
    reject(
        "fn main() { let _: fx::workflow::Durable<String, String, ()> = fx::workflow::Durable::from_result_once(Ok(String::from(\"owned\"))); }",
    )?;
    reject(
        "\nuse fx::{effect, effect_sync};\nfn main() { let _: fx::Effect<u8, ()> = effect! { bind; program: effect_sync! { body { () }, }, body: |_| fx::Effect::sync(|| 1u8), }; }",
    )?;
    reject(
        "\nuse fx::effect_sync;\nfn main() { let program: fx::Effect<u8, ()> = effect_sync! { body { 1 }, }; let _ = program.map(|value| value + 1); }",
    )?;
    for conversion in [
        "fx::Release::from(program)",
        "<fx::Release<()> as From<fx::Effect<(), ()>>>::from(program)",
        "program.into()",
        "Into::<fx::Release<()>>::into(program)",
    ] {
        reject(&format!(
            "\nuse fx::effect_sync;\nfn main() {{ let program: fx::Effect<(), ()> = effect_sync! {{ body: || (), }}; let _: fx::Release<()> = {conversion}; }}"
        ))?;
    }
    reject(
        "\nuse fx::effect_sync;\nfn main() { let _: fx::Effect<u8, ()> = effect_sync! { body { let hidden: fx::Effect<u8, ()> = fx::Effect::sync(|| 1); drop(hidden); 2 }, }; }",
    )?;
    reject(
        "\nuse fx::{effect};\nfn main() { let _: fx::Effect<u8, ()> = effect! { let hidden: fx::Effect<u8, ()> = fx::Effect::sync(|| 1); hidden? }; }",
    )?;
    Ok(())
}

#[test]
fn consumer_trait_impl_cannot_borrow_the_kernel_exemption() -> io::Result<()> {
    reject(
        "trait Recipe { fn recipe() -> Self; } impl Recipe for fx::Effect<u8, ()> { fn recipe() -> Self { fx::Effect::sync(|| 1) } } fn main() {}",
    )
}

#[test]
/// Instantiating a generic Default helper must retain constructor enforcement.
fn generic_default_helper_cannot_hide_a_native_builder() -> io::Result<()> {
    let ordinary = compile(
        "fn make<T: Default>() -> T { T::default() } fn main() { let _: String = make(); let _: Vec<fx::RecordBuilder<(), fx::RecordNil>> = make(); }",
    )?;
    assert!(
        ordinary.status.success(),
        "{}",
        String::from_utf8_lossy(&ordinary.stderr)
    );
    for body in [
        "fn make<T: Default>() -> T { T::default() } fn main() { let _: fx::RecordBuilder<(), fx::RecordNil> = make(); }",
        "fn make<T: Default>() -> T { T::default() } fn relay<T: Default>() -> T { make() } fn main() { let _: fx::RecordBuilder<(), fx::RecordNil> = relay(); }",
        "struct Maker; impl Maker { fn make<T: Default>(&self) -> T { T::default() } } fn main() { let _: fx::RecordBuilder<(), fx::RecordNil> = Maker.make(); }",
        "fn make<T: Default>() -> T { T::default() } fn main() { let constructor = make::<fx::RecordBuilder<(), fx::RecordNil>>; let _ = constructor(); }",
        "fn main() { let _: fx::RecordBuilder<(), fx::RecordNil> = Option::None.unwrap_or_default(); }",
        "fn take(existing: &mut fx::RecordBuilder<(), fx::RecordNil>) { let _ = std::mem::take(existing); } fn main() {}",
    ] {
        reject(body)?;
    }
    Ok(())
}

#[test]
fn lazy_codec_uses_the_dsl_without_banning_ordinary_lazy_values() -> io::Result<()> {
    let positive = compile(
        "\nuse fx::{codec};\nfn main() { let _ = codec! { lazy; lazy { fx::json_string() }, }; let _ = std::sync::LazyLock::new(|| String::from(\"ordinary\")); }",
    )?;
    assert!(
        positive.status.success(),
        "{}",
        String::from_utf8_lossy(&positive.stderr)
    );
    for constructor in [
        "std::sync::LazyLock::new(|| fx::json_string())",
        "{ use std::sync::LazyLock as Lazy; Lazy::new(|| fx::json_string()) }",
        "{ let make = std::sync::LazyLock::new; make(|| fx::json_string()) }",
        "<std::sync::LazyLock<fx::PureCodec<String, fx::JsonValue>, fn() -> fx::PureCodec<String, fx::JsonValue>>>::new(fx::json_string)",
    ] {
        reject(&format!("fn main() {{ let _ = {constructor}; }}"))?;
    }
    reject(
        "\nuse fx::{codec};\nfn main() { let _ = codec! { lazy; lazy { let _ = std::sync::LazyLock::new(|| fx::json_string()); fx::json_string() }, }; }",
    )
}

#[test]
fn codec_callbacks_infer_borrowed_values_and_owned_contexts() -> io::Result<()> {
    let result = compile(
        r#"
        
use fx::{codec, effect_sync};
fn decode(text: &String) -> Result<usize, fx::SchemaIssue> { Ok(text.len()) }
        fn encode(length: &usize) -> Result<String, fx::SchemaIssue> { Ok(length.to_string()) }
        fn contextual_decode(text: String, context: Option<fx::ParseContext>) -> fx::Effect<usize, fx::SchemaIssue> {
            let _ = context.as_ref();
            effect_sync! { once; body: move || text.len(), }
        }
        fn contextual_encode(length: usize, context: Option<fx::ParseContext>) -> fx::Effect<String, fx::SchemaIssue> {
            let _ = context.as_ref();
            effect_sync! { once; body: move || length.to_string(), }
        }
        fn main() {
            let _: fx::PureCodec<usize, String> = codec! {
                pure;
                encode: |length| Ok(length.to_string()),
                decode: |text| Ok(text.len()),
            };
            let _: fx::Codec<usize, String, (), ()> = codec! {
                contextual;
                decode: |text, context| {
                    let _ = context.as_ref();
                    effect_sync! { once; body: move || text.len(), }
                },
                encode: |length, context| {
                    let _ = context.as_ref();
                    effect_sync! { once; body: move || length.to_string(), }
                },
                ast: fx::SchemaAst::opaque(),
            };
            let _: fx::PureCodec<usize, String> = codec! { pure; encode: encode, decode: decode, };
            let _: fx::Codec<usize, String, (), ()> = codec! {
                contextual; decode: contextual_decode, encode: contextual_encode, ast: fx::SchemaAst::opaque(),
            };
        }
    "#,
    )?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

#[test]
fn same_named_procedural_macro_cannot_claim_native_export_identity() -> io::Result<()> {
    let directory =
        env::temp_dir().join(format!("effect-dsl-macro-identity-{}", std::process::id()));
    fs::create_dir(&directory)?;
    let source = directory.join("macro.rs");
    fs::write(
        &source,
        "extern crate proc_macro; #[proc_macro] pub fn effect_sync(_: proc_macro::TokenStream) -> proc_macro::TokenStream { \"crate::program::Effect::new()\".parse().expect(\"fixture tokens\") }",
    )?;
    let library = directory.join(format!("libeffect_macros{}", env::consts::DLL_SUFFIX));
    let build = Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-name=effect_macros",
            "--crate-type=proc-macro",
        ])
        .arg(&source)
        .arg("-o")
        .arg(&library)
        .output()?;
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let dependency = format!("spoof={}", library.display());
    let result = compile_crate_flags(
        "\nuse spoof::{effect_sync};\nmod program { pub struct Effect; impl Effect { pub fn new() -> Self { Self } } } fn main() { let _ = effect_sync!(); }",
        "effect",
        &["--extern", &dependency],
    );
    fs::remove_file(source)?;
    fs::remove_file(library)?;
    fs::remove_dir(directory)?;
    let result = result?;
    assert!(
        !result.status.success(),
        "same-named macro bypassed facade export identity: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("manual DSL construction through"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

#[test]
fn native_machine_permission_requires_the_reviewed_inherent_caller() -> io::Result<()> {
    // These minimal local declarations exercise compiler identities and caller
    // ownership only. Downstream DSL tests above use actual Effect metadata.
    const NATIVE: &str = r#"
        mod program { pub struct Effect; impl Effect { pub fn new() -> Self { Self } } }
        mod cache { pub struct Cache; impl Cache {
            pub fn get_observed() -> crate::program::Effect { crate::program::Effect::new() }
            pub fn make_with_options() -> crate::program::Effect { crate::program::Effect::new() }
        } }
        fn main() {}
    "#;
    let positive = compile_crate(NATIVE, "effect")?;
    assert!(
        positive.status.success(),
        "{}",
        String::from_utf8_lossy(&positive.stderr)
    );
    let ordinary_names = compile_crate(
        &format!(
            "{NATIVE} #[cfg(test)] mod verification {{ pub struct Cache; impl Cache {{ pub fn new() -> Self {{ Self }} }} pub fn durable_activity() {{}} pub fn construct() {{ let _ = Cache::new(); durable_activity(); }} }}"
        ),
        "effect",
    )?;
    assert!(
        ordinary_names.status.success(),
        "{}",
        String::from_utf8_lossy(&ordinary_names.stderr)
    );
    for caller in [
        "fn ordinary() { let _ = program::Effect::new(); }",
        "#[cfg(test)] mod verification { fn helper() { let _ = crate::program::Effect::new(); } }",
        "trait Recipe { fn get_observed(); } impl Recipe for cache::Cache { fn get_observed() { let _ = program::Effect::new(); } }",
        "impl cache::Cache { fn ordinary() { let _ = program::Effect::new(); } }",
        "impl cache::Cache { fn from_helper() { let _ = program::Effect::new(); } }",
        "#[cfg(test)] mod verification { struct Cache; impl Cache { fn make_with_options() { let _ = crate::program::Effect::new(); } } }",
        "#[cfg(test)] mod verification { struct Effect; impl Effect { fn sync() { let _ = crate::program::Effect::new(); } } }",
    ] {
        let negative = compile_crate(&format!("{NATIVE}\n{caller}"), "effect")?;
        assert!(
            !negative.status.success(),
            "native helper unexpectedly passed: {caller}"
        );
        assert!(
            String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through"),
            "{}",
            String::from_utf8_lossy(&negative.stderr)
        );
    }
    const RELEASE: &str = r#"
        mod program { pub struct Effect<A = ()>(pub std::marker::PhantomData<A>); }
        mod resource {
            pub struct Release;
            impl Release { pub fn from_effect(_: crate::program::Effect) -> Self { Self } }
            impl From<crate::program::Effect> for Release {
                fn from(program: crate::program::Effect) -> Self { Self::from_effect(program) }
            }
        }
        mod __private {
            pub fn primary_release<C: Into<crate::resource::Release>>(cleanup: C) -> crate::resource::Release { cleanup.into() }
        }
        fn transfer(cleanup: resource::Release) -> resource::Release { cleanup.into() }
        fn main() { let _: String = "ordinary".into(); let _ = String::from("ordinary"); }
    "#;
    let positive = compile_crate(RELEASE, "effect")?;
    assert!(
        positive.status.success(),
        "{}",
        String::from_utf8_lossy(&positive.stderr)
    );
    for conversion in [
        "resource::Release::from(program)",
        "<resource::Release as From<program::Effect>>::from(program)",
        "program.into()",
        "Into::<resource::Release>::into(program)",
        "{ let convert = <resource::Release as From<program::Effect>>::from; convert(program) }",
    ] {
        let negative = compile_crate(
            &format!(
                "{RELEASE} fn manual(program: program::Effect) {{ let _: resource::Release = {conversion}; }}"
            ),
            "effect",
        )?;
        assert!(
            !negative.status.success(),
            "Release conversion unexpectedly passed: {conversion}"
        );
        assert!(
            String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through"),
            "{}",
            String::from_utf8_lossy(&negative.stderr)
        );
    }
    let negative = compile_crate(
        &format!(
            "{RELEASE} fn manual<C: Into<resource::Release>>(cleanup: C) -> resource::Release {{ cleanup.into() }}"
        ),
        "effect",
    )?;
    assert!(
        !negative.status.success(),
        "generic conversion hid Release construction"
    );
    assert!(
        String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through"),
        "{}",
        String::from_utf8_lossy(&negative.stderr)
    );
    const BRIDGE: &str = r#"
        mod program { pub struct Effect; impl Effect {
            pub fn new() -> Self { Self }
            pub fn sync() -> Self { Self }
        } }
        mod internal { pub mod workflow {
            pub fn reusable() -> crate::program::Effect { crate::program::Effect::new() }
        } }
        fn main() {}
    "#;
    let positive = compile_crate(BRIDGE, "effect")?;
    assert!(
        positive.status.success(),
        "{}",
        String::from_utf8_lossy(&positive.stderr)
    );
    let negative = compile_crate(&BRIDGE.replace("Effect::new()", "Effect::sync()"), "effect")?;
    assert!(
        !negative.status.success(),
        "bridge admitted an unreviewed callee"
    );
    assert!(String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through"));

    const WORKFLOW: &str = r#"
        mod internal { pub mod workflow {
            pub struct Workflow;
            impl Workflow {
                pub fn new() -> Self { Self }
                pub fn from_exit() -> Self { Self::new() }
            }
            pub fn success() -> Workflow { Workflow::from_exit() }
        } }
        fn main() {}
    "#;
    let positive = compile_crate(WORKFLOW, "effect")?;
    assert!(
        positive.status.success(),
        "{}",
        String::from_utf8_lossy(&positive.stderr)
    );
    for construction in [
        "internal::workflow::Workflow::new()",
        "internal::workflow::Workflow::from_exit()",
        "{ use internal::workflow::Workflow as W; W::from_exit() }",
        "{ let build = internal::workflow::Workflow::from_exit; build() }",
        "{ macro_rules! input { ($value:expr) => { $value }; } input!(internal::workflow::Workflow::from_exit()) }",
    ] {
        let negative = compile_crate(
            &format!("{WORKFLOW} fn manual() {{ let _ = {construction}; }}"),
            "effect",
        )?;
        assert!(
            !negative.status.success(),
            "Workflow bypass passed: {construction}"
        );
        assert!(
            String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through"),
            "{}",
            String::from_utf8_lossy(&negative.stderr)
        );
    }

    const FACTORY: &str = r#"
        mod reactivity { pub mod atom_program {
            pub struct AtomProgram;
            impl AtomProgram {
                pub fn new() -> Self { Self }
                pub fn from_effect() -> Self { Self::new() }
            }
        } }
        fn main() {}
    "#;
    let positive = compile_crate(FACTORY, "effect")?;
    assert!(
        positive.status.success(),
        "{}",
        String::from_utf8_lossy(&positive.stderr)
    );
    for caller in [
        "fn manual() { let _ = reactivity::atom_program::AtomProgram::new(); }",
        "impl reactivity::atom_program::AtomProgram { fn ordinary() -> Self { Self::new() } }",
        "trait UserFactory { fn from_effect(); } impl UserFactory for reactivity::atom_program::AtomProgram { fn from_effect() { let _ = Self::new(); } }",
        "#[cfg(test)] mod verification { fn manual() { let _ = crate::reactivity::atom_program::AtomProgram::new(); } }",
    ] {
        let negative = compile_crate(&format!("{FACTORY}\n{caller}"), "effect")?;
        assert!(
            !negative.status.success(),
            "native factory caller passed: {caller}"
        );
        assert!(
            String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through")
        );
    }

    const OWNED_FACTORY: &str = r#"
        mod program {
            pub struct Effect;
            impl Effect { pub fn flat_map(self, callback: impl Fn() -> ()) -> Self { callback(); self } }
        }
        mod persistence { pub mod result_store {
            pub fn compare_exchange() -> crate::program::Effect {
                let input = std::sync::Mutex::new(Some(String::from("owned")));
                crate::program::Effect.flat_map(move || { let _ = input.lock().map(|mut slot| slot.take()); })
            }
        } }
        fn main() {}
    "#;
    let negative = compile_crate(OWNED_FACTORY, "effect")?;
    assert!(
        !negative.status.success(),
        "an owned slot must not exempt ordinary composition"
    );
    assert!(String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through"));
    for source in [
        OWNED_FACTORY.replace("compare_exchange", "ordinary"),
        OWNED_FACTORY
            .replace(
                "let input = std::sync::Mutex::new(Some(String::from(\"owned\")));",
                "let input = String::from(\"owned\");",
            )
            .replace(
                "let _ = input.lock().map(|mut slot| slot.take());",
                "let _ = input.len();",
            ),
        OWNED_FACTORY.replace("mod result_store", "mod tests"),
        OWNED_FACTORY
            .replace(
                "std::sync::Mutex::new(Some(String::from(\"owned\")))",
                "std::sync::Mutex::new(String::from(\"owned\"))",
            )
            .replace("slot.take()", "slot.len()"),
    ] {
        let negative = compile_crate(&source, "effect")?;
        assert!(
            !negative.status.success(),
            "owned factory permission escaped its native slot boundary"
        );
        assert!(
            String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through"),
            "{}",
            String::from_utf8_lossy(&negative.stderr)
        );
    }

    const DISPATCH: &str = r#"
        mod program { pub struct Effect; }
        mod internal { pub mod workflow {
            pub trait Bindable { fn bind_workflow(); }
            pub fn bind_effect() {}
        } }
        impl internal::workflow::Bindable for program::Effect {
            fn bind_workflow() { internal::workflow::bind_effect(); }
        }
        fn main() {}
    "#;
    let positive = compile_crate(DISPATCH, "effect")?;
    assert!(
        positive.status.success(),
        "{}",
        String::from_utf8_lossy(&positive.stderr)
    );
    let negative = compile_crate(&DISPATCH.replace("Bindable", "UserBindable"), "effect")?;
    assert!(
        !negative.status.success(),
        "unrelated trait inherited sealed dispatch"
    );
    assert!(String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through"));

    for fixture in [
        "mod workflow { pub mod durable { pub fn activity() {} } } use workflow::durable::activity as durable_activity; fn manual() { let make = durable_activity; make(); } fn main() {}",
        "mod internal { pub mod scoped_body { pub struct ScopedBody; impl ScopedBody { pub fn from_body() -> Self { Self } } } } fn manual() { let _ = internal::scoped_body::ScopedBody::from_body(); } fn main() {}",
        "mod schema { pub mod parse_context { pub struct ConfiguredCodec; impl ConfiguredCodec { pub fn parse(&self) {} } } } fn manual(codec: schema::parse_context::ConfiguredCodec) { codec.parse(); } fn main() {}",
    ] {
        let negative = compile_crate(fixture, "effect")?;
        assert!(
            !negative.status.success(),
            "resolved construction unexpectedly passed: {fixture}"
        );
        assert!(
            String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through"),
            "{}",
            String::from_utf8_lossy(&negative.stderr)
        );
    }
    const EMPTY_BUILDER: &str = r#"
        mod schema { pub mod schema_builder {
            pub struct RecordBuilder;
            impl RecordBuilder { pub fn new() -> Self { Self } }
            impl Default for RecordBuilder { fn default() -> Self { Self::new() } }
        } }
        fn main() {}
    "#;
    let positive = compile_crate(EMPTY_BUILDER, "effect")?;
    assert!(
        positive.status.success(),
        "{}",
        String::from_utf8_lossy(&positive.stderr)
    );
    for extra in [
        "trait Recipe { fn default() -> Self; } impl Recipe for schema::schema_builder::RecordBuilder { fn default() -> Self { Self::new() } }",
        "fn manual() { let _ = schema::schema_builder::RecordBuilder::default(); }",
    ] {
        let negative = compile_crate(&format!("{EMPTY_BUILDER} {extra}"), "effect")?;
        assert!(
            !negative.status.success(),
            "unreviewed builder route passed"
        );
        assert!(
            String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through"),
            "{}",
            String::from_utf8_lossy(&negative.stderr)
        );
    }
    const LAZY_CODEC: &str = "mod schema { pub mod codec { pub struct Codec; pub struct PureCodec; } } fn main() { let _ = std::sync::LazyLock::new(|| String::from(\"ordinary\")); }";
    let positive = compile_crate(LAZY_CODEC, "effect")?;
    assert!(
        positive.status.success(),
        "{}",
        String::from_utf8_lossy(&positive.stderr)
    );
    for constructor in [
        "std::sync::LazyLock::new(|| schema::codec::Codec)",
        "{ use std::sync::LazyLock as Lazy; Lazy::new(|| schema::codec::PureCodec) }",
        "{ let make = std::sync::LazyLock::new; make(|| schema::codec::PureCodec) }",
        "<std::sync::LazyLock<schema::codec::Codec, fn() -> schema::codec::Codec>>::new(|| schema::codec::Codec)",
    ] {
        let negative = compile_crate(
            &format!("{LAZY_CODEC} fn manual() {{ let _ = {constructor}; }}"),
            "effect",
        )?;
        assert!(
            !negative.status.success(),
            "lazy codec bypass passed: {constructor}: {}",
            String::from_utf8_lossy(&negative.stderr)
        );
        assert!(
            String::from_utf8_lossy(&negative.stderr).contains("manual DSL construction through"),
            "{}",
            String::from_utf8_lossy(&negative.stderr)
        );
    }
    Ok(())
}

#[test]
fn command_line_options_cannot_weaken_construction_enforcement() -> io::Result<()> {
    let body = "mod program { pub struct Effect; impl Effect { pub fn new() -> Self { Self } } } fn main() { let _ = program::Effect::new(); }";
    for flags in [
        vec!["--allow=manual_dsl_construction"],
        vec!["--cap-lints=allow"],
        vec!["--force-warn=manual_dsl_construction"],
    ] {
        let output = compile_crate_flags(body, "effect", &flags)?;
        assert!(
            !output.status.success(),
            "lint weakening unexpectedly passed: {flags:?}"
        );
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostics.contains("manual DSL construction through")
                || diagnostics.contains("compiler flags cannot weaken manual_dsl_construction"),
            "{diagnostics}"
        );
        let response = env::temp_dir().join(format!(
            "effect-dsl-flags-{}-{}.txt",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&response, flags.join("\n"))?;
        let argument = format!("@{}", response.display());
        let output = compile_crate_flags(body, "effect", &[&argument]);
        fs::remove_file(response)?;
        let output = output?;
        assert!(
            !output.status.success(),
            "response-file weakening unexpectedly passed: {flags:?}"
        );
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostics.contains("manual DSL construction through")
                || diagnostics.contains("compiler flags cannot weaken manual_dsl_construction"),
            "{diagnostics}"
        );
    }
    let response = env::temp_dir().join(format!(
        "effect-dsl-legitimate-flags-{}.txt",
        std::process::id()
    ));
    fs::write(&response, "--allow=dead_code\n")?;
    let argument = format!("@{}", response.display());
    let output = compile_crate_flags("fn main() {}", "effect", &[&argument]);
    fs::remove_file(response)?;
    let output = output?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

const SCHEMA: &str = r#"
struct Record { value: String }
fn policy() -> fx::ParsePolicy {
    let limit = std::num::NonZeroUsize::MIN.saturating_add(32);
    fx::ParsePolicy { limits: fx::ParseLimits { bytes: limit, depth: limit, nodes: limit, collection: limit, issues: limit },
        errors: fx::SchemaErrorMode::First, unknown_fields: fx::UnknownFields::Reject, duplicate_fields: fx::DuplicateFields::Reject }
}
"#;

#[test]
fn generated_schema_passes_but_its_handwritten_builder_input_fails() -> io::Result<()> {
    let positive = format!(
        "use fx::schema; {SCHEMA} fn main() {{ let _ = schema! {{ struct Record {{ value: fx::json_string() }} policy: policy() }}; }}"
    );
    let result = compile(&positive)?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    reject(&format!(
        "use fx::schema; {SCHEMA} fn main() {{ let _ = schema! {{ struct Record {{ value: {{ let _ = fx::RecordBuilder::<Record>::new(); fx::json_string() }} }} policy: policy() }}; }}"
    ))
}

#[test]
fn allow_cannot_weaken_the_required_lint() -> io::Result<()> {
    let result = compile(
        "#![allow(manual_dsl_construction)] fn main() { let _: fx::Effect<u8, ()> = fx::Effect::sync(|| 1); }",
    )?;
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("incompatible with previous forbid"));
    Ok(())
}

/// Policy edits must invalidate a Cargo unit whose Rust source remains unchanged.
#[test]
fn changed_policy_rechecks_a_cached_consumer() -> io::Result<()> {
    let original = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("missing workspace root"))?;
    let root = env::temp_dir().join(format!(
        "effect-dsl-cache-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let tool = root.join("tools/effect-dsl-lints");
    let macros = root.join("crates/effect_macros/src");
    let consumer = root.join("consumer");
    fs::create_dir_all(&tool)?;
    fs::create_dir_all(&macros)?;
    fs::create_dir_all(consumer.join("src"))?;
    fs::create_dir_all(root.join(".cargo"))?;
    for source in fs::read_dir(original.join("tools/effect-dsl-lints"))? {
        let source = source?.path();
        if source
            .extension()
            .is_some_and(|extension| extension == "rs")
        {
            let name = source
                .file_name()
                .ok_or_else(|| io::Error::other("missing source name"))?;
            fs::copy(&source, tool.join(name))?;
        }
    }
    for source in fs::read_dir(original.join("crates/effect_macros/src"))? {
        let source = source?.path();
        if source
            .extension()
            .is_some_and(|extension| extension == "rs")
        {
            let name = source
                .file_name()
                .ok_or_else(|| io::Error::other("missing macro source name"))?;
            fs::copy(&source, macros.join(name))?;
        }
    }
    let wrapper = tool.join("wrapper");
    let build = Command::new("rustc")
        .arg("--edition=2024")
        .arg(tool.join("wrapper.rs"))
        .arg("-o")
        .arg(&wrapper)
        .env("CARGO_MANIFEST_DIR", &tool)
        .output()?;
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    fs::write(
        consumer.join("Cargo.toml"),
        "[package]\nname=\"cache-consumer\"\nversion=\"0.1.0\"\nedition=\"2024\"\n[workspace]\n",
    )?;
    let body = "#![allow(dead_code)]\nfn observe(program: fx::Effect<u8, ()>) { let _ = program.either(); }\n";
    fs::write(consumer.join("src/lib.rs"), body)?;
    fs::write(
        root.join(".cargo/config.toml"),
        format!(
            "[build]\nrustc-wrapper={:?}\nrustflags=[\"--extern\", {:?}, \"-L\", {:?}]\n",
            wrapper.to_string_lossy(),
            format!("fx={}", metadata(original)?.display()),
            format!(
                "dependency={}",
                original.join("target/debug/deps").display()
            )
        ),
    )?;
    let check = || {
        Command::new("cargo")
            .arg("+1.99.0")
            .arg("check")
            .arg("--offline")
            .arg("--manifest-path")
            .arg(consumer.join("Cargo.toml"))
            .arg("-j2")
            .current_dir(&consumer)
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .output()
    };
    let first = check()?;
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let registry = tool.join("registry.rs");
    let old = fs::read_to_string(&registry)?;
    let revised = old.replacen("    \"new\",", "    \"new\",\n    \"either\",", 1);
    assert_ne!(old, revised);
    fs::write(registry, revised)?;
    let second = check()?;
    assert_eq!(fs::read_to_string(consumer.join("src/lib.rs"))?, body);
    let diagnostics = String::from_utf8_lossy(&second.stderr);
    assert!(
        !second.status.success(),
        "cached consumer escaped changed policy: {diagnostics}"
    );
    assert!(
        diagnostics.contains("manual DSL construction through") && diagnostics.contains("either"),
        "wrong failure: {diagnostics}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
