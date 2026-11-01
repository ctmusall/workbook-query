# PyInstaller spec: builds two single-file executables from one analysis.
#   wbquery      - windowed GUI (double-click; drop workbooks onto it to open them)
#   wbquery-cli  - console build for scripting / batch use
# Build with:  pyinstaller --noconfirm wbquery.spec

# Qt modules the app never uses; excluding them keeps the executable smaller.
excludes = [
    "tkinter", "PySide6.QtWebEngineCore", "PySide6.QtWebEngineWidgets", "PySide6.Qt3DCore",
    "PySide6.QtQuick", "PySide6.QtQml", "PySide6.QtMultimedia", "PySide6.QtPdf", "PySide6.QtCharts",
    "PySide6.QtDataVisualization", "PySide6.QtBluetooth", "PySide6.QtNetworkAuth", "PySide6.QtSql",
    "PySide6.QtTest", "PyQt5", "PyQt6", "IPython", "pytest", "scipy",
]


def make_analysis(script, gui):
    return Analysis(
        [script],
        pathex=["."],
        datas=[("wbquery/assets/icon.png", "wbquery/assets")] if gui else [],
        # Saving a chart as SVG / PDF loads these backends only then, so PyInstaller can't see them.
        hiddenimports=["openpyxl"] + (["matplotlib.backends.backend_agg", "matplotlib.backends.backend_qtagg",
                                       "matplotlib.backends.backend_svg", "matplotlib.backends.backend_pdf"]
                                      if gui else []),
        # The console build never imports Qt or matplotlib (charts are GUI-only); leaving them out saves ~130 MB.
        excludes=excludes if gui else excludes + ["PySide6", "shiboken6", "matplotlib"],
        noarchive=False,
    )


def make_exe(a, name, console):
    pyz = PYZ(a.pure)
    return EXE(
        pyz, a.scripts, a.binaries, a.datas, [],
        name=name,
        console=console,
        icon="wbquery/assets/icon.ico",
        upx=False,
        strip=False,
        debug=False,
    )


gui = make_analysis("scripts/run_gui.py", gui=True)
make_exe(gui, "wbquery", console=False)

cli = make_analysis("scripts/run_cli.py", gui=False)
make_exe(cli, "wbquery-cli", console=True)
