trigger OpportunityAudit on Opportunity (after update) {
    if (Trigger.isAfter && Trigger.isUpdate) {
        for (Opportunity opp : Trigger.new) {
            Opportunity old = Trigger.oldMap.get(opp.Id);
            if (opp.StageName != old.StageName) {
                AuditLog.record(opp.Id, old.StageName);
            }
        }
    }
}
