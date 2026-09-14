import { useLayoutEffect, useRef } from "react";
import { X } from "lucide-react";

import { CardsInHandIcon } from "@/components/game/panels/CardsInHandIcon";
import { cn } from "@/lib/utils";

interface MobileHandControlProps {
  count: number;
  open: boolean;
  locked: boolean;
  actionable: boolean;
  onToggle: () => void;
  onBoundsChange?: (bounds: DOMRect | null) => void;
}

export function MobileHandControl({
  count,
  open,
  locked,
  actionable,
  onToggle,
  onBoundsChange,
}: MobileHandControlProps) {
  const buttonRef = useRef<HTMLButtonElement>(null);

  useLayoutEffect(() => {
    const button = buttonRef.current;
    if (!button || !onBoundsChange) return;
    const reportBounds = () => onBoundsChange(button.getBoundingClientRect());
    reportBounds();
    const observer = new ResizeObserver(reportBounds);
    observer.observe(button);
    window.addEventListener("resize", reportBounds);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", reportBounds);
    };
  }, [open, onBoundsChange]);

  useLayoutEffect(() => () => onBoundsChange?.(null), [onBoundsChange]);

  return (
    <button
      ref={buttonRef}
      type="button"
      aria-expanded={open}
      aria-label={
        open
          ? locked
            ? "Hand required"
            : "Close hand"
          : `Open hand, ${count} cards${actionable ? ", actions available" : ""}`
      }
      className={cn(
        "pointer-events-auto absolute z-[4] flex min-h-12 items-center justify-center gap-2 rounded-full border border-border/80 bg-card/95 px-4 font-game text-sm font-semibold text-foreground shadow-xl backdrop-blur-md active:bg-accent",
        open ? "right-2 top-2" : "bottom-2 left-1/2 -translate-x-1/2",
        actionable &&
          !open &&
          "border-card-ring bg-card-ring/15 ring-2 ring-card-ring shadow-[0_0_18px_var(--card-ring)]",
        (locked || count === 0) && "opacity-70",
      )}
      disabled={locked || count === 0}
      onClick={onToggle}
    >
      {open ? (
        <X className="h-4 w-4" aria-hidden />
      ) : (
        <CardsInHandIcon count={count} className="h-7 w-9" />
      )}
      {open ? (locked ? "Select cards" : "Close") : `Hand ${count}`}
    </button>
  );
}
