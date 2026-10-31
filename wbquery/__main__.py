"""Entry point.

No arguments, or bare file paths (e.g. workbooks dropped onto the .exe), open
the GUI; anything with an option runs the CLI.
"""

import sys


def wants_gui(argv: list[str]) -> bool:
    return not any(a.startswith("-") for a in argv)


def main() -> int:
    if not wants_gui(sys.argv[1:]):
        from .cli import main as cli_main

        return cli_main(sys.argv[1:])
    from .gui import main as gui_main

    return gui_main()


if __name__ == "__main__":
    sys.exit(main())
