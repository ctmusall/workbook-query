"""PyInstaller entry point for the windowed build (GUI, or CLI when given options)."""

import sys

from wbquery.__main__ import main

sys.exit(main())
