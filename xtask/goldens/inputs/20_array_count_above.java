class ArrayCountAbove {
    int countAbove(int[] values, int threshold) {
        if (values.length == 0) {
            return 0;
        }
        int count = 0;
        for (int i = 0; i < values.length; i++) {
            if (values[i] > threshold) {
                count++;
            }
        }
        double ratio = (double) count / values.length;
        System.out.println(ratio);
        return count;
    }
}
