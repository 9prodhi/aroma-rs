class Fixture {
    void guard(Object lock, int state) {
        synchronized (lock) {
            if (state < 0) {
                throw new IllegalStateException("negative: " + state);
            }
        }
    }
}
