//! Exact export identities, native owner associations and reviewed lowerings.
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Fingerprint source with a stable algorithm independent of host/build hashing.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

/// All actual macro source files are reconciled, including modes and local forms.
fn sources(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(root.join("crates/effect_macros/src"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    paths.retain(|path| path.extension().is_some_and(|extension| extension == "rs"));
    paths.sort();
    Ok(paths)
}

/// Associate a procedural attribute through intervening documentation attributes.
fn exports(source: &str) -> io::Result<BTreeSet<String>> {
    source
        .split("#[proc_macro]")
        .skip(1)
        .map(|section| {
            let tail = section
                .split_once("pub fn ")
                .ok_or_else(|| io::Error::other("unmapped procedural export"))?
                .1;
            Ok(tail
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect())
        })
        .collect()
}

/// Reconcile the exact public names, not just an equal count.
pub fn verify(root: &Path) -> io::Result<()> {
    let actual = exports(&fs::read_to_string(
        root.join("crates/effect_macros/src/lib.rs"),
    )?)?;
    let registry = fs::read_to_string(root.join("tools/effect-dsl-lints/registry.rs"))?;
    let trust = registry
        .split_once("const EFFECT:")
        .ok_or_else(|| io::Error::other("missing compiler registry"))?
        .0;
    let names = trust
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix('"')?
                .split_once('"')
                .map(|(name, _)| name.to_owned())
        })
        .collect::<BTreeSet<_>>();
    if names != actual {
        return Err(io::Error::other(
            "compiler trust identities differ from actual procedural exports",
        ));
    }
    let associations = registry
        .split_once("pub const MACRO_OWNERS:")
        .ok_or_else(|| io::Error::other("missing lowering rule associations"))?
        .1;
    let mapped = associations
        .split('(')
        .skip(1)
        .filter_map(|section| {
            section
                .trim_start()
                .strip_prefix('"')?
                .split_once('"')
                .map(|(name, _)| name.to_owned())
        })
        .collect::<BTreeSet<_>>();
    if mapped != actual {
        return Err(io::Error::other(
            "an exported macro lacks a native construction rule association",
        ));
    }
    let paths = sources(root)?;
    let reviewed = fs::read_to_string(root.join("tools/effect-dsl-lints/reviewed.rs"))?;
    let mut expected = Vec::new();
    for line in reviewed
        .lines()
        .filter(|line| line.trim_start().starts_with("(\""))
    {
        let line = line.trim().trim_start_matches('(');
        let (name, tail) = line
            .strip_prefix('"')
            .and_then(|line| line.split_once('"'))
            .ok_or_else(|| io::Error::other("invalid reviewed source identity"))?;
        let hash = tail
            .trim()
            .strip_prefix(", 0x")
            .and_then(|tail| tail.strip_suffix("),"))
            .ok_or_else(|| io::Error::other("invalid reviewed fingerprint"))?;
        expected.push((
            name,
            u64::from_str_radix(hash, 16).map_err(io::Error::other)?,
        ));
    }
    if paths.len() != expected.len() {
        return Err(io::Error::other(
            "macro lowering file set changed; reconcile compiler rules",
        ));
    }
    for (path, (name, expected)) in paths.iter().zip(expected) {
        if path.file_name().and_then(|name| name.to_str()) != Some(name)
            || fingerprint(&fs::read(path)?) != expected
        {
            return Err(io::Error::other(format!(
                "{} changed; reconcile native construction rules and regressions before updating reviewed.rs",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Print a reviewable source snapshot after construction-rule changes are verified.
pub fn print_snapshot(root: &Path) -> io::Result<()> {
    println!(
        "// Reviewed macro lowering source fingerprints.\npub const SOURCES: &[(&str, u64)] = &["
    );
    for path in sources(root)? {
        println!(
            "    ({:?}, 0x{:016x}),",
            path.file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| io::Error::other("non-UTF8 source name"))?,
            fingerprint(&fs::read(&path)?)
        );
    }
    println!("];\n");
    Ok(())
}
