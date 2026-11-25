/**
 * SaveSync Decky frontend — шторка Quick Access (360 px).
 * Дизайн-референс: ../docs/design/decky-panel.html.
 * Состав экрана: статус демона, большая кнопка «Синхронизировать»,
 * баннер конфликта (резолвер — только в десктопном приложении),
 * список игр со статусами. Вся логика — в демоне, фронтенд только
 * вызывает методы Python-прокси (main.py).
 */
import { callable, definePlugin } from "@decky/api";
import {
  ButtonItem,
  Focusable,
  PanelSection,
  PanelSectionRow,
  staticClasses,
} from "@decky/ui";
import React, { useEffect, useState, VFC } from "react";

// ── Типы ответов (зеркало crates/core/src/ipc.rs) ───────────────────

interface GameStatus {
  game_key: string;
  display_name: string;
  versions: number;
  state: string; // ok | no_versions | conflict
}

interface StatusPayload {
  daemon_version: string;
  device_id: string;
  device_name: string;
  games: GameStatus[];
}

interface SyncResult {
  game_key: string;
  outcome: string;
  detail: string | null;
}

interface SyncPayload {
  results: SyncResult[];
}

interface StatusResponse {
  daemon_ok: boolean;
  status?: StatusPayload;
  error?: string;
}

const getStatus = callable<[], StatusResponse>("get_status");
const syncAll = callable<[], SyncPayload>("sync_all");

const STATE_ICON: Record<string, { icon: string; color: string }> = {
  ok: { icon: "✓", color: "#5cba32" },
  conflict: { icon: "⚠", color: "#d5463a" },
  no_versions: { icon: "–", color: "#8f98a0" },
};

const POLL_MS = 5000;

const Content: VFC = () => {
  const [daemonOk, setDaemonOk] = useState(false);
  const [status, setStatus] = useState<StatusPayload | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const res = await getStatus();
      setDaemonOk(res.daemon_ok);
      setStatus(res.status ?? null);
    } catch {
      setDaemonOk(false);
      setStatus(null);
    }
  };

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, POLL_MS);
    return () => clearInterval(timer);
  }, []);

  const onSyncAll = async () => {
    if (busy) return;
    setBusy(true);
    setNotice(null);
    try {
      const payload = await syncAll();
      const bad = (payload.results ?? []).filter(
        (r) => r.outcome === "error" || r.outcome === "conflict"
      );
      setNotice(
        bad.length === 0
          ? "✓ Все сейвы синхронизированы"
          : `⚠ ${bad.length} игр(ы) требуют внимания`
      );
      await refresh();
    } catch (e) {
      setNotice(`Ошибка: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const conflicts = (status?.games ?? []).filter((g) => g.state === "conflict");

  if (!daemonOk) {
    return (
      <PanelSection title="◈ SaveSync">
        <PanelSectionRow>
          <div style={{ color: "#d5463a", fontSize: "13px" }}>
            Демон не запущен. В Desktop Mode выполните:
          </div>
        </PanelSectionRow>
        <PanelSectionRow>
          <code style={{ fontSize: "12px", color: "#66c0f4" }}>
            systemctl --user start savesync
          </code>
        </PanelSectionRow>
      </PanelSection>
    );
  }

  return (
    <>
      <PanelSection title={`◈ SaveSync · ${status?.device_name ?? ""}`}>
        <PanelSectionRow>
          <ButtonItem layout="below" onClick={onSyncAll} disabled={busy}>
            {busy ? "⟳ Синхронизация…" : "⇅ Синхронизировать"}
          </ButtonItem>
        </PanelSectionRow>
        {notice && (
          <PanelSectionRow>
            <div style={{ fontSize: "12.5px", color: "#c7d5e0" }}>{notice}</div>
          </PanelSectionRow>
        )}
        {conflicts.length > 0 && (
          <PanelSectionRow>
            <div
              style={{
                padding: "10px",
                borderRadius: "8px",
                background: "rgba(213,70,58,.14)",
                border: "1px solid rgba(213,70,58,.5)",
                fontSize: "12.5px",
                color: "#fff",
                width: "100%",
              }}
            >
              ⚠ Конфликт: {conflicts.map((g) => g.display_name).join(", ")}
              <div style={{ fontSize: "11.5px", color: "#8f98a0", marginTop: 2 }}>
                Разрешается в десктопном приложении
              </div>
            </div>
          </PanelSectionRow>
        )}
      </PanelSection>

      <PanelSection title="Последние сейвы">
        {(status?.games ?? []).map((g) => {
          const st = STATE_ICON[g.state] ?? STATE_ICON.no_versions;
          return (
            <PanelSectionRow key={g.game_key}>
              <Focusable
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "10px",
                  minHeight: "44px",
                }}
              >
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div
                    style={{
                      fontSize: "13.5px",
                      fontWeight: 600,
                      whiteSpace: "nowrap",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                  >
                    {g.display_name}
                  </div>
                  <div style={{ fontSize: "11.5px", color: "#8f98a0" }}>
                    версий: {g.versions}
                  </div>
                </div>
                <span style={{ fontSize: "17px", color: st.color }}>{st.icon}</span>
              </Focusable>
            </PanelSectionRow>
          );
        })}
        {(status?.games ?? []).length === 0 && (
          <PanelSectionRow>
            <div style={{ fontSize: "12px", color: "#8f98a0" }}>
              Игры не найдены: savesync-daemon scan
            </div>
          </PanelSectionRow>
        )}
      </PanelSection>
    </>
  );
};

export default definePlugin(() => ({
  name: "SaveSync",
  titleView: <div className={staticClasses.Title}>SaveSync</div>,
  content: <Content />,
  icon: <span>⇅</span>,
}));
