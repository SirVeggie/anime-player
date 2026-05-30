import { type ReactNode, useEffect, useRef, useState } from "react";

export function AnimeCardLabel(props: {
  displayTitle: string;
  tooltipTitle: string;
  meta: ReactNode;
}) {
  const { displayTitle, tooltipTitle, meta } = props;
  const titleRef = useRef<HTMLDivElement>(null);
  const [titleTruncated, setTitleTruncated] = useState(false);

  useEffect(() => {
    const el = titleRef.current;
    if (!el) return;
    const update = () => {
      setTitleTruncated(el.scrollWidth > el.clientWidth);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, [displayTitle]);

  const showTooltip = tooltipTitle !== displayTitle || titleTruncated;

  return (
    <>
      <div className="anime-card-body">
        <div className="anime-card-title" ref={titleRef}>
          {displayTitle}
        </div>
        {meta}
      </div>
      {showTooltip ? <div className="anime-tooltip">{tooltipTitle}</div> : null}
    </>
  );
}
