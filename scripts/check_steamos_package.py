"""Check archive contents, binary ABI/version and an isolated no-service install."""
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tarfile
import tempfile

def main():
    if os.name == "nt":
        raise SystemExit("Run this package check on Linux")
    archive = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="savesync-package-") as temporary:
        root = Path(temporary)
        with tarfile.open(archive) as stream:
            for member in stream.getmembers():
                path = PurePosixPath(member.name)
                assert not path.is_absolute() and ".." not in path.parts
                assert not member.issym() and not member.islnk()
        subprocess.run(["tar", "-xzf", str(archive), "-C", str(root)], check=True)
        package = root / "savesync-steamos"
        version = (package / "VERSION").read_text().strip()
        assert (package / "licenses/manifests.txt").is_file()
        assert (package / "decky-plugin/dist/index.js").stat().st_size > 100
        for binary in ("savesync-daemon", "savesync-app"):
            path = package / binary
            output = subprocess.check_output([str(path), "--version"], text=True).strip()
            assert output.endswith(" " + version), output
            symbols = subprocess.check_output(["objdump", "-T", str(path)], text=True)
            required = [tuple(map(int, match)) for match in re.findall(r"GLIBC_(\d+)\.(\d+)", symbols)]
            assert max(required, default=(0, 0)) <= (2, 35), (binary, max(required))
        home = root / "isolated home"
        home.mkdir()
        environment = os.environ.copy()
        environment.update(HOME=str(home), XDG_DATA_HOME=str(home / ".local/share"), SAVESYNC_SOCKET="package-check-unused")
        subprocess.run(["bash", str(package / "install-steamos.sh"), "--no-start"], env=environment, check=True, timeout=30)
        assert (home / ".local/bin/savesync-daemon").is_file()
        assert (home / ".local/bin/savesync-app").is_file()
        assert (home / ".config/systemd/user/savesync.service").is_file()
        print(f"PASS: SteamOS {version} archive, GLIBC<=2.35, executable modes, Decky frontend and isolated install")

if __name__ == "__main__":
    main()
