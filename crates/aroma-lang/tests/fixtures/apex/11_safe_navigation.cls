public with sharing class OwnerLookup {
    public String ownerName(Account acc) {
        String name = acc?.Owner?.Name;
        return name == null ? 'unknown' : name;
    }
}
