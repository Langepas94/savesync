"""Exercise the real daemon CLI and IPC in isolated profiles, without a GUI.

Run cargo build -p savesync-daemon, then python scripts/smoke.py.
All save files, config and IPC names belong to a temporary test directory.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import uuid


def run(binary: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="savesync-smoke-") as temporary:
        root = Path(temporary)
        namespace = "ss-smoke-" + uuid.uuid4().hex[:10]
        manifests = root / "manifests"
        manifests.mkdir()
        (manifests / "smoke.yaml").write_text(
            "schema: 1\nname: Smoke Game\nslug: smoke-game\npaths:\n"
            " - pattern: '<home>/saves/*.sav'\n", encoding="utf-8"
        )
        environments = {}
        for device in ("pc", "deck"):
            home = root / device / "home"
            (home / "saves").mkdir(parents=True)
            (home / "saves/slot.sav").write_text(device + "-original", encoding="utf-8")
            appdata = root / device / "appdata"
            data = appdata / "savesync"
            data.mkdir(parents=True)
            (data / "config.json").write_text(json.dumps({
                "version": 1, "device_id": device, "device_name": device,
                "manifests_dir": str(manifests),
                "backends": {"fs_path": str(root / "remote")},
                "backups": {"dir": str(root / device / "backups"), "keep": 3},
            }), encoding="utf-8")
            environment = os.environ.copy()
            environment.update(APPDATA=str(appdata), LOCALAPPDATA=str(root / device / "localappdata"), XDG_DATA_HOME=str(appdata), XDG_CONFIG_HOME=str(root / device / "config"),
                               HOME=str(home), USERPROFILE=str(home),
                               SAVESYNC_SOCKET=namespace + "-" + device, SAVESYNC_STEAM_ROOT=str(root / "empty-steam"))
            for key in ("SAVESYNC_GOOGLE_CLIENT_ID", "SAVESYNC_GOOGLE_CLIENT_SECRET", "SAVESYNC_YANDEX_CLIENT_ID", "SAVESYNC_YANDEX_CLIENT_SECRET"):
                environment.pop(key, None)
            environments[device] = environment

        def command(device, *args, expect=0, parse=True):
            result = subprocess.run([str(binary), *args], env=environments[device],
                                    capture_output=True, text=True, encoding="utf-8", timeout=20)
            assert result.returncode == expect, (args, result.returncode, result.stdout, result.stderr)
            return json.loads(result.stdout) if parse else result.stdout

        def save(device, content, filename="slot.sav"):
            (root / device / "home/saves" / filename).write_text(content, encoding="utf-8")

        def outcome(device, expected, *args, exit_code=0):
            payload = command(device, *args, expect=exit_code)
            assert payload["data"]["results"][0]["outcome"] == expected, payload
            return payload

        command("pc", "remote", "login", "google", expect=1, parse=False)
        scan = command("pc", "scan")
        assert scan["data"]["found"] == 1, scan
        outcome("pc", "uploaded", "sync", "smoke-game")
        outcome("deck", "applied", "sync", "smoke-game")
        assert (root / "deck/home/saves/slot.sav").read_text() == "pc-original"
        assert list((root / "deck/appdata/savesync/state/backups").rglob("slot.sav"))
        outcome("deck", "up_to_date", "sync", "smoke-game")

        save("pc", "pc-branch")
        outcome("pc", "uploaded", "sync", "smoke-game")
        save("deck", "deck-branch")
        outcome("deck", "conflict", "sync", "smoke-game", exit_code=1)
        history = command("deck", "history", "smoke-game")["data"]
        assert len(history["heads"]) == 2, history
        selected = next(version["id"] for version in history["versions"]
                        if version["id"] in history["heads"] and version["device"] == "pc")
        outcome("deck", "applied", "restore", "smoke-game", selected)
        outcome("deck", "up_to_date", "sync", "smoke-game")
        assert len(command("deck", "history", "smoke-game")["data"]["heads"]) == 1
        outcome("pc", "applied", "sync", "smoke-game")
        save("deck", "new slot", "slot2.sav")
        outcome("deck", "uploaded", "sync", "smoke-game")
        outcome("pc", "applied", "sync", "smoke-game")
        assert (root / "pc/home/saves/slot2.sav").read_text() == "new slot"
        history = command("pc", "history", "smoke-game")["data"]
        oldest = min(history["versions"], key=lambda version: version["created_at"])["id"]
        outcome("pc", "applied", "restore", "smoke-game", oldest)
        assert not (root / "pc/home/saves/slot2.sav").exists()
        outcome("pc", "up_to_date", "sync", "smoke-game")
        outcome("deck", "applied", "sync", "smoke-game")
        assert not (root / "deck/home/saves/slot2.sav").exists()
        command("pc", "backup", "create", "smoke-game", parse=False)
        backups = list((root / "pc/backups").glob("*.savesync-backup"))
        assert backups
        command("pc", "backup", "import", str(backups[0]), parse=False)
        command("pc", "custom", "add", "Custom Test", "--path", "<home>/saves/*.sav", parse=False)
        outcome("pc", "uploaded", "sync", "custom:custom-test")
        command("pc", "manifests", "import", str(manifests), parse=False)
        assert "custom:custom-test" in command("pc", "custom", "list", parse=False)
        sentinel = root / "pc/appdata/savesync/state/sentinel.yaml"
        sentinel.write_text("must survive custom remove", encoding="utf-8")
        for slug in ("../sentinel", "custom:../sentinel", "..\\sentinel", str(sentinel.with_suffix(""))):
            command("pc", "custom", "remove", slug, expect=2, parse=False)
            assert sentinel.read_text(encoding="utf-8") == "must survive custom remove"
        command("pc", "custom", "add", "Disposable Rule", "--path", "<home>/saves/*.sav", parse=False)
        command("pc", "custom", "remove", "custom:disposable-rule", parse=False)

        # A separate namespace avoids touching any real user's running daemon.
        log = root / "daemon.log"
        with log.open("w", encoding="utf-8") as output:
            daemon = subprocess.Popen([str(binary), "serve"], env=environments["pc"],
                                      stdout=output, stderr=output,
                                      creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            try:
                for _ in range(50):
                    if daemon.poll() is not None:
                        raise AssertionError(log.read_text(encoding="utf-8"))
                    result = subprocess.run([str(binary), "status"], env=environments["pc"],
                                            capture_output=True, text=True, encoding="utf-8", timeout=5)
                    if result.returncode == 0:
                        break
                    time.sleep(0.1)
                else:
                    raise AssertionError("isolated IPC did not become ready")
                command("pc", "remote", "login", "yandex", expect=1, parse=False)
                command("pc", "remote", "setup", "fs", str(root / "remote"))
                status = json.loads(result.stdout)
                assert status["data"]["device_id"] == "pc"
                assert len(status["data"]["games"]) == 2
                outcome("pc", "up_to_date", "sync", "smoke-game")
                assert command("pc", "history", "smoke-game")["data"]["versions"]
                # Real binary enrollment and sync over QUIC, isolated from user LAN trust.
                command("pc", "remote", "setup", "lan", "0", "unused")
                pc_lan = command("pc", "remote", "lan", "status")["data"]
                invitation = command("pc", "remote", "lan", "invite")["data"]["invitation_pin"]
                deck_log = root / "deck-daemon.log"
                with deck_log.open("w", encoding="utf-8") as deck_output:
                    deck_daemon = subprocess.Popen([str(binary), "serve"], env=environments["deck"],
                                                  stdout=deck_output, stderr=deck_output,
                                                  creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                    try:
                        for _ in range(50):
                            ready = subprocess.run([str(binary), "status"], env=environments["deck"], capture_output=True, timeout=5)
                            if ready.returncode == 0:
                                break
                            time.sleep(0.1)
                        else:
                            raise AssertionError(deck_log.read_text(encoding="utf-8"))
                        offer = command("deck", "remote", "lan", "pair", f"127.0.0.1:{pc_lan['port']}", invitation)["data"]["offer"]
                        pending = command("pc", "remote", "lan", "status")["data"]["peers"]
                        assert len(pending) == 1 and pending[0]["sas"] == offer["sas"]
                        command("deck", "remote", "lan", "confirm", offer["peer_id"])
                        command("pc", "remote", "lan", "confirm", pending[0]["peer"]["id"])
                        command("pc", "remote", "setup", "fs-off")
                        command("deck", "remote", "setup", "fs-off")
                        save("pc", "progress over authenticated QUIC")
                        command("pc", "sync", "smoke-game")
                        command("deck", "sync", "smoke-game")
                        assert (root / "deck/home/saves/slot.sav").read_text() == "progress over authenticated QUIC"
                        command("deck", "stop")
                        assert deck_daemon.wait(timeout=5) == 0
                    finally:
                        if deck_daemon.poll() is None:
                            deck_daemon.terminate()
                        deck_daemon.wait(timeout=5)
                second = subprocess.run([str(binary), "serve"], env=environments["pc"],
                                        capture_output=True, timeout=5)
                assert second.returncode != 0, "second daemon unexpectedly acquired the IPC socket"
                command("pc", "stop")
                assert daemon.wait(timeout=5) == 0
                command("pc", "status", expect=1, parse=False)
            finally:
                if daemon.poll() is None:
                    daemon.terminate()
                daemon.wait(timeout=5)
        print("PASS: CLI scan, two-device sync, safety backup, conflict resolution, rescan,")
        print("      portable backup/import, Windows custom keys, isolated IPC, second-instance guard, paired QUIC LAN")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/debug/savesync-daemon" + (".exe" if os.name == "nt" else "")))
    run(parser.parse_args().binary.resolve())
