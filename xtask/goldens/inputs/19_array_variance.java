class ArrayVariance {
    double variance(double[] values) {
        if (values.length == 0) {
            return 0.0;
        }
        double sum = 0;
        for (int i = 0; i < values.length; i++) {
            sum += values[i];
        }
        double mean = sum / values.length;
        double squares = 0;
        for (int i = 0; i < values.length; i++) {
            squares += (values[i] - mean) * (values[i] - mean);
        }
        double result = squares / values.length;
        return result;
    }
}
