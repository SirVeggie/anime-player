export function GridTextFilter(props: {
  value: string;
  onChange: (value: string) => void;
}) {
  const { value, onChange } = props;
  return (
    <div className="grid-text-filter">
      <input
        type="text"
        value={value}
        onChange={(event) => onChange(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key !== "Escape" || !value) return;
          event.preventDefault();
          event.stopPropagation();
          onChange("");
        }}
        placeholder="Filter titles…"
        aria-label="Filter titles by name or filename"
      />
      {value ? (
        <button
          type="button"
          className="grid-text-filter__clear"
          onClick={() => onChange("")}
          aria-label="Clear title filter"
          title="Clear filter"
        >
          ×
        </button>
      ) : null}
    </div>
  );
}
