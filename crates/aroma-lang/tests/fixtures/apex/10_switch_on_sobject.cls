public with sharing class Describer {
    public String describe(SObject record) {
        switch on record {
            when Account a {
                return a.Name;
            }
            when Contact c {
                return c.LastName;
            }
            when else {
                return null;
            }
        }
    }
}
