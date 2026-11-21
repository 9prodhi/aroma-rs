public with sharing class LeadWriter {
    public void save(List<Lead> fresh, Lead existing) {
        insert fresh;
        existing.Status = 'Working';
        update existing;
    }
}
