class ArraySumAverage {
    double average(int[] values) {
        if (values.length == 0) {
            return 0.0;
        }
        long sum = 0;
        for (int i = 0; i < values.length; i++) {
            sum += values[i];
        }
        double avg = (double) sum / values.length;
        System.out.println(sum);
        return avg;
    }
}
