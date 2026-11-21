public with sharing class SyncJob {
    public void sync(List<Account> incoming, List<Account> stale) {
        upsert incoming Account.External_Id__c;
        delete stale;
        undelete stale;
        insert as user incoming;
    }
}
