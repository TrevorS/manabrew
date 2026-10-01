package forge.harness.parity;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.PrintStream;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;

final class LockstepLink {
    private static final JsonObject QUIT = JsonParser.parseString("{\"t\":\"quit\"}").getAsJsonObject();

    private final PrintStream out;
    private final LinkedBlockingQueue<JsonObject> inbox = new LinkedBlockingQueue<>();
    private volatile boolean aborted;

    LockstepLink(final InputStream in, final PrintStream out) {
        this.out = out;
        final Thread reader = new Thread(() -> read(in), "lockstep-stdin");
        reader.setDaemon(true);
        reader.start();
    }

    private void read(final InputStream in) {
        try (BufferedReader lines = new BufferedReader(new InputStreamReader(in))) {
            String line;
            while ((line = lines.readLine()) != null) {
                line = line.trim();
                if (line.isEmpty()) {
                    continue;
                }
                final JsonObject message = JsonParser.parseString(line).getAsJsonObject();
                if ("abort".equals(type(message))) {
                    aborted = true;
                }
                inbox.add(message);
            }
        } catch (IOException | RuntimeException e) {
            System.err.println("[lockstep] stdin: " + e);
        }
        aborted = true;
        inbox.add(QUIT);
    }

    static String type(final JsonObject message) {
        return message.get("t").getAsString();
    }

    void send(final String line) {
        synchronized (out) {
            out.println(line);
            out.flush();
        }
    }

    JsonObject take() throws InterruptedException {
        return inbox.take();
    }

    JsonObject poll(final long millis) {
        try {
            return inbox.poll(millis, TimeUnit.MILLISECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new LockstepDesync("abort", "interrupted");
        }
    }

    boolean aborted() {
        return aborted;
    }

    void clearAbort() {
        aborted = false;
    }
}
