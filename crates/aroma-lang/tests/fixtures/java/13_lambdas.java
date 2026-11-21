class Fixture {
    void wire(Button button, Calculator calc) {
        button.onClick(event -> handle(event));
        calc.register((a, b) -> a + b);
        calc.check((int v) -> {
            return v > 0;
        });
    }
}
