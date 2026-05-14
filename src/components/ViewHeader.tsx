import type { ReactNode } from "react";
import { ArrowLeftIcon } from "./Icons";

export function ViewHeader(props: {
  title: string;
  subtitle: ReactNode;
  action?: ReactNode;
  onBack?: () => void;
}) {
  const { title, subtitle, action, onBack } = props;
  return (
    <header className="view-header">
      <div className="view-title-row">
        {onBack ? (
          <button type="button" className="back-button" onClick={onBack} aria-label="Back">
            <ArrowLeftIcon />
          </button>
        ) : null}
        <div>
          <h1>{title}</h1>
          <p className="muted">{subtitle}</p>
        </div>
      </div>
      {action ? <div className="view-actions">{action}</div> : null}
    </header>
  );
}
