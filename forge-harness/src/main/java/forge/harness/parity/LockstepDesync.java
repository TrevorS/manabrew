package forge.harness.parity;

final class LockstepDesync extends RuntimeException {
    final String kind;

    LockstepDesync(final String kind, final String detail) {
        super(kind + ": " + detail);
        this.kind = kind;
    }
}
