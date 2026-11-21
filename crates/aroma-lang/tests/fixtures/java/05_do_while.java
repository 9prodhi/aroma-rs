class Fixture {
    void spin(Lock lock) {
        do {
            lock.tryAcquire();
        } while (!lock.isHeld());
    }
}
