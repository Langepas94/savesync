import React, { useEffect, useRef, useState, VFC } from "react";
import { callable } from "@decky/api";
import { ButtonItem, PanelSection, PanelSectionRow, TextField } from "@decky/ui";

type Action = { action: "status" | "discover" | "invite" | "close_pairing" }
  | { action: "pair"; address: string; pin: string }
  | { action: "confirm" | "reject" | "forget"; peer_id: string };

interface Peer {
  id: string;
  address: string | null;
  name?: string;
}

interface DiscoveredPeer {
  identity: string;
  name: string;
  address: string;
}

interface LanState {
  port: number | null;
  invitation_pin: string | null;
  invitation_addresses?: string[];
  peers: { peer: Peer; confirmed: boolean; sas: string | null }[];
  discovered: DiscoveredPeer[];
}

const request = callable<[Action], LanState>("lan_action");

function label(peer: Peer) {
  return [peer.name, peer.address].filter(Boolean).join(" · ") || `Устройство ${peer.id.slice(0, 12)}`;
}

export const LanPanel: VFC = () => {
  const [state, setState] = useState<LanState | null>(null);
  const [found, setFound] = useState<DiscoveredPeer[] | null>(null);
  const [pending, setPending] = useState<Action["action"] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [address, setAddress] = useState("");
  const [pin, setPin] = useState("");
  const revision = useRef(0);
  const busy = pending !== null;

  const act = async (action: Action) => {
    if (busy) return;
    revision.current += 1;
    setPending(action.action);
    setError(null);
    if (action.action === "discover") setFound(null);
    try {
      const result = await request(action);
      setState(result);
      if (action.action === "discover") setFound(result.discovered);
      if (action.action === "pair") setPin("");
    } catch (failure) {
      setError(String(failure));
    } finally {
      setPending(null);
    }
  };

  useEffect(() => {
    if (busy) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const current = revision.current;
      try {
        const result = await request({ action: "status" });
        if (!disposed && current === revision.current) {
          setState(result);
          setLoadError(null);
        }
      } catch (failure) {
        if (!disposed && current === revision.current && state === null) setLoadError(String(failure));
      } finally {
        if (!disposed && current === revision.current) timer = setTimeout(poll, 2000);
      }
    };
    if (state === null) void poll();
    else timer = setTimeout(poll, 2000);
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [busy, state === null]);

  const trusted = state?.peers.filter(entry => entry.confirmed) ?? [];
  const waiting = state?.peers.filter(entry => !entry.confirmed) ?? [];

  return <PanelSection title="Синхронизация по локальной сети">
    {error && <PanelSectionRow><div style={{ color: "#d5463a" }}>{error}</div></PanelSectionRow>}
    {loadError && <PanelSectionRow><div style={{ color: "#d5463a" }}>{loadError}</div></PanelSectionRow>}
    <PanelSectionRow><div>Доверенные устройства</div></PanelSectionRow>
    {state === null && !loadError && <PanelSectionRow><div>Загрузка списка…</div></PanelSectionRow>}
    {state && trusted.length === 0 && <PanelSectionRow><div>Доверенных устройств пока нет.</div></PanelSectionRow>}
    {trusted.map(({ peer }) => {
      const discovered = !peer.name && found?.find(device => device.identity === peer.id);
      return <React.Fragment key={peer.id}>
        <PanelSectionRow><div>{discovered ? `${discovered.name} (имя в сети)` : label(peer)}</div></PanelSectionRow>
        <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "forget", peer_id: peer.id })}>Забыть устройство</ButtonItem></PanelSectionRow>
      </React.Fragment>;
    })}
    {state && !state.port && <PanelSectionRow><div>Включите LAN в «Подключениях» в режиме рабочего стола.</div></PanelSectionRow>}
    {state?.port && <>
      {waiting.map(({ peer, sas }) => <React.Fragment key={peer.id}>
        <PanelSectionRow><div>{label(peer)} · Код проверки: {sas}</div></PanelSectionRow>
        <PanelSectionRow><div>Сравните код и подтвердите на обоих устройствах.</div></PanelSectionRow>
        <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "confirm", peer_id: peer.id })}>Коды совпадают</ButtonItem></PanelSectionRow>
        <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "reject", peer_id: peer.id })}>Отклонить</ButtonItem></PanelSectionRow>
      </React.Fragment>)}
      <PanelSectionRow><div>Добавить устройство</div></PanelSectionRow>
      {state.invitation_pin ? <>
        <PanelSectionRow><div>PIN: {state.invitation_pin}. Введите на втором устройстве в течение двух минут.</div></PanelSectionRow>
        {(state.invitation_addresses ?? []).map(localAddress => <PanelSectionRow key={localAddress}><div>{localAddress}</div></PanelSectionRow>)}
        <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "close_pairing" })}>Закрыть приглашение</ButtonItem></PanelSectionRow>
      </> : <>
        <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "invite" })}>Показать мой PIN</ButtonItem></PanelSectionRow>
        <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "discover" })}>{pending === "discover" ? "Поиск устройств…" : "Найти устройства"}</ButtonItem></PanelSectionRow>
        {found?.length === 0 && <PanelSectionRow><div>Устройства не найдены. Проверьте сеть или введите адрес вручную.</div></PanelSectionRow>}
        {found?.map(peer => {
          const known = state.peers.find(entry => entry.peer.id === peer.identity);
          const name = `${peer.name} · ${peer.address}`;
          return <PanelSectionRow key={peer.identity + peer.address}>
            {known ? <div>{name} · {known.confirmed ? "Доверенное" : "Ожидает подтверждения"}</div>
              : <ButtonItem layout="below" disabled={busy} onClick={() => setAddress(peer.address)}>{name}</ButtonItem>}
          </PanelSectionRow>;
        })}
        <PanelSectionRow><TextField label="Адрес второго устройства (IP:порт)" value={address} onChange={event => setAddress(event.target.value)} disabled={busy} /></PanelSectionRow>
        <PanelSectionRow><TextField label="PIN второго устройства" value={pin} onChange={event => setPin(event.target.value)} bIsPassword mustBeNumeric disabled={busy} /></PanelSectionRow>
        <PanelSectionRow><ButtonItem layout="below" disabled={busy || !address.trim() || !/^\d{6}$/.test(pin)} onClick={() => act({ action: "pair", address: address.trim(), pin })}>Сопрячь</ButtonItem></PanelSectionRow>
      </>}
    </>}
  </PanelSection>;
};
