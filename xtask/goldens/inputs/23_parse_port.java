class ParsePort {
    int parsePort(String text) {
        if (text == null) {
            return 80;
        }
        String trimmed = text.trim();
        int port = 80;
        try {
            port = Integer.parseInt(trimmed);
        } catch (NumberFormatException e) {
            System.err.println("bad port: " + trimmed);
        }
        if (port < 1 || port > 65535) {
            port = 80;
        }
        return port;
    }
}
