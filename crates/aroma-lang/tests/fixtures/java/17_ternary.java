class Fixture {
    int clamp(int x, int max) {
        return x > max ? max : x < 0 ? 0 : x;
    }
}
