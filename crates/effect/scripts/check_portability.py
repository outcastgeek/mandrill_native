"""Assert the independent consumer has no concrete execution-runtime dependencies."""
from pathlib import Path
import subprocess
import re

root = Path(__file__).resolve().parents[1]
manifest = root / "tests/fixtures/runtime_neutral/Cargo.toml"
for arguments in ([], ["--no-default-features"]):
    result = subprocess.run(
        ["cargo", "tree", "--manifest-path", str(manifest), "-e", "normal,build", "--prefix", "none", *arguments],
        check=True, capture_output=True, text=True, timeout=120,
    )
    packages = {line.split()[0] for line in result.stdout.splitlines() if line.strip()}
    forbidden = {name for name in packages if re.match(r"^(tokio($|-)|rayon($|-)|native-async-runtime$)", name)}
    if forbidden:
        raise SystemExit(f"Runtime dependency leaked into core: {sorted(forbidden)}")
print("PASS: default and no-default-features consumers have no Tokio/Rayon/native-runtime dependencies")
