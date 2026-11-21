public with sharing class Priorities {
    public Integer rank(String priority) {
        switch on priority {
            when 'High', 'Urgent' {
                return 1;
            }
            when 'Low' {
                return 3;
            }
            when else {
                return 2;
            }
        }
    }
}
