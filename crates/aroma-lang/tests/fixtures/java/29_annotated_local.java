class Fixture {
    void raw(List list) {
        @SuppressWarnings(value = "unchecked") final List<String> typed = list;
        typed.clear();
    }
}
