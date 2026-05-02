export type Toast = { id: number; kind: "success" | "error"; message: string };

export function ToastStack(props: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  const { toasts, onDismiss } = props;
  return (
    <div className="toast-stack" aria-live="polite" aria-atomic="true">
      {toasts.map((toast) => (
        <button
          type="button"
          key={toast.id}
          className={`toast toast--${toast.kind}`}
          onClick={() => onDismiss(toast.id)}
        >
          {toast.message}
        </button>
      ))}
    </div>
  );
}
