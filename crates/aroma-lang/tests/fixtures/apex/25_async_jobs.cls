public with sharing class Dispatcher {
    @future(callout=true)
    public static void notify(Set<Id> ids) {
        HttpRequest req = new HttpRequest();
        req.setMethod('POST');
        new Http().send(req);
        System.enqueueJob(new RetryJob(ids));
    }
}
