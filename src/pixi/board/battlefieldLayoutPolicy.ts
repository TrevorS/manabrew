export interface BattlefieldLayoutPolicy {
  compact: boolean;
  battlefieldRows: number;
  reserveHandSpace: boolean;
  handPresentation: "inline" | "sheet";
  showPhaseDivider: boolean;
}

export const DESKTOP_BATTLEFIELD_LAYOUT = {
  compact: false,
  battlefieldRows: 3,
  reserveHandSpace: true,
  handPresentation: "inline",
  showPhaseDivider: true,
} as const satisfies BattlefieldLayoutPolicy;

export const MOBILE_BATTLEFIELD_LAYOUT = {
  compact: true,
  battlefieldRows: 2,
  reserveHandSpace: false,
  handPresentation: "sheet",
  showPhaseDivider: true,
} as const satisfies BattlefieldLayoutPolicy;
