class ArrayMinMax {
    int range(int[] values) {
        if (values.length == 0) {
            return 0;
        }
        int min = values[0];
        int max = values[0];
        for (int i = 1; i < values.length; i++) {
            min = Math.min(min, values[i]);
            max = Math.max(max, values[i]);
        }
        int range = max - min;
        System.out.println(min + " " + max);
        return range;
    }
}
