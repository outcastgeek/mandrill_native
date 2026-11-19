//! Regressions run the mandatory native Cargo wrapper against actual Effect metadata.
use std::borrow::Cow;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
/// A successful mandatory check validates metadata once before downstream probes.
static METADATA: OnceLock<Result<(PathBuf, Vec<PathBuf>), String>> = OnceLock::new();

/// Select the metadata built by the required native workspace check.
fn build_artifacts(root: &Path) -> io::Result<(PathBuf, Vec<PathBuf>)> {
    METADATA
        .get_or_init(|| validated_metadata(root).map_err(|error| error.to_string()))
        .clone()
        .map_err(io::Error::other)
}

/// Require current source to pass the configured compiler lint before selecting its library artifact.
fn validated_metadata(root: &Path) -> io::Result<(PathBuf, Vec<PathBuf>)> {
    let check = Command::new("cargo")
        .args([
            "+1.100.0",
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
        let stdout = String::from_utf8_lossy(&check.stdout);
        let diagnostics = compiler_diagnostics(&stdout);
        return Err(io::Error::other(format!(
            "mandatory Effect metadata check failed: {}\n{diagnostics}",
            String::from_utf8_lossy(&check.stderr)
        )));
    }
    let output = String::from_utf8(check.stdout).map_err(io::Error::other)?;
    let manifest = root.join("crates/effect/Cargo.toml").canonicalize()?;
    let (artifact, directories) = cargo_artifacts(&output, &manifest)?;
    if !artifact.metadata()?.is_file() {
        return Err(io::Error::other(
            "Cargo reported missing Effect library metadata",
        ));
    }
    Ok((artifact, directories))
}

#[derive(Default)]
struct CargoMessage<'a> {
    reason: Option<Cow<'a, str>>,
    manifest: Option<&'a str>,
    target: Option<&'a str>,
    filenames: Option<&'a str>,
}

impl<'a> CargoMessage<'a> {
    fn parse(line: &'a str) -> io::Result<Self> {
        let mut message = Self::default();
        if !line.trim_start().starts_with('{') {
            return Ok(message);
        }
        json_fields(line, |name, value| {
            match name {
                "reason" => message.reason = Some(json_text(value)?),
                "manifest_path" => message.manifest = Some(value),
                "target" => message.target = Some(value),
                "filenames" => message.filenames = Some(value),
                _ => {}
            }
            Ok(())
        })?;
        Ok(message)
    }

    fn effect_library(&self, manifest: &Path) -> io::Result<bool> {
        let Some(target) = self.target else {
            return Ok(false);
        };
        let mut name = None;
        let mut kind = None;
        json_fields(target, |key, value| {
            match key {
                "name" => name = Some(json_text(value)?),
                "kind" => kind = Some(json_text_array(value)?),
                _ => {}
            }
            Ok(())
        })?;
        if name.as_deref() != Some("effect") || !kind.is_some_and(|kind| kind.as_slice() == ["lib"])
        {
            return Ok(false);
        }

        let path = json_text(self.manifest.ok_or_else(bad_json)?)?;
        let path = Path::new(path.as_ref());
        Ok(path == manifest || path.canonicalize().is_ok_and(|path| path == manifest))
    }
}

fn compiler_diagnostics(output: &str) -> Cow<'_, str> {
    let messages = output
        .lines()
        .filter(|line| {
            CargoMessage::parse(line)
                .is_ok_and(|message| message.reason.as_deref() == Some("compiler-message"))
        })
        .collect::<Vec<_>>();
    if messages.is_empty() {
        // Retain stdout if Cargo failed before emitting a compiler message.
        Cow::Borrowed(output)
    } else {
        Cow::Owned(messages.join("\n"))
    }
}

