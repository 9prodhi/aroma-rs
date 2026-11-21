public with sharing class Invoice {
    private Decimal total;
    public Invoice(Decimal amount) {
        this.total = amount;
        Ledger.register(this);
    }
}
