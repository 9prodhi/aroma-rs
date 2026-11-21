class Fixture {
    String grade(int score) {
        if (score > 90) {
            return "A";
        } else if (score > 75) {
            return "B";
        } else {
            return "C";
        }
    }
}
