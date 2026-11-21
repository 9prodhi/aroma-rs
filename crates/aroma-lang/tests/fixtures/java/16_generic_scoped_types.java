class Fixture {
    void entries(java.util.Map<String, Integer> counts) {
        for (Map.Entry<String, Integer> entry : counts.entrySet()) {
            java.util.List<String> keys = new java.util.ArrayList<>();
            keys.add(entry.getKey());
        }
    }
}
