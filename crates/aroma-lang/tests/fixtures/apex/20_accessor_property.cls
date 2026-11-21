public with sharing class Thermostat {
    private Decimal celsius;
    public Decimal fahrenheit {
        get { return celsius * 9 / 5 + 32; }
        set { celsius = (value - 32) * 5 / 9; }
    }
}
