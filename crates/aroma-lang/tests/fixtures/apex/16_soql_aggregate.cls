public with sharing class StageStats {
    public Map<String, Integer> counts() {
        Map<String, Integer> out = new Map<String, Integer>();
        for (AggregateResult ar : [SELECT StageName, COUNT(Id) total FROM Opportunity GROUP BY StageName]) {
            out.put((String) ar.get('StageName'), (Integer) ar.get('total'));
        }
        return out;
    }
}
