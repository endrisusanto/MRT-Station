export type AgentStatus = {
  installed: boolean;
  running: boolean;
  version: string;
  protocolVersion: number;
  compatible: boolean;
  updateAvailable: boolean;
};

export type Device = {
  id: string;
  displayName: string;
  model: string;
  serialNumber: string;
  firmware: string;
  transport: "usb" | "serial" | "wifi";
  mode: "normal" | "download";
  connected: boolean;
  port?: string;
  vid?: number;
  pid?: number;
};

export type Session = {
  userId: string;
  displayName: string;
  expiresAt: string;
  remainingSeconds: number;
};

export type TokenMode = {
  id: string;
  displayName: string;
  description: string;
  permitted: boolean;
  attributes: Record<string, string>;
};

export type OperationKind = "token_info" | "install" | "remove" | "recover";
export type OperationState = "queued" | "running" | "completed" | "failed" | "cancelled";

export type AppError = {
  code: string;
  message: string;
  retryable: boolean;
  legacyCode?: number;
};

export type DeviceResult = {
  deviceId: string;
  success: boolean;
  message: string;
  tokenId?: string;
  error?: AppError;
};

export type OperationStatus = {
  id: string;
  kind: OperationKind;
  state: OperationState;
  completed: number;
  total: number;
  startedAt: string;
  finishedAt?: string;
  results: DeviceResult[];
};

