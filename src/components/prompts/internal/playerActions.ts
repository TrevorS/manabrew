import type { Prompt, PromptOutput, PassUntil } from "@/protocol";

// "Pass" means decline whatever the current prompt asks: during combat
// declaration that's an empty attacker/blocker set; otherwise a priority pass.
export function passOutput(
  prompt: Prompt | null,
  until: PassUntil | null,
  exhaustStack = false,
): PromptOutput["output"] | null {
  if (!prompt) return null;
  switch (prompt.input.type) {
    case "chooseAttackers":
      return { type: "declareAttackers", assignments: [] };
    case "chooseBlockers":
      return { type: "declareBlockers", assignments: [] };
    case "chooseAction":
      return { type: "pass", until: until ?? undefined, exhaustStack };
    default:
      return { type: "pass", until: undefined, exhaustStack: false };
  }
}

export function declareAttackersOutput(
  prompt: Prompt | null,
  attackerIds: string[],
  targetId?: string,
): PromptOutput["output"] {
  const options = prompt?.input.type === "chooseAttackers" ? prompt.input.attackers : [];
  return {
    type: "declareAttackers",
    assignments: attackerIds.flatMap((attackerId) => {
      const option = options.find((a) => a.attackerId === attackerId);
      const valid = option?.validTargetIds ?? [];
      const required = option?.mustAttackTargetIds ?? [];
      const target =
        required.find((id) => id === targetId) ??
        required[0] ??
        (targetId != null && valid.includes(targetId) ? targetId : valid[0]);
      return target == null ? [] : [{ attackerId, targetId: target }];
    }),
  };
}
