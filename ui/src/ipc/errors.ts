import type { IpcError } from "@/bindings";

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "kind" in value && "message" in value;
}

/** A user-presentable message for anything a command can reject with. */
export function describeError(error: unknown): string {
  if (isIpcError(error)) return error.message;
  if (error instanceof Error) return error.message;
  return String(error);
}
