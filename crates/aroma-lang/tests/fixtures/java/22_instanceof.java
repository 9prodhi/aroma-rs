class Fixture {
    int size(Object shape) {
        if (shape instanceof Square) {
            return 4;
        }
        if (shape instanceof Polygon p) {
            return p.sides();
        }
        return 0;
    }
}
