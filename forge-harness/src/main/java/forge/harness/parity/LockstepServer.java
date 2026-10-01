package forge.harness.parity;

import com.google.common.eventbus.Subscribe;
import com.google.gson.JsonObject;
import forge.deck.Deck;
import forge.game.Game;
import forge.game.GameEndReason;
import forge.game.GameRules;
import forge.game.GameType;
import forge.game.Match;
import forge.game.event.GameEventTurnPhase;
import forge.game.phase.PhaseType;
import forge.game.player.Player;
import forge.game.player.PlayerController;
import forge.game.player.RegisteredPlayer;
import forge.harness.common.CountingRandom;
import forge.harness.common.DecisionLog;
import forge.harness.common.ForgeEngineReset;
import forge.harness.common.ParityCardMap;
import forge.harness.common.ParityLog;
import forge.harness.common.SnapshotExtractor;
import forge.util.MyRandom;
import forge.util.ThreadUtil;

import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.EnumSet;
import java.util.List;
import java.util.Set;

public final class LockstepServer {
    private final LockstepLink link;

    private LockstepServer(final InputStream in, final PrintStream out) {
        this.link = new LockstepLink(in, out);
    }

    public static void run(final InputStream in, final PrintStream out) {
        if (!ThreadUtil.isSynchronous()) {
            out.println("{\"t\":\"fatal\",\"error\":\"forge.synchronous must be true\"}");
            out.flush();
            return;
        }
        new LockstepServer(in, out).serve();
    }

    private void serve() {
        link.send("{\"t\":\"ready\"}");
        while (true) {
            final JsonObject message;
            try {
                message = link.take();
            } catch (InterruptedException e) {
                return;
            }
            final String type = LockstepLink.type(message);
            if ("quit".equals(type)) {
                return;
            }
            if ("start".equals(type)) {
                link.clearAbort();
                play(message);
                forge.ImageKeys.clearCaches();
            }
        }
    }

    private void play(final JsonObject request) {
        final String deck1Spec = request.get("deck1").getAsString();
        final String deck2Spec = request.get("deck2").getAsString();
        final long seed = request.get("seed").getAsLong();
        final int maxTurns = request.get("max_turns").getAsInt();
        final String logPath = request.has("log") ? request.get("log").getAsString() : null;

        String error = null;
        String desync = null;
        Game game = null;
        PrintStream log = null;
        try {
            if (logPath != null) {
                log = new PrintStream(new FileOutputStream(logPath), true, StandardCharsets.UTF_8);
            }
            final Deck deck1 = PresetDecks.buildDeck(deck1Spec);
            final Deck deck2 = PresetDecks.buildDeck(deck2Spec);
            if (deck1 == null || deck2 == null) {
                throw new IllegalArgumentException("unknown deck");
            }
            final Set<GameType> variants = EnumSet.of(GameType.Constructed);
            final GameRules rules = new GameRules(GameType.Constructed);
            rules.setAppliedVariants(variants);
            rules.setSimTimeout(120);

            final CountingRandom gameRng = new CountingRandom(seed, "game");
            MyRandom.setRandom(gameRng);
            ForgeEngineReset.resetAllIdCounters();
            final CountingRandom shared = new CountingRandom(seed, "agent");

            final CountingRandom rustRandom = new CountingRandom(seed, "agent");
            rustRandom.setTape(new RustTape(link, shared));

            final List<RegisteredPlayer> players = new ArrayList<>();
            final Deck[] decks = {deck1, deck2};
            for (int seat = 0; seat < 2; seat++) {
                final String name = "Player" + (seat + 1);
                final RegisteredPlayer rp = RegisteredPlayer.forVariants(2, variants, decks[seat], null, false, null, null);
                rp.setPlayer(new DeterministicLobbyPlayer(name, rustRandom, false, false));
                players.add(rp);
            }

            final Match match = new Match(rules, players, "Lockstep");
            game = match.createGame();
            ParityCardMap.reset();
            subscribe(game, maxTurns, log);

            ParityLog.enable(shared);
            final PrintStream decisionLog = log;
            DecisionLog.setSink(decisionLog == null ? null : decisionLog::println, false);
            match.startGame(game);
        } catch (LockstepDesync e) {
            desync = e.getMessage();
        } catch (IOException | RuntimeException e) {
            error = String.valueOf(e);
            System.err.println("[lockstep] game error: " + e);
            e.printStackTrace(System.err);
        } finally {
            DecisionLog.setSink(null, false);
            ParityLog.disable();
            if (game != null && !game.isGameOver()) {
                game.setGameOver(GameEndReason.Draw);
            }
            if (log != null) {
                log.close();
            }
        }

        final JsonObject end = new JsonObject();
        end.addProperty("t", "end");
        int winner = -1;
        int turn = 0;
        if (game != null) {
            turn = game.getPhaseHandler().getTurn();
            if (game.getOutcome() != null && !game.getOutcome().isDraw()) {
                for (int seat = 0; seat < game.getRegisteredPlayers().size(); seat++) {
                    if (game.getOutcome().isWinner(game.getRegisteredPlayers().get(seat).getLobbyPlayer())) {
                        winner = seat;
                    }
                }
            }
        }
        end.addProperty("winner", winner);
        end.addProperty("turn", turn);
        end.addProperty("error", error);
        end.addProperty("desync", desync);
        if (game != null && (desync != null || error != null)) {
            final com.google.gson.JsonObject snapshot = com.google.gson.JsonParser.parseString(SnapshotExtractor.snapshotJson(game)).getAsJsonObject();
            snapshot.addProperty("agent_rng_calls", 0);
            end.add("snapshot", snapshot);
        }
        link.send(end.toString());
    }

    private void subscribe(final Game game, final int maxTurns, final PrintStream log) {
        game.subscribeToEvents(new Object() {
            @Subscribe
            public void onTurnPhase(final GameEventTurnPhase event) {
                final int turn = game.getPhaseHandler().getTurn();
                final int active = game.getPhaseHandler().getPlayerTurn().getId();
                for (final Player player : game.getPlayers()) {
                    final DeterministicController det = deterministic(player.getController());
                    if (det != null) {
                        if (det.getCurrentTurn() != turn) {
                            det.setCurrentTurn(turn);
                            det.logTurnChanged(turn, active);
                        }
                        det.logPhaseChanged(event.phase().name());
                    }
                }
                if (turn > maxTurns) {
                    game.setGameOver(GameEndReason.Draw);
                    return;
                }
                if (event.phase() != PhaseType.UNTAP) {
                    return;
                }
                if (turn == 1) {
                    ParityCardMap.initializeFromOpeningState(game);
                }
                ParityCardMap.syncWithGame(game);
                final String snapshot = SnapshotExtractor.snapshotJson(game);
                if (log != null) {
                    log.println(snapshot);
                }
                link.send("{\"t\":\"snap\",\"s\":" + snapshot + "}");
            }
        });
    }

    private static DeterministicController deterministic(final PlayerController controller) {
        return controller instanceof DeterministicController det ? det : null;
    }
}
