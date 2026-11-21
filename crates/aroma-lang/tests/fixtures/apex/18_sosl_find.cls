public with sharing class Finder {
    public List<List<SObject>> find(String term) {
        return [FIND :term IN ALL FIELDS RETURNING Account(Name), Contact(LastName)];
    }
}
