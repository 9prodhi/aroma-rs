public with sharing class ContactQueries {
    public List<Contact> byAccount(Id accountId, Set<String> titles) {
        return [SELECT Id, LastName FROM Contact WHERE AccountId = :accountId AND Title IN :titles];
    }
}
