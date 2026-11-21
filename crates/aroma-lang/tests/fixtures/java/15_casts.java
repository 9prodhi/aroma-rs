class Fixture {
    long widen(Object value, double d) {
        String s = (String) value;
        int i = (int) d;
        return (long) i + s.length();
    }
}
