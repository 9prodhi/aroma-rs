@IsTest
private class AccountServiceTest {
    @IsTest
    static void createsAccount() {
        Account acc = new Account(Name = 'Acme');
        Test.startTest();
        insert acc;
        Test.stopTest();
        System.assertEquals(1, [SELECT COUNT() FROM Account]);
    }
}
