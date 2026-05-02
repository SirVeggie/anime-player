import { useState } from "react";

export function CustomDropdown(props: {
  label: string;
  value: number;
  options: Array<{ value: number; label: string }>;
  onChange: (value: number) => void;
}) {
  const { label, value, options, onChange } = props;
  const [open, setOpen] = useState(false);

  return (
    <div className={`custom-select${open ? " custom-select--open" : ""}`}>
      <button type="button" className="custom-select-trigger" onClick={() => setOpen((current) => !current)}>
        <span>{label}</span>
        <span className="chevron" aria-hidden>
          ▾
        </span>
      </button>
      {open ? (
        <div className="custom-select-menu">
          {options.map((option) => (
            <button
              type="button"
              key={option.value}
              className={option.value === value ? "custom-select-option active" : "custom-select-option"}
              onClick={() => {
                onChange(option.value);
                setOpen(false);
              }}
            >
              {option.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
