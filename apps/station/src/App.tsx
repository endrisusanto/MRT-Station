import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import { api } from "./api";
import type {
  AgentStatus,
  Device,
  OperationKind,
  OperationStatus,
  Session,
  TokenMode
} from "./types";

const terminalStates = new Set(["completed", "failed", "cancelled"]);

function formatError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Unexpected error";
}

export default function App() {
  const [agent, setAgent] = useState<AgentStatus | null>(null);
  const [session, setSession] = useState<Session | null>(null);
  const [devices, setDevices] = useState<Device[]>([]);
  const [modes, setModes] = useState<TokenMode[]>([]);
  const [selectedDevices, setSelectedDevices] = useState<string[]>([]);
  const [selectedModes, setSelectedModes] = useState<string[]>([]);
  const [expiry, setExpiry] = useState("");
  const [operation, setOperation] = useState<OperationStatus | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");

  const refresh = useCallback(async () => {
    setError("");
    try {
      const status = await api.getAgentStatus();
      setAgent(status);
      const [currentSession, foundDevices] = await Promise.all([
        api.getSession(),
        api.listDevices()
      ]);
      setSession(currentSession);
      setDevices(foundDevices);
      setModes(currentSession ? await api.getPermissions() : []);
    } catch (cause) {
      setAgent(null);
      setError(`Agent unavailable: ${formatError(cause)}`);
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 10_000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  useEffect(() => {
    if (!operation || terminalStates.has(operation.state)) return;
    const timer = window.setInterval(async () => {
      try {
        setOperation(await api.getOperation(operation.id));
      } catch (cause) {
        setError(formatError(cause));
      }
    }, 400);
    return () => window.clearInterval(timer);
  }, [operation?.id, operation?.state]);

  const selectedDeviceModels = useMemo(
    () => devices.filter((device) => selectedDevices.includes(device.id)),
    [devices, selectedDevices]
  );

  async function handleLogin(username: string, password: string) {
    setBusy(true);
    setError("");
    try {
      const nextSession = await api.login(username, password);
      setSession(nextSession);
      setModes(await api.getPermissions());
    } catch (cause) {
      setError(formatError(cause));
    } finally {
      setBusy(false);
    }
  }

  async function handleLogout() {
    await api.logout();
    setSession(null);
    setModes([]);
    setSelectedModes([]);
  }

  async function runOperation(kind: OperationKind) {
    setError("");
    try {
      setOperation(
        await api.startOperation(
          kind,
          selectedDevices,
          selectedModes,
          expiry ? new Date(`${expiry}T23:59:59`).toISOString() : null
        )
      );
    } catch (cause) {
      setError(formatError(cause));
    }
  }

  if (busy && !agent) {
    return <main className="center"><div className="spinner" /><p>Connecting to EM Agent...</p></main>;
  }

  if (!agent) {
    return (
      <main className="center">
        <section className="empty-card">
          <span className="status-dot danger" />
          <h1>EM Agent unavailable</h1>
          <p>Start the EM Agent service, then reconnect.</p>
          {error && <div className="error">{error}</div>}
          <button onClick={() => void refresh()}>Reconnect</button>
        </section>
      </main>
    );
  }

  return (
    <div className="app-shell">
      <header>
        <div><span className="brand-mark">EM</span><strong>Station</strong></div>
        <div className="agent-pill"><span className="status-dot" />Agent {agent.version}</div>
      </header>

      <aside>
        <div className="profile">
          <div className="avatar">{session?.displayName.slice(0, 2).toUpperCase() ?? "?"}</div>
          {session ? (
            <><strong>{session.displayName}</strong><small>Session {Math.ceil(session.remainingSeconds / 60)} min</small><button className="link" onClick={() => void handleLogout()}>Log out</button></>
          ) : <><strong>Not signed in</strong><small>AD session required</small></>}
        </div>
        <nav><span className="active">Devices</span><span>Operations</span><span>Diagnostics</span></nav>
      </aside>

      <main>
        {error && <div className="error banner">{error}<button onClick={() => setError("")}>Dismiss</button></div>}
        {!session ? <LoginCard onLogin={handleLogin} busy={busy} /> : (
          <>
            <section className="section-heading">
              <div><p className="eyebrow">Connected hardware</p><h1>Choose devices</h1><p>Select one or more targets for the token operation.</p></div>
              <button className="secondary" onClick={() => void refresh()}>Refresh</button>
            </section>
            <section className="device-grid">
              {devices.map((device) => {
                const selected = selectedDevices.includes(device.id);
                return <button key={device.id} className={`device-card ${selected ? "selected" : ""}`} onClick={() => setSelectedDevices((items) => selected ? items.filter((id) => id !== device.id) : [...items, device.id])}>
                  <span className={`device-icon ${device.transport}`}>{device.transport === "usb" ? "USB" : "COM"}</span>
                  <span><strong>{device.displayName}</strong><small>{device.model} · {device.serialNumber}</small><small>{device.port} · {device.firmware}</small></span>
                  <span className="check">{selected ? "✓" : ""}</span>
                </button>;
              })}
            </section>

            <section className="workspace">
              <div>
                <p className="eyebrow">Token configuration</p>
                <h2>Modes and expiry</h2>
                <div className="mode-list">
                  {modes.filter((mode) => mode.permitted).map((mode) => <label key={mode.id}><input type="checkbox" checked={selectedModes.includes(mode.id)} onChange={(event) => setSelectedModes((items) => event.target.checked ? [...items, mode.id] : items.filter((id) => id !== mode.id))} /><span><strong>{mode.displayName}</strong><small>{mode.description}</small></span></label>)}
                </div>
                <label className="field"><span>Expiry date</span><input type="date" value={expiry} min={new Date().toISOString().slice(0, 10)} onChange={(event) => setExpiry(event.target.value)} /></label>
              </div>
              <div className="action-panel">
                <p>{selectedDeviceModels.length} device(s) selected</p>
                <button disabled={!selectedDevices.length || !selectedModes.length || !!operation && !terminalStates.has(operation.state)} onClick={() => void runOperation("install")}>Install tokens</button>
                <button className="secondary" disabled={!selectedDevices.length} onClick={() => void runOperation("token_info")}>Read token info</button>
                <div className="split-actions"><button className="ghost" disabled={!selectedDevices.length} onClick={() => void runOperation("remove")}>Remove</button><button className="ghost" disabled={!selectedDevices.length} onClick={() => void runOperation("recover")}>Recover ESI</button></div>
              </div>
            </section>
          </>
        )}
      </main>
      {operation && <OperationPanel operation={operation} onCancel={async () => setOperation(await api.cancelOperation(operation.id))} onClose={() => setOperation(null)} />}
    </div>
  );
}

function LoginCard({ onLogin, busy }: { onLogin: (username: string, password: string) => Promise<void>; busy: boolean }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  function submit(event: FormEvent) {
    event.preventDefault();
    void onLogin(username, password).finally(() => setPassword(""));
  }
  return <section className="login-card"><p className="eyebrow">Secure access</p><h1>Sign in with Samsung AD</h1><p>Your password is sent directly to the local agent and is never stored by the interface.</p><form onSubmit={submit}><label className="field"><span>AD username</span><input autoFocus autoComplete="username" value={username} onChange={(event) => setUsername(event.target.value)} /></label><label className="field"><span>Password</span><input type="password" autoComplete="current-password" value={password} onChange={(event) => setPassword(event.target.value)} /></label><button disabled={busy || !username || !password}>{busy ? "Signing in..." : "Sign in"}</button></form></section>;
}

function OperationPanel({ operation, onCancel, onClose }: { operation: OperationStatus; onCancel: () => void; onClose: () => void }) {
  const terminal = terminalStates.has(operation.state);
  const progress = operation.total ? Math.round(operation.completed / operation.total * 100) : 0;
  return <div className="modal-backdrop"><section className="operation-panel"><p className="eyebrow">{operation.kind.replace("_", " ")}</p><h2>{terminal ? `Operation ${operation.state}` : "Operation in progress"}</h2><div className="progress"><span style={{ width: `${progress}%` }} /></div><p>{operation.completed} of {operation.total} devices complete</p><div className="result-list">{operation.results.map((result) => <div key={result.deviceId} className={result.success ? "result-ok" : "result-bad"}><strong>{result.deviceId}</strong><span>{result.message}</span>{result.tokenId && <code>{result.tokenId}</code>}</div>)}</div>{terminal ? <button onClick={onClose}>Done</button> : <button className="secondary" onClick={onCancel}>Cancel operation</button>}</section></div>;
}

