# Ability effects

The 200+ `*_effect.rs` resolvers that execute spell and ability resolution. **Most parity bugs land here.**

Read first: `manabrew-engine/AGENTS.md`, `docs/agents/PARITY_PHILOSOPHY.md`, `docs/agents/ENGINE_BUGFIX_WORKFLOW.md`.

Java mirror: `forge/forge-game/src/main/java/forge/game/ability/effects/`.

## File naming

| Forge name               | Rust file                                 |
| ------------------------ | ----------------------------------------- |
| `DealDamageEffect.java`  | `damage_deal_effect.rs`                   |
| `ChangeZoneEffect.java`  | `change_zone_effect/` (folder when split) |
| `CountersPutEffect.java` | `counters_put_effect.rs`                  |

PascalCase → snake_case, keep the `_effect` suffix. Don't drop or rename. If the Java file is a folder of helpers, keep the same split.

## Adding or fixing an effect

1. Find the Java file. Read it end-to-end.
2. Locate or create the Rust file with the matching name.
3. Implement the resolver. Mirror the Java method names — `resolve`, `getStackDescription`, helpers — translated to snake_case.
4. Register the effect in `mod.rs`'s dispatch (the `match` on `ApiType`).
5. If you added a new API type, also register it in `manabrew-rs/crates/manabrew-engine/src/ability/api_type.rs`.
6. Verify via `yarn parity` against a deck that exercises the effect. If none exists, add a regression entry — see `manabrew-rs/crates/parity/AGENTS.md`.

## Conventions

- **One effect per file.** Helpers shared across effects go in `helpers.rs`, `effect_context.rs`, or `combat_helpers.rs`. Don't create new shared modules until two effects actually share.
- **No card-specific branches.** If you find yourself writing `if card.name() == "..."`, the rule lives elsewhere — almost always in `staticability/`, `replacement/`, or a `trigger/`. Find it.
- **Targeting and restrictions live upstream.** By the time an effect resolves, targets are validated. Don't re-validate; trust the spell-ability machinery.
- **Don't bypass replacements.** Damage, zone changes, life loss, counters — every mutation that has a replacement type must go through the corresponding `replacement/` callsite. See `replacement/replacement_handler.rs`.
- **Mirror Java's branching.** Even when it looks redundant. See `docs/agents/PARITY_PHILOSOPHY.md`.
- **A sub-ability chain resolves through `effect_resolver::resolve_effect_chain`, never a private loop.** Java has one walker (`AbilityUtils.resolve` follows `getSubAbility` after each node) and so does this port: it stops after a node whose `UnlessCost$` handling already resolved the rest of the chain (`sub_ability_handled_internally`), and hands a parent's chosen target to a child that chose none (the `Defined$ Targeted` rule). A hand-rolled `while let Some(sa) = current { resolve_effect(ctx, &sa); current = sa.sub_ability... }` does neither: seven effects had one, and each resolved an `UnlessCost$` tail twice. `charm_effect.rs` is the one loop that stays, because it sets up each mode's targets per node before resolving it.

## Where things live

| Concern                | File                                                                                         |
| ---------------------- | -------------------------------------------------------------------------------------------- |
| Damage resolution      | `damage_*_effect.rs`, `damage_resolve_effect.rs`, `damage_base_effect.rs`                    |
| Counter manipulation   | `counters_*_effect.rs`                                                                       |
| Zone changes (general) | `change_zone_effect/` (folder), `change_zone_all_effect.rs`, `change_zone_resolve_effect.rs` |
| Token creation         | `token_effect.rs`, `token_effect_base.rs`, `trait_token_effect.rs`                           |
| Mana                   | `mana_effect.rs`, `mana_reflected_effect.rs`, `drain_mana_effect.rs`                         |
| Pump / animate         | `pump_*_effect.rs`, `animate_*_effect.rs`, `trait_animate_effect.rs`                         |
| Targeting helpers      | `targeting_triggers.rs`, `helpers.rs`                                                        |
| Effect dispatch        | `mod.rs` — the `match` on `ApiType`                                                          |
