class ParseIntDefault {
    int parseOrDefault(String text, int fallback) {
        if (text == null) {
            return fallback;
        }
        String trimmed = text.trim();
        int value = fallback;
        try {
            value = Integer.parseInt(trimmed);
        } catch (NumberFormatException e) {
            System.err.println("bad number: " + trimmed);
        }
        System.out.println(value);
        return value;
    }
}
