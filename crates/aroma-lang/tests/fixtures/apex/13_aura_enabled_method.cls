public with sharing class AccountController {
    @AuraEnabled(cacheable=true)
    public static List<Account> search(String term) {
        String likeTerm = '%' + term + '%';
        return [SELECT Id, Name FROM Account WHERE Name LIKE :likeTerm WITH USER_MODE LIMIT 10];
    }
}
