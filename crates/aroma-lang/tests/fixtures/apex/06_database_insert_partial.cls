public with sharing class BulkInserter {
    public Integer insertAll(List<Contact> rows) {
        Integer failures = 0;
        Database.SaveResult[] results = Database.insert(rows, false);
        for (Database.SaveResult result : results) {
            if (!result.isSuccess()) {
                failures++;
            }
        }
        return failures;
    }
}
