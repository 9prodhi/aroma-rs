class Fixture {
    void render(Model model, Writer writer) {
        writer.write(model.title(), format(model.body(), 80));
        flush();
    }
}
