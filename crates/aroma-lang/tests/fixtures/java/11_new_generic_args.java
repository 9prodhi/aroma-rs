class Fixture {
    Map<String, List<Integer>> index(int size) {
        Map<String, List<Integer>> map = new HashMap<String, List<Integer>>(size);
        List<Integer> empty = new ArrayList<>();
        map.put("", empty);
        return map;
    }
}
