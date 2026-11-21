public with sharing class OpportunityQueries {
    public List<Opportunity> open(Decimal minAmount) {
        List<Opportunity> opps = [SELECT Name, Amount FROM Opportunity WHERE Amount > :minAmount WITH USER_MODE ORDER BY Amount DESC LIMIT 50];
        return opps;
    }
}
