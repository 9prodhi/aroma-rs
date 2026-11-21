public with sharing class Closer {
    public void closeAll(Id ownerId) {
        List<Task> done = new List<Task>();
        for (Task t : [SELECT Id, Status FROM Task WHERE OwnerId = :ownerId]) {
            t.Status = 'Completed';
            done.add(t);
        }
        update done;
    }
}
