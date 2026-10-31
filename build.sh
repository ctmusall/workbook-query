#!/usr/bin/env bash
# Build Linux executables into dist/ (runs the test suite first).
set -euo pipefail
cd "$(dirname "$0")"

if [ ! -d .venv ]; then
    python3 -m venv .venv
fi
.venv/bin/pip install -q --upgrade pip
.venv/bin/pip install -q -e '.[dev]'

QT_QPA_PLATFORM=offscreen .venv/bin/pytest --cov=wbquery -q
.venv/bin/pyinstaller --noconfirm --clean wbquery.spec

echo
echo "Built:"
ls -lh dist/
