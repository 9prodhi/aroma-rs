class ParseLongTimeout {
    long parseTimeout(String text, long fallback) {
        if (text == null) {
            return fallback;
        }
        String trimmed = text.trim().toLowerCase();
        long millis = fallback;
        try {
            millis = Long.parseLong(trimmed.replace("ms", ""));
        } catch (NumberFormatException e) {
            System.err.println("bad timeout: " + trimmed);
        }
        System.out.println(millis);
        return millis;
    }
}
