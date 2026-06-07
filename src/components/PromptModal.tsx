import { type FormEvent, useEffect, useRef } from "react";

export function PromptModal(props: {
  title: string;
  description?: string;
  label: string;
  initialValue?: string;
  submitLabel?: string;
  busy?: boolean;
  error?: string | null;
  onSubmit: (value: string) => void;
  onClose: () => void;
}) {
  const {
    title,
    description,
    label,
    initialValue = "",
    submitLabel = "Save",
    busy = false,
    error = null,
    onSubmit,
    onClose,
  } = props;
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
    return () => window.cancelAnimationFrame(frame);
  }, []);

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    if (busy) return;
    onSubmit(inputRef.current?.value ?? initialValue);
  };

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onClose();
      }}
    >
      <section className="modal prompt-modal" role="dialog" aria-modal="true" aria-labelledby="prompt-modal-title">
        <div className="modal-heading">
          <div>
            <h2 id="prompt-modal-title">{title}</h2>
            {description ? <p className="muted">{description}</p> : null}
          </div>
        </div>
        <form className="prompt-modal-form" onSubmit={handleSubmit}>
          <label className="stacked-field">
            <span>{label}</span>
            <input
              ref={inputRef}
              type="text"
              defaultValue={initialValue}
              disabled={busy}
            />
          </label>
          {error ? <p className="error">{error}</p> : null}
          <div className="modal-actions">
            <button type="button" onClick={onClose} disabled={busy}>
              Cancel
            </button>
            <button type="submit" disabled={busy}>
              {busy ? "Saving…" : submitLabel}
            </button>
          </div>
        </form>
      </section>
    </div>
  );
}