/// Cargo reports exact artifacts even when fresh. The check's target/profile/
/// feature selection is authoritative, including its dependency locations.
fn cargo_artifacts(output: &str, manifest: &Path) -> io::Result<(PathBuf, Vec<PathBuf>)> {
    let mut artifact = None;
    let mut directories = Vec::new();
    for line in output.lines() {
        let message = CargoMessage::parse(line)?;
        if message.reason.as_deref() != Some("compiler-artifact") {
            continue;
        }
        let selected = message.effect_library(manifest)?;
        let filenames = json_text_array(message.filenames.ok_or_else(bad_json)?)?;
        for filename in filenames {
            let path = Path::new(filename.as_ref());
            if !path.is_absolute() {
                continue;
            }
            if let Some(parent) = path.parent() {
                directories.push(parent.to_path_buf());
            }
            if selected
                && path
                    .extension()
                    .is_some_and(|extension| extension == "rmeta")
            {
                if artifact.replace(path.to_path_buf()).is_some() {
                    return Err(io::Error::other(
                        "Cargo reported ambiguous Effect library metadata",
                    ));
                }
            }
        }
    }
    directories.sort_unstable();
    directories.dedup();
    let artifact =
        artifact.ok_or_else(|| io::Error::other("Cargo did not report Effect library metadata"))?;
    Ok((artifact, directories))
}

fn bad_json() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid Cargo JSON metadata")
}

fn json_fields<'a>(
    input: &'a str,
    mut visit: impl FnMut(&str, &'a str) -> io::Result<()>,
) -> io::Result<()> {
    let mut input = input.trim();
    json_punctuation(&mut input, '{')?;
    if input == "}" {
        return Ok(());
    }
    loop {
        let key = json_text(take_json_string(&mut input)?)?;
        json_punctuation(&mut input, ':')?;
        visit(&key, take_json_value(&mut input)?)?;
        if input == "}" {
            return Ok(());
        }
        json_punctuation(&mut input, ',')?;
    }
}

fn json_text_array(input: &str) -> io::Result<Vec<Cow<'_, str>>> {
    let mut input = input.trim();
    json_punctuation(&mut input, '[')?;
    let mut values = Vec::new();
    if input == "]" {
        return Ok(values);
    }
    loop {
        values.push(json_text(take_json_string(&mut input)?)?);
        if input == "]" {
            return Ok(values);
        }
        json_punctuation(&mut input, ',')?;
    }
}

fn json_punctuation(input: &mut &str, expected: char) -> io::Result<()> {
    *input = input
        .strip_prefix(expected)
        .ok_or_else(bad_json)?
        .trim_start();
    Ok(())
}

fn take_json_string<'a>(input: &mut &'a str) -> io::Result<&'a str> {
    let body = input.strip_prefix('"').ok_or_else(bad_json)?;
    let mut escaped = false;
    for (offset, byte) in body.bytes().enumerate() {
        if byte < b' ' {
            return Err(bad_json());
        }
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            let (quoted, tail) = input.split_at(offset + 2);
            *input = tail.trim_start();
            return Ok(quoted);
        }
    }
    Err(bad_json())
}

/// Skip irrelevant Cargo fields while keeping nested objects/arrays and
/// quoted delimiters intact. Only fields the harness consumes are decoded.
fn take_json_value<'a>(input: &mut &'a str) -> io::Result<&'a str> {
    let first = input.as_bytes().first().copied().ok_or_else(bad_json)?;
    let end = match first {
        b'"' => return take_json_string(input),
        b'{' | b'[' => {
            let mut groups = Vec::new();
            let mut rest = *input;
            loop {
                let next = rest.chars().next().ok_or_else(bad_json)?;
                match next {
                    '"' => {
                        take_json_string(&mut rest)?;
                        continue;
                    }
                    '{' => groups.push('}'),
                    '[' => groups.push(']'),
                    '}' | ']' => {
                        if groups.pop() != Some(next) {
                            return Err(bad_json());
                        }
                        if groups.is_empty() {
                            break input.len() - rest.len() + 1;
                        }
                    }
                    _ => {}
                }
                rest = &rest[next.len_utf8()..];
            }
        }
        b'-' | b'0'..=b'9' | b'f' | b'n' | b't' => input
            .find(|character: char| {
                character == ',' || character == '}' || character.is_whitespace()
            })
            .unwrap_or(input.len()),
        _ => return Err(bad_json()),
    };
    let (value, rest) = input.split_at(end);
    *input = rest.trim_start();
    Ok(value)
}

