"""Exercise packaging failures without building, signing, or stopping real apps."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class BuildScriptsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="zeditor scripts ")
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        self.bin = self.work / "bin"
        self.bin.mkdir()
        self.build = self.work / "build output"
        self.install = self.work / "Applications"
        self.app = self.install / "Zeditor.app"
        self.events = self.work / "events"
        self.env = {
            **os.environ,
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "CARGO_TARGET_DIR": str(self.build),
            "ZEDITOR_INSTALL_DIR": str(self.install),
            "ZEDITOR_SIGNING_IDENTITY": "Test Identity",
            "TEST_EVENTS": str(self.events),
        }
        self.command("cargo", '''
[[ "$*" == *"--locked"* && "$*" == *"--bin zeditor"* ]] || exit 91
while [[ "$1" != "--target-dir" ]]; do shift; done
mkdir -p "$2/release"
printf '#!/bin/bash\\nexit 0\\n' > "$2/release/zeditor"
chmod +x "$2/release/zeditor"
''')
        self.command("codesign", '''
echo "codesign $1" >> "$TEST_EVENTS"
[[ "${FAIL_SIGN:-}" != "$1" ]]
''')
        self.command("pkill", 'echo pkill >> "$TEST_EVENTS"')
        self.command("open", 'echo open >> "$TEST_EVENTS"')
        self.command("mv", '''
if [[ "${FAIL_REPLACE:-}" == 1 && "$1" == *".Zeditor-update."*"/Zeditor.app" ]]; then
    exit 92
fi
exec /bin/mv "$@"
''')

    def command(self, name, body):
        path = self.bin / name
        path.write_text("#!/bin/bash\nset -euo pipefail\n" + body + "\n")
        path.chmod(0o755)

    def run_script(self, script, **env):
        return subprocess.run(
            ["/bin/bash", str(ROOT / script)], cwd=self.work,
            env={**self.env, **env}, capture_output=True, text=True,
        )

    def existing_app(self):
        self.app.mkdir(parents=True)
        (self.app / "keep").write_text("working installation")

    def assert_no_staging(self):
        self.assertEqual(list(self.install.glob(".Zeditor-update.*")), [])

    def test_bundle_has_correct_executable_plist_and_icon_from_another_directory(self):
        result = self.run_script("bundle.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        contents = self.build / "Zeditor.app/Contents"
        self.assertTrue(os.access(contents / "MacOS/zeditor", os.X_OK))
        self.assertEqual((contents / "Info.plist").read_bytes(), (ROOT / "Info.plist").read_bytes())
        self.assertEqual((contents / "Resources/AppIcon.icns").read_bytes(), (ROOT / "AppIcon.icns").read_bytes())

    def test_first_install_creates_applications_directory(self):
        result = self.run_script("update.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.app / "Contents/MacOS/zeditor").exists())
        self.assertEqual(self.events.read_text().splitlines(), [
            "codesign --force", "codesign --verify", "pkill", "open",
        ])
        self.assert_no_staging()

    def test_successful_update_replaces_previous_app(self):
        self.existing_app()
        result = self.run_script("update.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.app / "keep").exists())
        self.assertTrue((self.app / "Contents/MacOS/zeditor").exists())
        self.assert_no_staging()

    def test_signing_failures_preserve_app_without_stopping_it(self):
        self.existing_app()
        for failure in ("--force", "--verify"):
            with self.subTest(failure=failure):
                result = self.run_script("update.sh", FAIL_SIGN=failure)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual((self.app / "keep").read_text(), "working installation")
                self.assertNotIn("pkill", self.events.read_text())
                self.assertNotIn("open", self.events.read_text())
                self.assert_no_staging()

    def test_failed_replacement_restores_previous_app(self):
        self.existing_app()
        result = self.run_script("update.sh", FAIL_REPLACE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.app / "keep").read_text(), "working installation")
        self.assertNotIn("open", self.events.read_text())
        self.assert_no_staging()


if __name__ == "__main__":
    unittest.main()
