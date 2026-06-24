import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

export type ContextMenuAction = {
  type: "action";
  id: string;
  label: string;
  danger?: boolean;
  disabled?: boolean;
  /** Shown on hover when the item is enabled. */
  title?: string;
  disabledTitle?: string;
  onSelect: () => void;
};

export type ContextMenuSubmenuItem = {
  id: string;
  label: string;
  disabled?: boolean;
  disabledTitle?: string;
  onSelect: () => void;
};

export type ContextMenuSubmenu = {
  type: "submenu";
  id: string;
  label: string;
  disabled?: boolean;
  disabledTitle?: string;
  items: ContextMenuSubmenuItem[];
};

export type ContextMenuSeparator = {
  type: "separator";
  id: string;
};

export type ContextMenuItem = ContextMenuAction | ContextMenuSubmenu | ContextMenuSeparator;

export type ContextMenuState = {
  x: number;
  y: number;
  items: ContextMenuItem[];
};

type DisabledTooltipState = {
  text: string;
  x: number;
  y: number;
};

export function useContextMenu() {
  const [menu, setMenu] = useState<ContextMenuState | null>(null);

  const openMenu = useCallback((event: ReactMouseEvent, items: ContextMenuItem[]) => {
    event.preventDefault();
    event.stopPropagation();
    setMenu({ x: event.clientX, y: event.clientY, items });
  }, []);

  const closeMenu = useCallback(() => {
    setMenu(null);
  }, []);

  return { menu, openMenu, closeMenu };
}

function clampMenuPosition(
  x: number,
  y: number,
  width: number,
  height: number,
): { x: number; y: number } {
  const margin = 8;
  const maxX = Math.max(margin, window.innerWidth - width - margin);
  const maxY = Math.max(margin, window.innerHeight - height - margin);
  return {
    x: Math.min(Math.max(margin, x), maxX),
    y: Math.min(Math.max(margin, y), maxY),
  };
}

/** Viewport position for a nested flyout beside `anchorRect`. */
function nestedSubmenuViewportPosition(
  anchorRect: DOMRect,
  nestedWidth: number,
  nestedHeight: number,
): { x: number; y: number } {
  const margin = 8;
  const gap = 2;
  const menuPadding = 4;

  let x = anchorRect.right + gap;
  if (x + nestedWidth > window.innerWidth - margin) {
    x = anchorRect.left - nestedWidth - gap;
  }
  x = Math.min(Math.max(margin, x), Math.max(margin, window.innerWidth - nestedWidth - margin));

  let y = anchorRect.top - menuPadding;
  if (y + nestedHeight > window.innerHeight - margin) {
    y = window.innerHeight - margin - nestedHeight;
  }
  y = Math.min(Math.max(margin, y), Math.max(margin, window.innerHeight - nestedHeight - margin));

  return { x, y };
}

function ContextMenuTooltip(props: { tooltip: DisabledTooltipState | null }) {
  const { tooltip } = props;
  if (!tooltip) return null;

  return createPortal(
    <div
      className="context-menu-tooltip"
      role="tooltip"
      style={{ left: tooltip.x, top: tooltip.y }}
    >
      {tooltip.text}
    </div>,
    document.body,
  );
}

function ContextMenuItemButton(props: {
  label: string;
  danger?: boolean;
  disabled?: boolean;
  title?: string;
  disabledTitle?: string;
  itemIndex: number;
  className?: string;
  buttonRef?: React.RefObject<HTMLButtonElement | null>;
  suffix?: ReactNode;
  ariaHasPopup?: boolean | "menu";
  ariaExpanded?: boolean;
  onHover?: () => void;
  onShowDisabledTooltip: (tooltip: DisabledTooltipState | null) => void;
  onClick?: () => void;
}) {
  const {
    label,
    danger = false,
    disabled = false,
    title,
    disabledTitle,
    itemIndex,
    className = "",
    buttonRef,
    suffix,
    ariaHasPopup,
    ariaExpanded,
    onHover,
    onShowDisabledTooltip,
    onClick,
  } = props;

  const revealItemTooltip = useCallback(
    (target: HTMLButtonElement) => {
      const tooltipText = disabled ? disabledTitle : title;
      if (!tooltipText) {
        onShowDisabledTooltip(null);
        return;
      }
      const rect = target.getBoundingClientRect();
      const margin = 8;
      const maxWidth = 240;
      const x = Math.min(rect.right + 10, window.innerWidth - maxWidth - margin);
      onShowDisabledTooltip({
        text: tooltipText,
        x,
        y: rect.top + rect.height / 2,
      });
    },
    [disabled, disabledTitle, onShowDisabledTooltip, title],
  );

  const handleMouseEnter = useCallback(
    (event: ReactMouseEvent<HTMLButtonElement>) => {
      onHover?.();
      revealItemTooltip(event.currentTarget);
    },
    [onHover, revealItemTooltip],
  );

  return (
    <button
      ref={buttonRef}
      type="button"
      role="menuitem"
      className={`context-menu-item${danger ? " context-menu-item--danger" : ""}${disabled ? " context-menu-item--disabled" : ""}${className ? ` ${className}` : ""}`}
      disabled={disabled}
      style={{ "--item-index": itemIndex } as CSSProperties}
      aria-haspopup={ariaHasPopup}
      aria-expanded={ariaExpanded}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={() => onShowDisabledTooltip(null)}
      onFocus={(event) => revealItemTooltip(event.currentTarget)}
      onBlur={() => onShowDisabledTooltip(null)}
      onClick={onClick}
    >
      <span>{label}</span>
      {suffix}
    </button>
  );
}

