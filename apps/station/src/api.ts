import { invoke } from "@tauri-apps/api/core";
import type {
  HealthSnapshot,
  AgentStatus,
  DiagnosticSnapshot,
  Device,
  OperationKind,
  OperationStatus,
  Session,
  TokenMode
} from "./types";

export const api = {
  getAgentStatus: () => invoke<AgentStatus>("get_agent_status"),
  listDevices: () => invoke<Device[]>("list_devices"),
  getSession: () => invoke<Session | null>("get_session"),
  login: (username: string, password: string) =>
    invoke<Session>("login", { username, password }),
  logout: () => invoke<void>("logout"),
  getPermissions: () => invoke<TokenMode[]>("get_permissions"),
  startOperation: (
    kind: OperationKind,
    deviceIds: string[],
    modeIds: string[],
    expiresAt: string | null
  ) => invoke<OperationStatus>("start_operation", { kind, deviceIds, modeIds, expiresAt }),
  getOperation: (operationId: string) =>
    invoke<OperationStatus>("get_operation", { operationId }),
  cancelOperation: (operationId: string) =>
    invoke<OperationStatus>("cancel_operation", { operationId }),
  getHealth: () => invoke<HealthSnapshot>("get_health"),
  getDiagnostics: () => invoke<DiagnosticSnapshot>("get_diagnostics")
};
