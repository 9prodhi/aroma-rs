public with sharing class OrderService {
    public static void submit(List<Order> orders) {
        OrderService.validate(orders);
        PaymentGateway.charge(orders);
    }
    public static void validate(List<Order> orders) {
        System.assert(!orders.isEmpty());
    }
}
