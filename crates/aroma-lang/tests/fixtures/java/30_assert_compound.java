class Fixture {
    int scale(int x, int factor) {
        assert factor != 0 : "factor";
        x *= factor;
        x -= 1;
        return x % 7;
    }
}
