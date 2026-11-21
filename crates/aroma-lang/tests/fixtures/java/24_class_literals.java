class Fixture {
    void types(Registry registry) {
        registry.bind(String.class, int[].class);
        Class<?> c = Fixture.class;
    }
}
