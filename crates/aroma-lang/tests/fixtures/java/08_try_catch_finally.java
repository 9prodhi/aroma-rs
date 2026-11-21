class Fixture {
    void load(Store store) {
        try {
            store.open();
        } catch (IOException | IllegalStateException e) {
            log(e);
        } finally {
            store.close();
        }
    }
}
