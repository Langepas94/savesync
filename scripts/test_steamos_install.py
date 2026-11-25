"""Installer regression tests on temporary homes and a mocked systemctl; no live service."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

def executable(path, text):
    path.write_text(text, encoding="utf-8")
    path.chmod(0o755)

def main():
    if os.name == "nt":
        print("SKIP: SteamOS installer tests require POSIX; Ubuntu CI runs them")
        return
    with tempfile.TemporaryDirectory(prefix="savesync-install-") as temporary:
        root = Path(temporary)
        home = root / "home with spaces"
        home.mkdir()
        package = root / "package"
        package.mkdir()
        shutil.copy(ROOT / "packaging/install-steamos.sh", package / "install-steamos.sh")
        shutil.copytree(ROOT / "packaging/systemd", package / "systemd")
        for name in ("savesync-daemon", "savesync-app"):
            executable(package / name, "#!/bin/bash\nif [[ $1 == --version ]]; then echo 'SaveSync 1.0.0'; exit 0; fi\nprintf '%s %s\\n' \"$(basename \"$0\")\" \"$*\" >> \"$TEST_LOG\"\n")
        tools = root / "tools"
        tools.mkdir()
        executable(tools / "systemctl", "#!/bin/bash\nprintf 'systemctl %s\\n' \"$*\" >> \"$TEST_LOG\"\nif [[ ${TEST_FAIL_START:-0} == 1 && $* == *'enable --now'* ]]; then exit 1; fi\nexit 0\n")
        log = root / "calls.log"
        environment = os.environ.copy()
        environment.update(HOME=str(home), TEST_LOG=str(log), PATH=str(tools) + os.pathsep + environment["PATH"])
        def install(success=True):
            result = subprocess.run(["bash", str(package / "install-steamos.sh")], env=environment, capture_output=True, text=True, timeout=30)
            assert (result.returncode == 0) == success, (result.stdout, result.stderr)
        install()
        daemon = home / ".local/bin/savesync-daemon"
        app = home / ".local/bin/savesync-app"
        unit = home / ".config/systemd/user/savesync.service"
        desktop = home / ".local/share/applications/savesync.desktop"
        assert daemon.is_file() and app.is_file() and unit.is_file()
        assert str(app) in desktop.read_text()
        assert "ExecStop=" in unit.read_text() and "TimeoutStopSec=infinity" in unit.read_text()
        marker = home / ".local/share/savesync/preserve.txt"
        marker.write_text("user data", encoding="utf-8")
        before = {path: path.read_bytes() for path in (daemon, app, unit, desktop)}
        log.write_text("")
        install()
        calls = log.read_text()
        assert "savesync-daemon stop" in calls and "savesync-app --quit" in calls
        assert calls.index("savesync-daemon stop") < calls.index("systemctl --user enable --now")
        assert marker.read_text() == "user data"
        # Failed service start restores all installed executable/unit/desktop files.
        for name in ("savesync-daemon", "savesync-app"):
            content = (package / name).read_text().replace("1.0.0", "2.0.0")
            executable(package / name, content)
        environment["TEST_FAIL_START"] = "1"
        install(success=False)
        assert all(path.read_bytes() == content for path, content in before.items())
        assert marker.read_text() == "user data"
        assert not list((home / ".local/bin").glob(".savesync-stage.*"))
        (package / "savesync-app").unlink()
        install(success=False)
        assert all(path.read_bytes() == content for path, content in before.items())
        print("PASS: SteamOS install, graceful upgrade ordering, rollback, incomplete package, preserved profile, paths with spaces")

if __name__ == "__main__":
    main()
