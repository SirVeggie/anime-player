import { GRID_FILTER_OPTIONS } from "../animeGridFilter";

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

export function GridFilterBar(props: {
  value: number;
  onChange: (value: number) => void;
}) {
  const { value, onChange } = props;
  return (
    <div className="grid-filter-bar" role="radiogroup" aria-label="Filter by watch status">
      {GRID_FILTER_OPTIONS.map((option) => {
        const active = option.value === value;
        return (
          <button
            type="button"
            key={option.value}
            className={active ? "grid-filter-chip grid-filter-chip--active" : "grid-filter-chip"}
            role="radio"
            aria-checked={active}
            onClick={() => onChange(option.value)}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
