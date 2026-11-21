public with sharing class InvoiceFactory {
    public Invoice__c build(Account acc) {
        Invoice__c inv = new Invoice__c(Name = 'INV-1', Amount__c = 10);
        inv.Account__c = acc.Id;
        String region = inv.Account__r.Region__c;
        return inv;
    }
}