fn json_text(quoted: &str) -> io::Result<Cow<'_, str>> {
    let text = quoted
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .ok_or_else(bad_json)?;
    let mut rest = text;
    let Some(mut escape) = rest.find('\\') else {
        return Ok(Cow::Borrowed(text));
    };
    let mut decoded = String::with_capacity(text.len());
    loop {
        let (plain, sequence) = rest.split_at(escape);
        decoded.push_str(plain);
        let mut characters = sequence.chars();
        characters.next();
        let code = characters.next().ok_or_else(bad_json)?;
        rest = characters.as_str();
        decoded.push(match code {
            '"' | '\\' | '/' => code,
            'b' => '\u{8}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'u' => {
                let high = unicode_unit(&mut rest)?;
                let scalar = if (0xd800..=0xdbff).contains(&high) {
                    rest = rest.strip_prefix("\\u").ok_or_else(bad_json)?;
                    let low = unicode_unit(&mut rest)?;
                    if !(0xdc00..=0xdfff).contains(&low) {
                        return Err(bad_json());
                    }
                    0x1_0000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(low) - 0xdc00)
                } else {
                    u32::from(high)
                };
                char::from_u32(scalar).ok_or_else(bad_json)?
            }
            _ => return Err(bad_json()),
        });
        let Some(next) = rest.find('\\') else {
            decoded.push_str(rest);
            return Ok(Cow::Owned(decoded));
        };
        escape = next;
    }
}

fn unicode_unit(input: &mut &str) -> io::Result<u16> {
    let (unit, rest) = input.split_at_checked(4).ok_or_else(bad_json)?;
    if !unit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(bad_json());
    }
    *input = rest;
    u16::from_str_radix(unit, 16).map_err(|_| bad_json())
}

