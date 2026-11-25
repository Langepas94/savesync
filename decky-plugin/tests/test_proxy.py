"""Тесты JSON-RPC прокси Decky (decky-plugin/main.py).

Без Decky Loader и демона: поднимаем фейковый сервер на unix-сокете
во временном пути и проверяем формат запросов/ответов протокола
(crates/core/src/ipc.rs). Запуск: python -m unittest discover tests.
"""

import importlib.util
import json
import socket
import threading
import unittest
from pathlib import Path

PLUGIN_DIR = Path(__file__).resolve().parent.parent

if not hasattr(socket, "AF_UNIX"):
    raise unittest.SkipTest("AF_UNIX недоступен на этой платформе (цель — SteamOS/Linux)")

spec = importlib.util.spec_from_file_location("savesync_proxy", PLUGIN_DIR / "main.py")
proxy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proxy)


def serve_once(path: Path, response: dict, captured: list) -> threading.Thread:
    def _run():
        srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        srv.bind(str(path))
        srv.listen(1)
        conn, _ = srv.accept()
        data = b""
        while not data.endswith(b"\n"):
            chunk = conn.recv(1 << 16)
            if not chunk:
                break
            data += chunk
        captured.append(json.loads(data.decode("utf-8")))
        conn.sendall((json.dumps(response) + "\n").encode("utf-8"))
        conn.close()
        srv.close()

    thread = threading.Thread(target=_run, daemon=True)
    thread.start()
    return thread


class RpcTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(self.id().replace(".", "_")).with_suffix(".sock")
        self.sock_path = Path.cwd() / self.tmp
        self.addCleanup(lambda: self.sock_path.unlink(missing_ok=True))
        self._orig = proxy.SOCKET_ADDRESS
        proxy.SOCKET_ADDRESS = str(self.sock_path)
        self.addCleanup(setattr, proxy, "SOCKET_ADDRESS", self._orig)

    def test_status_request_shape(self):
        captured: list = []
        serve_once(
            self.sock_path,
            {"id": 1, "result": {"type": "status", "data": {"games": []}}},
            captured,
        )
        result = proxy._rpc("status")
        self.assertEqual(result["data"], {"games": []})
        # unit-метод сериализуется без поля params (см. ipc.rs serde tag/content)
        self.assertEqual(captured[0], {"id": 1, "method": "status"})

    def test_sync_sends_params_and_game_key(self):
        captured: list = []
        serve_once(
            self.sock_path,
            {"id": 1, "result": {"type": "sync", "data": {"results": []}}},
            captured,
        )
        proxy._rpc("sync", {"game_key": "elden-ring"})
        self.assertEqual(
            captured[0],
            {"id": 1, "method": "sync", "params": {"game_key": "elden-ring"}},
        )

    def test_error_response_raises_with_message(self):
        serve_once(
            self.sock_path,
            {"id": 1, "error": {"message": "backend offline"}},
            [],
        )
        with self.assertRaises(RuntimeError) as ctx:
            proxy._rpc("status")
        self.assertIn("backend offline", str(ctx.exception))

    def test_daemon_down_reported_not_thrown(self):
        # Сокета нет: get_status обязан вернуть daemon_ok=False, а не упасть.
        async def run():
            plugin = proxy.Plugin()
            return await plugin.get_status()

        import asyncio

        result = asyncio.run(run())
        self.assertFalse(result["daemon_ok"])
        self.assertIn("error", result)


if __name__ == "__main__":
    unittest.main()
