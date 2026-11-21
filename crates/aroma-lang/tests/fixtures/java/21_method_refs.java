class Fixture {
    void refs(List<String> items, Set<String> seen) {
        items.forEach(System.out::println);
        items.removeIf(seen::contains);
        items.stream().map(String::valueOf).collect(Collectors.toCollection(ArrayList::new));
    }
}
