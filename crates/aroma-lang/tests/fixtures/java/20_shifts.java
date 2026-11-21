class Fixture {
    int bits(int x, int n) {
        int a = x >> n;
        int b = x >>> 2;
        int c = 1 << n;
        x >>= 1;
        return a | b | c;
    }
}
