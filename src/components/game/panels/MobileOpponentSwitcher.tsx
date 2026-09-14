import { ChevronRight } from "lucide-react";

interface MobileOpponentSwitcherProps {
  name: string;
  index: number;
  count: number;
  disabled: boolean;
  onNext: () => void;
}

export function MobileOpponentSwitcher({
  name,
  index,
  count,
  disabled,
  onNext,
}: MobileOpponentSwitcherProps) {
  if (count < 2) return null;

  return (
    <button
      type="button"
      disabled={disabled}
      className="pointer-events-auto absolute left-1/2 top-1 z-[2] flex min-h-12 max-w-52 -translate-x-1/2 items-center gap-2 rounded-full border border-border/80 bg-card/90 px-3 font-game text-xs font-semibold text-foreground shadow-lg backdrop-blur-sm disabled:opacity-50"
      aria-label={`Focused opponent ${name}, ${index + 1} of ${count}. Focus next opponent`}
      onClick={onNext}
    >
      <span className="truncate">{name}</span>
      <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
        {index + 1}/{count}
      </span>
      <ChevronRight className="h-4 w-4 shrink-0 text-primary" aria-hidden />
    </button>
  );
}
