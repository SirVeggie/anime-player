import { useCallback, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import appIconUrl from "../../src-tauri/icons/icon.png?url";

const appWindow = getCurrentWindow();

export function WindowTitleBar(props: { playerOpen: boolean; playerControlsChromeVisible: boolean }) {
  const { playerOpen, playerControlsChromeVisible } = props;
  const [maximized, setMaximized] = useState(false);

  const refreshMaximized = useCallback(async () => {
    try {
      setMaximized(await appWindow.isMaximized());
    } catch {
      /* The title bar should never block the rest of the UI. */
    }
  }, []);

  useEffect(() => {
    void refreshMaximized();
  }, [refreshMaximized]);

  const toggleMaximized = useCallback(async () => {
    try {
      await appWindow.toggleMaximize();
      await refreshMaximized();
    } catch {
      /* ignore */
    }
  }, [refreshMaximized]);

  return (
    <header
      className={`window-titlebar${playerOpen ? " window-titlebar--player" : ""}${
        playerOpen && playerControlsChromeVisible ? " window-titlebar--player-visible" : ""
      }`}
    >
      <div className="window-titlebar-title">
        <img className="window-titlebar-icon" src={appIconUrl} alt="" aria-hidden />
        <span>Anime Player</span>
      </div>
      <div className="window-controls">
        <button type="button" className="window-control" onClick={() => void appWindow.minimize()} aria-label="Minimize" tabIndex={-1}>
          <svg viewBox="0 0 12 12" aria-hidden>
            <path d="M2 6.5h8v1H2z" />
          </svg>
        </button>
        <button
          type="button"
          className="window-control"
          onClick={() => void toggleMaximized()}
          aria-label={maximized ? "Restore" : "Maximize"}
          tabIndex={-1}
        >
          {maximized ? (
            <svg viewBox="0 0 12 12" aria-hidden>
              <path d="M3 2h7v7H8V8h1V3H4v1H3V2zm-1 3h6v5H2V5zm1 1v3h4V6H3z" />
            </svg>
          ) : (
            <svg viewBox="0 0 12 12" aria-hidden>
              <path d="M2 2h8v8H2V2zm1 1v6h6V3H3z" />
            </svg>
          )}
        </button>
        <button
          type="button"
          className="window-control window-control--close"
          onClick={() => void appWindow.close()}
          aria-label="Close"
          tabIndex={-1}
        >
          <svg viewBox="0 0 12 12" aria-hidden>
            <path d="m3.1 2.4 2.9 2.9 2.9-2.9.7.7L6.7 6l2.9 2.9-.7.7L6 6.7 3.1 9.6l-.7-.7L5.3 6 2.4 3.1l.7-.7z" />
          </svg>
        </button>
      </div>
    </header>
  );
}