#[test]
#[cfg(unix)]
fn cargo_artifact_decodes_exact_paths() -> io::Result<()> {
    let output = r#"
        { "reason": "compiler-artifact", "filenames": ["/vendor/effect/libeffect.rmeta"], "manifest_path": "/vendor/effect/Cargo.toml", "target": {"kind": ["lib"],"name":"effect"} }
        { "reason": "compiler-artifact", "filenames": ["/wrong-kind/libeffect.rmeta"], "manifest_path": "/workspace/crates/effect/Cargo.toml", "target": {"kind": ["bin"],"name":"effect","other":{"kind":["lib"],"name":"effect"}} }
        { "filenames": ["\/target/custom [\"] \\ \u03c0 \ud83d\ude80/deps/libeffect.rmeta", "/target/custom [\"] \\ π 🚀/deps/libeffect.rlib"], "target": {"kind": ["lib"],"name":"effect"}, "manifest_path": "/workspace/crates/\u0065ffect/Cargo.toml", "reason": "compiler-artifact", "fresh": true, "ignored": [{"filenames":["/ignored/libwrong.rmeta"],"closing":"] } \""}] }
        { "reason": "compiler-artifact", "filenames": ["/build root/proc/dependency.so"], "target": {"name":"derived","kind":["proc-macro"]} }
        { "reason": "build-finished", "filenames": ["/unrelated/bogus.rmeta"], "success": true }
    "#;
    let manifest = Path::new("/workspace/crates/effect/Cargo.toml");
    let (artifact, directories) = cargo_artifacts(output, manifest)?;
    let selected = Path::new("/target/custom [\"] \\ π 🚀/deps");
    assert_eq!(artifact, selected.join("libeffect.rmeta"));
    assert_eq!(
        directories,
        [
            PathBuf::from("/build root/proc"),
            selected.to_path_buf(),
            PathBuf::from("/vendor/effect"),
            PathBuf::from("/wrong-kind"),
        ]
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn cargo_artifact_rejects_ambiguous_metadata() {
    let output = r#"{"reason":"compiler-artifact","manifest_path":"/workspace/crates/effect/Cargo.toml","target":{"name":"effect","kind":["lib"]},"filenames":["/check/first.rmeta","/check/second.rmeta"]}"#;
    let manifest = Path::new("/workspace/crates/effect/Cargo.toml");
    let Err(error) = cargo_artifacts(output, manifest) else {
        panic!("ambiguous metadata was accepted");
    };
    assert!(error.to_string().contains("ambiguous"));
}

#[test]
fn cargo_artifact_preserves_compiler_diagnostics() {
    let diagnostic = r#"{ "message": { "rendered": "diagnostic with ] and \"" }, "\u0072eason": "compiler-message" }"#;
    let output = format!("{{\"reason\":\"build-finished\"}}\n{diagnostic}\n");
    assert_eq!(compiler_diagnostics(&output), diagnostic);
    assert_eq!(
        compiler_diagnostics("unstructured Cargo output"),
        "unstructured Cargo output"
    );
}

/// Exact metadata and dependency directories from the same Cargo invocation.
fn metadata(root: &Path) -> io::Result<PathBuf> {
    build_artifacts(root).map(|(metadata, _)| metadata)
}

fn dependency_directories(root: &Path) -> io::Result<Vec<PathBuf>> {
    build_artifacts(root).map(|(_, directories)| directories)
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
        .env(
            "CARGO_MANIFEST_DIR",
            root.join("crates/effect/tests/fixtures/runtime_neutral"),
        )
        .env("CARGO_PKG_NAME", "effect-neutral-consumer");
    for directory in dependency_directories(root)? {
        command
            .arg("-L")
            .arg(format!("dependency={}", directory.display()));
    }
    if crate_name == "effect" {
        command.arg("--cfg=test");
    } else {
        command
            .arg("--extern")
            .arg(format!("fx={}", metadata(root)?.display()));
    }
    let result = command.args(flags).output();
    // A failed probe can also leave rustc's expanded-type diagnostic in this directory.
    fs::remove_dir_all(directory)?;
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
    let mut rustflags = vec![
        "--extern".to_owned(),
        format!("fx={}", metadata(original)?.display()),
    ];
    for directory in dependency_directories(original)? {
        rustflags.extend([
            "-L".to_owned(),
            format!("dependency={}", directory.display()),
        ]);
    }
    fs::write(
        root.join(".cargo/config.toml"),
        format!(
            "[build]\nrustc-wrapper={:?}\nrustflags={rustflags:?}\n",
            wrapper.to_string_lossy(),
        ),
    )?;
    let check = || {
        Command::new("cargo")
            .arg("+1.100.0")
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

#[test]
fn prepared_consumer_forms_keep_renamed_and_native_boundaries() -> io::Result<()> {
    let result = compile(
        r#"
        use fx::memory::{HasMemoryPool, Memory, MemoryError, RegionId, SlotPool};
        use fx::{DynamicRunContract, Effect};
        use std::sync::Arc;

        #[macro_export]
        macro_rules! foreign_future { () => { 7u8 } }
        mod native { pub(crate) use crate::foreign_future as effect_future; }

        struct Owned(u8);
        struct App { memory: Memory<'static>, outputs: SlotPool<'static, Owned> }
        fx::memory! { capability App => self.memory; }
        impl HasMemoryPool<()> for App {
            type Value = Owned;
            fn memory_pool(&self, _: &()) -> &SlotPool<'static, Owned> { &self.outputs }
        }

        fn main() -> Result<(), MemoryError> {
            let memory = fx::memory! {
                total: 32 KiB, mode: prepared, metadata: work, partition work: 32 KiB,
            }?.prepare()?;
            let region = RegionId::named("work");
            let _: Effect<fx::memory::SlotLease<'static, Owned>, MemoryError, App> =
                fx::memory_lease! { pool: (), on_error: std::convert::identity, };
            let values = fx::memory_vec! { allocator: memory.allocator(region)?, capacity: 1, init |index| {
                Ok::<_, std::convert::Infallible>(Owned(u8::try_from(index).unwrap_or(0)))
            }, };
            let owned = fx::effect_drop_guard! {
                value: Owned(11),
                cleanup: |item: Owned| { let _ = item.0; },
            };
            let recovered = std::mem::DropGuard::dismiss(owned);
            let plan = fx::effect_prepare! {
                graph;
                program: fx::effect! {
                    let qualified = native::effect_future!();
                    let nested = || native::effect_future!();
                    let value = fx::effect_try_sync! { body: move || Ok::<_, !>(qualified + nested() + recovered.0), }?;
                    value
                },
                contract: DynamicRunContract::GeneratedGraph { clock_ticks: 0, contexts: 1, listeners: 0, bridges: 0 },
            };
            let demand = plan.demand(1)?;
            let frames = memory.frames(region, demand.frames_per_run, demand.frame_layout)?;
            let run = fx::effect_prepare! {
                prepare; plan: plan, memory: memory, region: region, capacity: 1, frames: frames,
            }?;
            let _ = fx::effect_prepare! { admit; run: &run, environment: Arc::new(()), }?;
            let _ = values;
            Ok(())
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
fn prepared_consumer_forms_reject_invalid_units_ownership_and_callbacks() -> io::Result<()> {
    for (body, diagnostic) in [
        (
            "fn main() { let _: fx::Effect<u8, fx::memory::MemoryError> = fx::Effect::from_prepared_exit_future_with_context(|_: std::sync::Arc<()>, _| async { fx::Exit::Success(1) }, std::convert::identity); }",
            "manual DSL construction through",
        ),
        (
            "fn main() { let _ = fx::memory! { total: 16 bogus, mode: prepared, partition work: 16 B, }; }",
            "expected B, kB, MB, KiB, MiB or GiB",
        ),
        (
            "fn main() { let _ = fx::memory! { total: 1 KiB, total: 2 KiB, mode: prepared, partition work: 1 KiB, }; }",
            "duplicate memory field",
        ),
        (
            "fn main() { let _ = fx::effect_drop_guard! { value: 1u8, cleanup: async |_: u8| {}, }; }",
            "DropGuard cleanup must be synchronous",
        ),
        (
            "fn main() { let _ = fx::effect_drop_guard! { value: 1u8, cleanup: |_: u8| {}, surplus: (), }; }",
            "unknown or conflicting descriptor field",
        ),
        (
            "fn main() { let _ = fx::memory_lease! { environment: (), pool: (), on_error: std::convert::identity, }; }",
            "HasMemoryPool",
        ),
        (
            r#"use fx::memory::*;
                fn leak(backing: &mut [std::mem::MaybeUninit<u8>]) -> Result<SlotLease<'static, u8>, MemoryError> {
                    let config = fx::memory! { total: 4 KiB, mode: prepared, metadata: work, partition work: 4 KiB, }?;
                    let memory = config.plan()?.prepare_borrowed(backing)?;
                    memory.slots(RegionId::named("work"), 1, |_| Ok(1u8))?.try_checkout()
                }
                fn main() {}"#,
            "lifetime may not live long enough",
        ),
        (
            "fn main() { let guard = fx::effect_drop_guard! { value: String::new(), cleanup: drop, }; drop(guard); let _ = std::mem::DropGuard::dismiss(guard); }",
            "use of moved value",
        ),
    ] {
        let result = compile(body)?;
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            !result.status.success(),
            "invalid prepared consumer compiled: {body}"
        );
        assert!(
            stderr.contains(diagnostic),
            "expected {diagnostic:?}: {stderr}"
        );
    }
    Ok(())
}
