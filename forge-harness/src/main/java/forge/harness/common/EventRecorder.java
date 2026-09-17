package forge.harness.common;

import java.util.Set;
import java.util.function.Consumer;

import com.google.common.eventbus.Subscribe;

import forge.game.Game;
import forge.game.event.GameEvent;

/**
 * Writes every Forge game event into the parity log as an {@code event} entry.
 *
 * <p>Forge fires these synchronously on the game thread, at the point the state
 * changes, so their position between two snapshots says what Forge did in that
 * window and in what order. The Rust side has no equivalent bus; the entries
 * are read next to a divergence, not compared.
 */
public final class EventRecorder {
    /** Bookkeeping events that repeat what snapshots and callbacks already say. */
    private static final Set<String> SKIPPED = Set.of(
        "GameEventTurnPhase",
        "GameEventPlayerPriority",
        "GameEventAddLog",
        "GameEventRandomLog",
        "GameEventCardStatsChanged",
        "GameEventPlayerStatsChanged",
        "GameEventZone",
        "GameEventCombatChanged",
        "GameEventCombatUpdate",
        "GameEventManaPool",
        "GameEventSnapshotRestored");

    private final Game game;
    private final Consumer<String> sink;

    public EventRecorder(Game game, Consumer<String> sink) {
        this.game = game;
        this.sink = sink;
    }

    @Subscribe
    public void onEvent(GameEvent event) {
        String type = event.getClass().getSimpleName();
        if (SKIPPED.contains(type)) {
            return;
        }
        String text;
        try {
            text = String.valueOf(event);
        } catch (RuntimeException e) {
            text = "<" + e.getClass().getSimpleName() + " in toString>";
        }
        sink.accept("{\"entry_type\":\"event\""
            + ",\"turn\":" + game.getPhaseHandler().getTurn()
            + ",\"phase\":\"" + escape(phaseName()) + "\""
            + ",\"kind\":\"" + escape(type.replaceFirst("^GameEvent", "")) + "\""
            + ",\"text\":\"" + escape(text) + "\"}");
    }

    private String phaseName() {
        forge.game.phase.PhaseType phase = game.getPhaseHandler().getPhase();
        return phase == null ? "" : SnapshotExtractor.phaseToRustName(phase);
    }

    private static String escape(String s) {
        StringBuilder out = new StringBuilder(s.length() + 8);
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '\\': out.append("\\\\"); break;
                case '"': out.append("\\\""); break;
                case '\n': out.append("\\n"); break;
                case '\r': out.append("\\r"); break;
                case '\t': out.append("\\t"); break;
                default:
                    if (c < 0x20) {
                        out.append(String.format("\\u%04x", (int) c));
                    } else {
                        out.append(c);
                    }
            }
        }
        return out.toString();
    }
}
