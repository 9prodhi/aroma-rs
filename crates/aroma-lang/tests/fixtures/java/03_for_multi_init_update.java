class Fixture {
    void walk(int n) {
        for (int lo = 0, hi = n; lo < hi; lo++, hi--) {
            swap(lo, hi);
        }
        for (;;) {
            break;
        }
    }
}
