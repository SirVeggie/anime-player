export function CustomCheckbox(props: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  /** Native browser tooltip until custom tooltips exist. */
  tooltip?: string;
  disabled?: boolean;
  className?: string;
}) {
  const { checked, onChange, label, tooltip, disabled = false, className } = props;
  return (
    <label
      className={["custom-checkbox", className].filter(Boolean).join(" ")}
      title={tooltip}
    >
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => {
          // Read synchronously: React's updater can run twice under StrictMode
          // and null `currentTarget` on the second pass.
          onChange(e.currentTarget.checked);
        }}
      />
      <span className="custom-checkbox__box" aria-hidden />
      <span className="custom-checkbox__text">{label}</span>
    </label>
  );
}
