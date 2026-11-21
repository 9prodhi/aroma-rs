public with sharing class SafeSaver {
    public Boolean trySave(Account acc) {
        try {
            update acc;
            return true;
        } catch (DmlException e) {
            System.debug(LoggingLevel.ERROR, e.getMessage());
            return false;
        } finally {
            Logger.flush();
        }
    }
}
