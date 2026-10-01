package forge.harness.parity;

import com.google.gson.JsonObject;
import forge.harness.common.CountingRandom;
import forge.harness.common.DrawTape;

final class RustTape implements DrawTape {
    private static final long WAIT_MILLIS = 600_000;

    private final LockstepLink link;
    private final CountingRandom shared;
    private int consumed;

    RustTape(final LockstepLink link, final CountingRandom shared) {
        this.link = link;
        this.shared = shared;
    }

    @Override
    public int draw(final int bound) {
        final int fallback = shared.nextInt(bound);
        final JsonObject draw = next();
        final int rustBound = draw.get("b").getAsInt();
        final int value = draw.get("v").getAsInt();
        if (rustBound != bound) {
            throw new LockstepDesync("draw", "rust draw " + consumed + " bound " + rustBound + ", java bound " + bound);
        }
        if (draw.get("f").getAsInt() == 1 && value != fallback) {
            throw new LockstepDesync("draw", "rust fallback draw " + consumed + " " + value + ", java fallback " + fallback);
        }
        consumed++;
        return value;
    }

    private JsonObject next() {
        boolean asked = false;
        final long deadline = System.currentTimeMillis() + WAIT_MILLIS;
        while (true) {
            if (link.aborted()) {
                throw new LockstepDesync("abort", "rust aborted the game");
            }
            JsonObject message = link.poll(0);
            if (message == null) {
                if (!asked) {
                    link.send("{\"t\":\"need\",\"k\":" + consumed + "}");
                    asked = true;
                }
                message = link.poll(1000);
            }
            if (message == null) {
                if (System.currentTimeMillis() > deadline) {
                    throw new LockstepDesync("timeout", "no rust draw for " + WAIT_MILLIS + " ms");
                }
                continue;
            }
            final String type = LockstepLink.type(message);
            if ("d".equals(type)) {
                return message;
            }
            throw new LockstepDesync("sequence", "rust sent " + type + " while java waited for draw " + consumed);
        }
    }
}
