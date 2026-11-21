class Fixture {
    void counters(int i, int j, boolean b) {
        i++;
        --j;
        int k = -i + ~j;
        b = !b;
    }
}
