public with sharing class Hierarchy {
    public List<Account> withContacts() {
        return [SELECT Name, Id, (SELECT LastName, Id FROM Contacts) FROM Account];
    }
}
