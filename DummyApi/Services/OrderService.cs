using DummyApi.Models;

namespace DummyApi.Services;

public interface IOrderService
{
    IEnumerable<Order> GetAll(string? status = null);
    Order? GetById(int id);
    Order Create(CreateOrderRequest request);
    Order? UpdateStatus(int id, string status);
    bool Delete(int id);
}

public class OrderService : IOrderService
{
    private readonly List<Order> _orders = new()
    {
        new Order
        {
            Id = 1,
            UserId = 1,
            Items = new() { new OrderItem { ProductId = 1, Quantity = 1, UnitPrice = 999.99m } },
            Total = 999.99m,
            Status = "Shipped",
            CreatedAt = DateTime.UtcNow.AddDays(-3),
        },
        new Order
        {
            Id = 2,
            UserId = 2,
            Items = new() { new OrderItem { ProductId = 3, Quantity = 5, UnitPrice = 4.99m } },
            Total = 24.95m,
            Status = "Pending",
            CreatedAt = DateTime.UtcNow.AddHours(-6),
        },
    };

    private int _nextId = 3;

    public IEnumerable<Order> GetAll(string? status = null) =>
        status is null ? _orders : _orders.Where(o => o.Status == status);

    public Order? GetById(int id) => _orders.FirstOrDefault(o => o.Id == id);

    public Order Create(CreateOrderRequest request)
    {
        var order = new Order
        {
            Id = _nextId++,
            UserId = request.UserId,
            Items = request.Items,
            Total = request.Items.Sum(i => i.Quantity * i.UnitPrice),
            Status = "Pending",
            CreatedAt = DateTime.UtcNow,
        };
        _orders.Add(order);
        return order;
    }

    public Order? UpdateStatus(int id, string status)
    {
        var order = GetById(id);
        if (order is null) return null;
        order.Status = status;
        return order;
    }

    public bool Delete(int id)
    {
        var order = GetById(id);
        if (order is null) return false;
        _orders.Remove(order);
        return true;
    }
}
