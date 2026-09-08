#!/usr/bin/env python3
"""Run the production Rust resolver against isolated real Fontconfig databases.

Needs rustc, fontconfig, DejaVu Sans and Liberation Mono; no Cargo, GPUI, network,
or display is needed. No system fonts/configuration are modified.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from xml.sax.saxutils import escape

REPO = Path(__file__).resolve().parents[2]


class CodeFontTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="diffz-font-tests-")
        cls.addClassCleanup(cls.temp.cleanup)
        cls.root = Path(cls.temp.name)
        cls.binary = cls.root / "resolve-font"
        harness = cls.root / "main.rs"
        module = json.dumps(str(REPO / "crates/app/src/code_font.rs"))
        harness.write_text(f'#[path = {module}] mod code_font;\n' + r'''
fn main() {
    let requested = std::env::args().nth(1);
    match code_font::resolve(requested.as_deref()) {
        Ok(family) => println!("{}", family),
        Err(error) => { eprintln!("{}", error); std::process::exit(1); }
    }
}
''')
        subprocess.run(["rustc", "--edition=2021", str(harness), "-o", str(cls.binary)],
                       check=True)
        cls.fonts = {}
        clean_env = dict(os.environ)
        clean_env.pop("FC_DEBUG", None)
        # Resolve fixture files before isolating the font configuration. An
        # absent fixture is a setup error, never a silently substituted font.
        for family in ("Liberation Mono", "DejaVu Sans"):
            output = subprocess.check_output(
                ["fc-match", "--format", "%{family[0]}\n%{file}\n", "--", family],
                env=clean_env, text=True).splitlines()
            if len(output) != 2 or output[0] != family:
                raise RuntimeError(f"Install the test font {family}: got {output!r}")
            cls.fonts[family] = Path(output[1])

    def setUp(self):
        self.case = Path(tempfile.mkdtemp(dir=self.root))
        self.font_dir = self.case / "fonts"
        self.font_dir.mkdir()
        for family, source in self.fonts.items():
            shutil.copyfile(source, self.font_dir / (family + source.suffix))
        self.config = self.case / "fonts.conf"
        self.config.write_text(
            '<fontconfig><reset-dirs/>'
            f'<dir>{escape(str(self.font_dir))}</dir>'
            f'<cachedir>{escape(str(self.case / "cache"))}</cachedir>'
            '<alias><family>monospace</family><prefer>'
            '<family>Liberation Mono</family></prefer></alias></fontconfig>')
        self.env = {**os.environ, "FONTCONFIG_FILE": str(self.config)}
        self.env.pop("FC_DEBUG", None)

    def resolve(self, *args, **env):
        return subprocess.run([str(self.binary), *args], env={**self.env, **env},
                              capture_output=True, text=True, timeout=10)

    def assert_mono(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "Liberation Mono")

    def test_default_with_no_dejavu_sans_mono_installed(self):
        listed = subprocess.check_output(["fc-list", "--format", "%{family}\n"],
                                         env=self.env, text=True)
        self.assertNotIn("DejaVu Sans Mono", listed)
        self.assert_mono(self.resolve())

    def test_explicit_installed_monospace_font(self):
        self.assert_mono(self.resolve("Liberation Mono"))

    def test_missing_explicit_font_does_not_select_proportional_fallback(self):
        self.assert_mono(self.resolve("DejaVu Sans Mono"))

    def test_explicit_proportional_font_falls_back_to_monospace(self):
        self.assert_mono(self.resolve("DejaVu Sans"))

    def test_fontconfig_properties_in_family_are_not_interpreted(self):
        self.assert_mono(self.resolve("Liberation Mono:spacing=0"))

    def test_only_proportional_fonts_is_a_clear_error(self):
        for path in self.font_dir.glob("Liberation Mono*"):
            path.unlink()
        result = self.resolve()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no usable monospace font", result.stderr)

    def test_no_fonts_is_a_clear_error(self):
        for path in self.font_dir.iterdir():
            path.unlink()
        result = self.resolve()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no usable monospace font", result.stderr)

    def test_missing_fontconfig_has_an_actionable_error(self):
        result = self.resolve(PATH=str(self.case / "empty-path"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("install fontconfig", result.stderr)

    def test_fontconfig_debug_does_not_corrupt_machine_output(self):
        self.assert_mono(self.resolve(FC_DEBUG="1"))


if __name__ == "__main__":
    unittest.main()
