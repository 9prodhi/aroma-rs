class Fixture {
    void copy(Path from, Path to) throws IOException {
        try (InputStream in = Files.newInputStream(from); OutputStream out = Files.newOutputStream(to)) {
            in.transferTo(out);
        }
    }
}
