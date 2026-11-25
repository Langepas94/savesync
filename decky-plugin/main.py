"""SaveSync Decky backend: тонкий прокси к savesync-daemon по unix-сокету.

Протокол — тот же JSON-RPC, что и у desktop-приложения
(crates/core/src/ipc.rs): одна строка JSON на запрос, одна на ответ.
На Linux interprocess::GenericNamespaced использует abstract namespace
сокет, поэтому адрес — "\\0savesync-daemon".

Фронтенд (index.tsx) вызывает методы класса Plugin через decky.call.
Никакой логики синхронизации здесь нет — только перекладка запросов.
"""

import asyncio
import json
import socket
import os
import struct

SOCKET_ADDRESS = "\0savesync-daemon"
RPC_TIMEOUT = 30  # sync большой библиотеки может занять секунды


def _allowed_peer_uid(current: int, peer: int, deck_uid: int | None = None) -> bool:
    return peer == current or peer == 0 or (current == 0 and peer == deck_uid)


def _authorize_peer(sock) -> None:
    """Authenticate the daemon before sending a pairing PIN to an abstract socket."""
    credentials = sock.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i"))
    _, uid, _ = struct.unpack("3i", credentials)
    deck_uid = None
    if os.geteuid() == 0 and uid != 0:
        try:
            import decky
            deck_uid = os.stat(decky.DECKY_USER_HOME).st_uid
        except (ImportError, AttributeError, OSError):
            try:
                deck_uid = os.stat("/home/deck").st_uid
            except OSError:
                pass
    if not _allowed_peer_uid(os.geteuid(), uid, deck_uid):
        raise PermissionError("IPC daemon does not belong to the Deck user")


def _rpc(method: str, params: dict | None = None) -> dict:
    """Один запрос — одно соединение — один ответ (как ipc::client::call)."""
    request = {"id": 1, "method": method}
    if params is not None:
        request["params"] = params

    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.settimeout(RPC_TIMEOUT)
        sock.connect(SOCKET_ADDRESS)
        _authorize_peer(sock)
        sock.sendall((json.dumps(request) + "\n").encode("utf-8"))

        buf = b""
        while not buf.endswith(b"\n"):
            chunk = sock.recv(1 << 16)
            if not chunk:
                raise RuntimeError("демон закрыл соединение без ответа")
            buf += chunk

    response = json.loads(buf.decode("utf-8"))
    if response.get("error"):
        raise RuntimeError(response["error"]["message"])
    return response.get("result") or {}


class Plugin:
    """Точки входа для фронтенда (decky.call("имя", ...))."""

    async def get_status(self) -> dict:
        """Статус демона и игр; daemon_ok=False, если демон не поднят."""
        try:
            payload = await asyncio.to_thread(_rpc, "status")
            return {"daemon_ok": True, "status": payload.get("data") or {}}
        except Exception as exc:  # демон не запущен / сокет недоступен
            return {"daemon_ok": False, "error": str(exc)}

    async def sync_all(self) -> dict:
        """Ручная синхронизация всех игр (MVP-триггер)."""
        payload = await asyncio.to_thread(_rpc, "sync", {"game_key": None})
        return payload.get("data") or {}

    async def sync_game(self, game_key: str) -> dict:
        """Ручная синхронизация одной игры."""
        payload = await asyncio.to_thread(_rpc, "sync", {"game_key": game_key})
        return payload.get("data") or {}

    async def lan_action(self, action: dict) -> dict:
        """Pairing stays in the daemon; QAM only forwards explicit user choices."""
        payload = await asyncio.to_thread(_rpc, "lan", {"action": action})
        if payload.get("type") != "lan" or not isinstance(payload.get("data"), dict):
            raise RuntimeError("daemon returned an incompatible LAN response")
        data = payload["data"]
        if data.get("error"):
            raise RuntimeError(data["error"])
        return data

    async def _main(self):
        # Точка входа Decky Loader: плагин загружен, ничего не поднимаем —
        # прокси работает по требованию, без фоновых задач.
        pass

    async def _unload(self):
        pass
