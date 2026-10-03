import { GRID_FILTER_OPTIONS } from "../animeGridFilter";

export function GridFilterBar(props: {
  value: number;
  onChange: (value: number) => void;
}) {
  const { value, onChange } = props;
  return (
    <div className="grid-filter-bar" role="radiogroup" aria-label="Filter titles">
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
