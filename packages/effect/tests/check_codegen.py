"""Verify the declared static-cost proofs share LLVM's optimized functions."""
from pathlib import Path
import re
import sys

ir = Path(sys.argv[1]).read_text()
symbol = r'(?:"[^"]+"|[-A-Za-z0-9_.$]+)'
aliases = {
    name.strip('"'): target.strip('"')
    for name, target in re.findall(
        rf'^@({symbol}) = [^\n]*\balias [^\n]*\bptr @({symbol})(?:\s|$)',
        ir, re.MULTILINE,
    )
}


def resolve(name):
    seen = set()
    while name in aliases:
        if name in seen:
            sys.exit(f"Invalid alias cycle at {name}")
        seen.add(name)
        name = aliases[name]
    if not re.search(rf'^define [^\n]*@"?{re.escape(name)}"?\(', ir, re.MULTILINE):
        sys.exit(f"Missing optimized definition for {name}")
    return name


def instructions(name):
    # Function attributes and debug labels can prevent merging even when the
    # instructions are identical. Compare the complete optimized body as well.
    body = re.search(rf'^define [^\n]*@"?{re.escape(name)}"?\([^\n]*\{{\n(.*?)^\}}', ir, re.MULTILINE | re.DOTALL)
    if body is None:
        sys.exit(f"Missing optimized body for {name}")
    registers = {}
    result = []
    for line in body.group(1).splitlines():
        line = line.strip()
        if not line or line.startswith('#dbg_') or re.match(r'^[-A-Za-z0-9_.$]+:', line):
            continue
        line = re.sub(r', ![-A-Za-z0-9_.]+ ![0-9]+', '', line)
        line = re.sub(r'%[-A-Za-z0-9_.$]+', lambda match: registers.setdefault(match.group(), f'%{len(registers)}'), line)
        result.append(line)
    return result


failures = []
for baseline, consumers in (
    ("handwritten", ("effect_map", "effect_bind", "effect_environment_bind")),
    ("handwritten_two_binds", ("effect_two_binds",)),
    ("handwritten_eight_binds", ("effect_eight_binds",)),
    ("handwritten_recovery", ("effect_recovery",)),
):
    expected = resolve(baseline)
    for consumer in consumers:
        actual = resolve(consumer)
        if actual != expected and instructions(actual) != instructions(expected):
            failures.append(f"{consumer} retains different optimized instructions from {baseline}")
        else:
            print(f"{consumer}: same optimized instructions as {baseline}")
if failures:
    sys.exit("\n".join(failures))
