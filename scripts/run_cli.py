"""PyInstaller entry point for the console build (always the CLI)."""

import sys

from wbquery.cli import main

sys.exit(main())