function ContextMenuSubmenuRow(props: {
  item: ContextMenuSubmenu;
  itemIndex: number;
  open: boolean;
  onOpen: () => void;
  onCloseMenu: () => void;
  onShowDisabledTooltip: (tooltip: DisabledTooltipState | null) => void;
}) {
  const { item, itemIndex, open, onOpen, onCloseMenu, onShowDisabledTooltip } = props;
  const rowRef = useRef<HTMLButtonElement>(null);
  const nestedRef = useRef<HTMLDivElement>(null);
  const [nestedPosition, setNestedPosition] = useState<{ x: number; y: number } | null>(null);

  useLayoutEffect(() => {
    if (!open) {
      setNestedPosition(null);
      return;
    }
    const anchor = rowRef.current;
    const nested = nestedRef.current;
    if (!anchor || !nested) return;

    const anchorRect = anchor.getBoundingClientRect();
    const nestedRect = nested.getBoundingClientRect();
    setNestedPosition(nestedSubmenuViewportPosition(anchorRect, nestedRect.width, nestedRect.height));
  }, [open, item.items]);

  return (
    <div className={`context-menu-submenu${open ? " context-menu-submenu--open" : ""}`}>
      <ContextMenuItemButton
        buttonRef={rowRef}
        label={item.label}
        disabled={item.disabled}
        disabledTitle={item.disabledTitle}
        itemIndex={itemIndex}
        className="context-menu-item--submenu"
        suffix={
          <span className="context-menu-chevron" aria-hidden>
            ›
          </span>
        }
        ariaHasPopup="menu"
        ariaExpanded={open}
        onHover={onOpen}
        onShowDisabledTooltip={onShowDisabledTooltip}
      />
      {open
        ? createPortal(
            <div
              ref={nestedRef}
              className="context-menu context-menu--nested"
              role="menu"
              style={
                nestedPosition
                  ? { left: nestedPosition.x, top: nestedPosition.y, visibility: "visible" }
                  : { left: -9999, top: 0, visibility: "hidden" }
              }
              onMouseEnter={onOpen}
            >
              {item.items.map((subItem, subIndex) => (
                <ContextMenuItemButton
                  key={subItem.id}
                  label={subItem.label}
                  disabled={subItem.disabled}
                  disabledTitle={subItem.disabledTitle}
                  itemIndex={subIndex}
                  onShowDisabledTooltip={onShowDisabledTooltip}
                  onClick={() => {
                    if (subItem.disabled) return;
                    onCloseMenu();
                    subItem.onSelect();
                  }}
                />
              ))}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}

function ContextMenuPanel(props: {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}) {
  const { x, y, items, onClose } = props;
  const panelRef = useRef<HTMLDivElement>(null);
  const [openSubmenuId, setOpenSubmenuId] = useState<string | null>(null);
  const [position, setPosition] = useState({ x, y });
  const [disabledTooltip, setDisabledTooltip] = useState<DisabledTooltipState | null>(null);

  useLayoutEffect(() => {
    const panel = panelRef.current;
    if (!panel) return;
    const rect = panel.getBoundingClientRect();
    setPosition(clampMenuPosition(x, y, rect.width, rect.height));
  }, [items, x, y]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (panelRef.current?.contains(target)) return;
      if (target instanceof Element && target.closest(".context-menu--nested")) return;
      if (target instanceof Element && target.closest(".context-menu-tooltip")) return;
      onClose();
    };
    const onScroll = () => onClose();

    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  const openSubmenu = useCallback((id: string, disabled?: boolean) => {
    if (disabled) return;
    setOpenSubmenuId(id);
    setDisabledTooltip(null);
  }, []);

  let itemIndex = 0;

  return (
    <>
      <div
        ref={panelRef}
        className="context-menu"
        role="menu"
        style={{ left: position.x, top: position.y }}
        onMouseLeave={(event) => {
          const related = event.relatedTarget;
          if (related instanceof Node && panelRef.current?.contains(related)) return;
          if (related instanceof Element && related.closest(".context-menu--nested")) return;
          if (related instanceof Element && related.closest(".context-menu-tooltip")) return;
          setOpenSubmenuId(null);
          setDisabledTooltip(null);
        }}
      >
        {items.map((item) => {
          if (item.type === "separator") {
            return <div key={item.id} className="context-menu-separator" role="separator" />;
          }

          if (item.type === "submenu") {
            const index = itemIndex++;
            return (
              <ContextMenuSubmenuRow
                key={item.id}
                item={item}
                itemIndex={index}
                open={openSubmenuId === item.id}
                onOpen={() => openSubmenu(item.id, item.disabled)}
                onCloseMenu={onClose}
                onShowDisabledTooltip={setDisabledTooltip}
              />
            );
          }

          const index = itemIndex++;
          return (
            <ContextMenuItemButton
              key={item.id}
              label={item.label}
              danger={item.danger}
              disabled={item.disabled}
              title={item.title}
              disabledTitle={item.disabledTitle}
              itemIndex={index}
              onHover={() => setOpenSubmenuId(null)}
              onShowDisabledTooltip={setDisabledTooltip}
              onClick={() => {
                if (item.disabled) return;
                onClose();
                item.onSelect();
              }}
            />
          );
        })}
      </div>
      <ContextMenuTooltip tooltip={disabledTooltip} />
    </>
  );
}

export function ContextMenu(props: {
  menu: ContextMenuState | null;
  onClose: () => void;
}) {
  const { menu, onClose } = props;
  if (!menu) return null;

  return createPortal(
    <div className="context-menu-layer">
      <ContextMenuPanel
        x={menu.x}
        y={menu.y}
        items={menu.items}
        onClose={onClose}
      />
    </div>,
    document.body,
  );
}
