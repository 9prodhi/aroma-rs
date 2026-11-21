class RepeatString {
    String repeat(String unit, int times, String separator) {
        StringBuilder sb = new StringBuilder();
        int index = 0;
        while (index < times) {
            if (index > 0) {
                sb.append(separator);
            }
            sb.append(unit);
            index++;
        }
        sb.append("!");
        String result = sb.toString();
        return result;
    }
}
