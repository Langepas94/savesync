import React, { useEffect, useState, VFC } from "react";
import { callable } from "@decky/api";
import { ButtonItem, PanelSection, PanelSectionRow, TextField } from "@decky/ui";

type Action = { action: "status" | "discover" | "invite" | "close_pairing" }
  | { action: "pair"; address: string; pin: string }
  | { action: "confirm" | "forget"; peer_id: string };
interface LanState {
  port: number | null;
  invitation_pin: string | null;
  peers: { peer: { id: string; address: string | null }; confirmed: boolean; sas: string | null }[];
  discovered: { identity: string; name: string; address: string }[];
}
const request = callable<[Action], LanState>("lan_action");

export const LanPanel: VFC = () => {
  const [state, setState] = useState<LanState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [address, setAddress] = useState("");
  const [pin, setPin] = useState("");
  const act = async (action: Action) => {
    if (busy) return;
    setBusy(true); setError(null);
    try { setState(await request(action)); }
    catch (failure) { setError(String(failure)); }
    finally { setBusy(false); }
  };
  useEffect(() => { void act({ action: "status" }); }, []);
  return <PanelSection title="LAN · доверенные устройства">
    {error && <PanelSectionRow><div style={{ color: "#d5463a", fontSize: "12px" }}>{error}</div></PanelSectionRow>}
    {!state?.port && <PanelSectionRow><div style={{ fontSize: "12px" }}>Сначала включите LAN в Desktop Mode. Облако для сопряжения не требуется.</div></PanelSectionRow>}
    <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "invite" })}>Показать PIN на 2 минуты</ButtonItem></PanelSectionRow>
    {state?.invitation_pin && <>
      <PanelSectionRow><div>PIN: {state.invitation_pin}</div></PanelSectionRow>
      <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "close_pairing" })}>Закрыть приглашение</ButtonItem></PanelSectionRow>
    </>}
    <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "discover" })}>Найти устройства</ButtonItem></PanelSectionRow>
    {(state?.discovered ?? []).map(peer => <PanelSectionRow key={peer.identity + peer.address}><ButtonItem layout="below" disabled={busy} onClick={() => setAddress(peer.address)}>{peer.name} · {peer.address}</ButtonItem></PanelSectionRow>)}
    <PanelSectionRow><TextField label="Адрес host:port" value={address} onChange={event => setAddress(event.target.value)} disabled={busy} /></PanelSectionRow>
    <PanelSectionRow><TextField label="PIN другого устройства" value={pin} onChange={event => setPin(event.target.value)} bIsPassword mustBeNumeric disabled={busy} /></PanelSectionRow>
    <PanelSectionRow><ButtonItem layout="below" disabled={busy || !address.trim() || !/^\d{6}$/.test(pin)} onClick={() => { const code = pin; setPin(""); void act({ action: "pair", address: address.trim(), pin: code }); }}>Сопрячь</ButtonItem></PanelSectionRow>
    <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "status" })}>Обновить SAS / доверие</ButtonItem></PanelSectionRow>
    {(state?.peers ?? []).map(entry => <React.Fragment key={entry.peer.id}>
      <PanelSectionRow><div style={{ fontSize: "12px" }}>{entry.confirmed ? "Доверенное устройство" : `SAS: ${entry.sas ?? ""}`} · {entry.peer.id.slice(0, 12)}</div></PanelSectionRow>
      {!entry.confirmed && <>
        <PanelSectionRow><div style={{ fontSize: "12px" }}>Сравните SAS на обоих устройствах. Подтверждайте только при совпадении.</div></PanelSectionRow>
        <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "confirm", peer_id: entry.peer.id })}>SAS совпадает — доверять</ButtonItem></PanelSectionRow>
      </>}
      <PanelSectionRow><ButtonItem layout="below" disabled={busy} onClick={() => act({ action: "forget", peer_id: entry.peer.id })}>{entry.confirmed ? "Забыть устройство" : "Отклонить"}</ButtonItem></PanelSectionRow>
    </React.Fragment>)}
  </PanelSection>;
};
