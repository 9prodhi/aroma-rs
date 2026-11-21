public with sharing class Collections2 {
    public Map<Id, List<Contact>> group(List<Contact> contacts) {
        Map<Id, List<Contact>> byAccount = new Map<Id, List<Contact>>();
        Set<Id> seen = new Set<Id>();
        List<String> tags = new List<String>{ 'a', 'b' };
        for (Contact c : contacts) {
            if (!byAccount.containsKey(c.AccountId)) {
                byAccount.put(c.AccountId, new List<Contact>());
            }
            byAccount.get(c.AccountId).add(c);
            seen.add(c.Id);
        }
        return byAccount;
    }
}
