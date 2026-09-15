export interface BattlefieldLayoutPolicy {
  compact: boolean;
  selfBattlefieldRows: number;
  opponentBattlefieldRows: number;
  selfFieldShare: number;
  focusedSelfFieldShare: number;
  opponentCardScaleRatio: number;
  focusedOpponentCardScaleRatio: number;
  reserveHandSpace: boolean;
  handPresentation: "inline" | "sheet";
  showPhaseDivider: boolean;
}

export const DESKTOP_BATTLEFIELD_LAYOUT = {
  compact: false,
  selfBattlefieldRows: 3,
  opponentBattlefieldRows: 3,
  selfFieldShare: 0.5,
  focusedSelfFieldShare: 0.5,
  opponentCardScaleRatio: 1,
  focusedOpponentCardScaleRatio: 1,
  reserveHandSpace: true,
  handPresentation: "inline",
  showPhaseDivider: true,
} as const satisfies BattlefieldLayoutPolicy;

export const MOBILE_BATTLEFIELD_LAYOUT = {
  compact: true,
  selfBattlefieldRows: 2,
  opponentBattlefieldRows: 1,
  selfFieldShare: 2 / 3,
  focusedSelfFieldShare: 0.5,
  opponentCardScaleRatio: 1,
  focusedOpponentCardScaleRatio: 1.25,
  reserveHandSpace: false,
  handPresentation: "sheet",
  showPhaseDivider: true,
} as const satisfies BattlefieldLayoutPolicy;
