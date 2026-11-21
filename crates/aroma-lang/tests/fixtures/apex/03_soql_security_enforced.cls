public with sharing class CaseQueries {
    public Integer countOpen() {
        List<Case> cases = [SELECT Id, Status FROM Case WHERE IsClosed = false WITH SECURITY_ENFORCED];
        return cases.size();
    }
}
