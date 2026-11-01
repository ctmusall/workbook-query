# Developer tasks. Install `just` (https://just.systems) or read the recipes and run the commands.
# `just ci` runs the same checks as GitHub Actions.

set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

default: ci

# Format the code.
fmt:
    cargo fmt --all

# Formatting, lints and dependency checks, without changing anything.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo deny check

# Run the test suite.
test:
    cargo test --workspace

# Build the release executables into target/release/.
build:
    cargo build --release --workspace

# Run the built CLI on the smoke-test query, as CI does.
smoke: build
    ./target/release/wbquery-cli --version
    ./target/release/wbquery-cli -q tests/golden/smoke/query.json -o target/smoke/results --print

# Everything CI checks.
ci: lint test smoke

# Regenerate the golden fixtures from the Python implementation (only while it still exists).
golden:
    .venv/bin/python tests/golden/generate.py
    .venv/bin/python tests/golden/generate_charts.py

# Save every fixture chart as SVG, PNG and PDF into target/charts/ to look at.
charts:
    WBQ_CHART_DIR=target/charts cargo test -p wbquery-charts --release --test render
