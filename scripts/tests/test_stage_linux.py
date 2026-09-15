import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[2]


class StageLinuxTest(unittest.TestCase):
    def test_stages_license_and_third_party_notice(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            binary = temp / "diffz"
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(binary.stat().st_mode | stat.S_IXUSR)
            stage = temp / "linux-root"

            env = os.environ.copy()
            env["DIFFZ_BINARY"] = str(binary)
            ginstall = shutil.which("ginstall")
            if ginstall:
                install_bin = temp / "bin"
                install_bin.mkdir()
                (install_bin / "install").symlink_to(ginstall)
                env["PATH"] = f"{install_bin}{os.pathsep}{env['PATH']}"
            subprocess.run(
                ["bash", str(REPO / "scripts/stage-linux.sh"), str(stage)],
                cwd=REPO,
                env=env,
                check=True,
            )

            staged_notice = stage / "usr/share/doc/diffz/THIRD_PARTY_NOTICES.md"
            self.assertTrue(staged_notice.is_file(), "staged THIRD_PARTY_NOTICES.md is missing")
            staged_license = stage / "usr/share/licenses/diffz/LICENSE"
            self.assertTrue(staged_license.is_file(), "staged LICENSE is missing")
            self.assertEqual(staged_notice.read_bytes(), (REPO / "THIRD_PARTY_NOTICES.md").read_bytes())
            self.assertEqual(staged_license.read_bytes(), (REPO / "LICENSE").read_bytes())


if __name__ == "__main__":
    unittest.main()
