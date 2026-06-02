import { invoke } from "@tauri-apps/api/core";

export function diagnosticLog(message: string, level: "INFO" | "WARN" | "ERROR" = "INFO"): void {
  void invoke("diagnostic_log", { message, level }).catch(() => {
    /* logging must never break the app */
  });
}

export function installFrontendDiagnostics(): void {
  diagnosticLog("frontend boot");

  window.addEventListener("error", (event) => {
    const detail = event.error instanceof Error ? event.error.stack ?? event.error.message : String(event.error ?? "");
    diagnosticLog(
      `window error: ${event.message} at ${event.filename ?? "?"}:${event.lineno ?? "?"}:${event.colno ?? "?"} ${detail}`,
      "ERROR",
    );
  });

  window.addEventListener("unhandledrejection", (event) => {
    const reason = event.reason;
    const detail =
      reason instanceof Error ? (reason.stack ?? reason.message) : typeof reason === "string" ? reason : JSON.stringify(reason);
    diagnosticLog(`unhandled rejection: ${detail}`, "ERROR");
  });
}
