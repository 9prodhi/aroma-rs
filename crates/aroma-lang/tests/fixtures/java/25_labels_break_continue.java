class Fixture {
    void scan(int[][] rows) {
        outer:
        for (int[] row : rows) {
            for (int cell : row) {
                if (cell < 0) continue outer;
                if (cell == 0) break outer;
            }
        }
    }
}
