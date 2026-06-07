export function ConfirmModal(props: {
  title: string;
  description: string;
  warning?: string;
  confirmLabel?: string;
  busy?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const {
    title,
    description,
    warning,
    confirmLabel = "Delete",
    busy = false,
    onConfirm,
    onClose,
  } = props;

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onClose();
      }}
    >
      <section
        className="modal delete-confirm-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-modal-title"
        aria-describedby="confirm-modal-description"
      >
        <div className="modal-heading">
          <div>
            <h2 id="confirm-modal-title">{title}</h2>
            <p className="muted" id="confirm-modal-description">
              {description}
            </p>
          </div>
        </div>
        {warning ? <p className="delete-confirm-warning">{warning}</p> : null}
        <div className="modal-actions">
          <button type="button" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button type="button" className="button-danger" onClick={onConfirm} disabled={busy}>
            {busy ? "Deleting…" : confirmLabel}
          </button>
        </div>
      </section>
    </div>
  );
}
