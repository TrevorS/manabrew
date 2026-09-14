import { Check } from "lucide-react";

import { PHASES } from "@/components/game/game.constants";
import { cn } from "@/lib/utils";
import type { StepKind } from "@/protocol";

const COMBAT_STOP: StepKind = "combatDeclareAttackers";
const PHASE_CONTROLS = PHASES.filter(
  (phase) => phase.id !== "untap" && (!phase.combat || phase.id === COMBAT_STOP),
).map((phase) => ({
  id: phase.id,
  label: phase.combat ? "Combat" : phase.label,
  short: phase.combat ? "COM" : phase.short,
}));

interface OpponentStops {
  id: string;
  name: string;
  stops: ReadonlySet<string>;
}

interface MobilePhaseStopsProps {
  open: boolean;
  currentStep: StepKind;
  selfStops: ReadonlySet<string>;
  opponents: OpponentStops[];
  onClose: () => void;
  onToggleSelf: (phase: string) => void;
  onToggleOpponent: (opponentId: string, phase: string) => void;
}

export function MobilePhaseStops({
  open,
  currentStep,
  selfStops,
  opponents,
  onClose,
  onToggleSelf,
  onToggleOpponent,
}: MobilePhaseStopsProps) {
  if (!open) return null;

  const currentLabel = PHASES.find((phase) => phase.id === currentStep)?.label ?? currentStep;

  return (
    <div className="pointer-events-auto absolute inset-0 z-[3]">
      <button
        type="button"
        className="absolute inset-0 bg-background/70 backdrop-blur-[2px]"
        aria-label="Close phase stops"
        onClick={onClose}
      />
      <section
        aria-label="Phase stops"
        className="absolute inset-x-2 bottom-2 max-h-[calc(100%-1rem)] overflow-y-auto rounded-xl border border-border/80 bg-card/95 p-3 shadow-2xl backdrop-blur-md"
      >
        <div className="mb-3 flex items-center justify-between gap-3 pr-10">
          <div>
            <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              Current phase
            </p>
            <h2 className="font-game text-base font-semibold text-foreground">{currentLabel}</h2>
          </div>
          <button
            type="button"
            className="min-h-12 rounded-lg border border-border bg-background/60 px-4 text-sm font-semibold text-foreground"
            onClick={onClose}
          >
            Done
          </button>
        </div>

        <StopRow label="Your stops" stops={selfStops} onToggle={(phase) => onToggleSelf(phase)} />
        {opponents.map((opponent) => (
          <StopRow
            key={opponent.id}
            label={`${opponent.name}'s stops`}
            stops={opponent.stops}
            onToggle={(phase) => onToggleOpponent(opponent.id, phase)}
          />
        ))}
      </section>
    </div>
  );
}

function StopRow({
  label,
  stops,
  onToggle,
}: {
  label: string;
  stops: ReadonlySet<string>;
  onToggle: (phase: string) => void;
}) {
  return (
    <div className="mb-3 last:mb-0">
      <p className="mb-1.5 truncate text-xs font-semibold text-muted-foreground">{label}</p>
      <div className="grid grid-cols-7 gap-1">
        {PHASE_CONTROLS.map((phase) => {
          const enabled = stops.has(phase.id);
          return (
            <button
              key={phase.id}
              type="button"
              aria-pressed={enabled}
              title={phase.label}
              className={cn(
                "relative flex min-h-12 min-w-0 items-center justify-center rounded-md border px-1 font-game text-[10px] font-bold",
                enabled
                  ? "border-primary bg-primary/20 text-foreground"
                  : "border-border/70 bg-background/40 text-muted-foreground",
              )}
              onClick={() => onToggle(phase.id)}
            >
              {phase.short}
              {enabled && <Check className="absolute right-0.5 top-0.5 h-3 w-3" aria-hidden />}
            </button>
          );
        })}
      </div>
    </div>
  );
}
